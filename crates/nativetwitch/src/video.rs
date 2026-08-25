//! Keeps the newest decoded frame available to the UI thread.
//!
//! Rendering happens on a thread of its own for a specific reason:
//! `mpv_render_context_render` blocks until the frame's display time, for up to
//! `video-timing-offset` (50 ms by default). That wait is what keeps video timed
//! to audio, so we want it - but running it on the UI thread would stall GPUI's
//! entire frame loop. So mpv paces itself over here, and the UI thread only ever
//! picks up whatever finished frame is currently sitting in the slot.
//!
//! The slot holds exactly one frame. If the UI falls behind, older frames are
//! dropped rather than queued: for live video the newest frame is the only one
//! worth showing, and an unbounded queue of 3.5 MB frames is a memory leak with
//! extra steps.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::channel::mpsc;
use gpui::RenderImage;
use image::{Frame, RgbaImage};
use mpv_frames::{Config, Player};
use smallvec::smallvec;

/// Upper bound on render size. 1440p is the highest Twitch tier, so anything
/// beyond this is scaling up, which measured as the single most expensive thing
/// this pipeline can do.
pub const MAX_RENDER_WIDTH: u32 = 2560;
pub const MAX_RENDER_HEIGHT: u32 = 1440;

fn pack_size(width: u32, height: u32) -> u64 {
    ((width as u64) << 32) | height as u64
}

fn unpack_size(packed: u64) -> (u32, u32) {
    ((packed >> 32) as u32, packed as u32)
}

/// A cloneable way to ask a running stream to render at a different size.
///
/// Handed to the UI so a layout pass can report the pane's real size without
/// holding a borrow on the stream itself.
#[derive(Clone)]
pub struct SizeHandle(Arc<AtomicU64>);

impl SizeHandle {
    /// Ask for a new render size in physical pixels.
    ///
    /// Capped so that maximising onto a 4K display does not quietly start
    /// pushing 33 MB per frame through the CPU; the element scales the last
    /// stretch, which is far cheaper than rendering it.
    pub fn request(&self, width: u32, height: u32) {
        let width = width.clamp(160, MAX_RENDER_WIDTH);
        let height = height.clamp(90, MAX_RENDER_HEIGHT);
        self.0.store(pack_size(width, height), Ordering::Relaxed);
    }
}

/// A running stream. Dropping this stops the render thread and tears down mpv.
pub struct VideoStream {
    latest: Arc<Mutex<Option<Arc<RenderImage>>>>,
    stop: Arc<AtomicBool>,
    /// Render size in physical pixels, packed as `(width << 32) | height`.
    ///
    /// One atomic rather than two so width and height can never be read from
    /// different frames, which would allocate a buffer that matches neither.
    target: Arc<AtomicU64>,
    /// Paused state, applied between frames like volume.
    paused: Arc<AtomicBool>,
    /// Written by the UI, read by the render thread between frames.
    ///
    /// An atomic rather than a channel because volume is a *level*, not an
    /// event: if the user drags a slider, only the final value matters and
    /// intermediate ones can be dropped without anyone noticing.
    volume: Arc<AtomicU8>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl VideoStream {
    /// Start playing `url`, rendering frames at `width` x `height`.
    ///
    /// Returns the stream plus a channel that fires once per new frame. The
    /// channel carries no data - the frame itself lives in the slot, so a
    /// missed notification just means the UI coalesces two frames into one.
    pub fn start(
        url: String,
        width: u32,
        height: u32,
        volume: u8,
    ) -> anyhow::Result<(Self, mpsc::Receiver<()>)> {
        let latest: Arc<Mutex<Option<Arc<RenderImage>>>> = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let volume_level = Arc::new(AtomicU8::new(volume));
        let target = Arc::new(AtomicU64::new(pack_size(width, height)));
        let paused = Arc::new(AtomicBool::new(false));
        let (mut tx, rx) = mpsc::channel::<()>(1);

        let thread = std::thread::Builder::new()
            .name("mpv-render".into())
            .spawn({
                let latest = latest.clone();
                let stop = stop.clone();
                let volume_level = volume_level.clone();
                let target = target.clone();
                let paused = paused.clone();
                move || {
                    let config = Config {
                        audio: true,
                        volume,
                        ..Config::default()
                    };
                    let player = match Player::open_with(&url, config) {
                        Ok(p) => p,
                        Err(e) => {
                            eprintln!("video: could not open {url}: {e}");
                            return;
                        }
                    };

                    // Source resolution, learned from mpv once the first frame
                    // decodes. Rendering above it means mpv upscales on the CPU,
                    // which measured at 117-196% of a core - far and away the
                    // most expensive thing this pipeline can do. Staying at or
                    // below source and letting the GPU stretch the last bit is
                    // effectively free.
                    let mut source: Option<(u32, u32)> = None;
                    let (mut current_w, mut current_h) = (width, height);
                    let mut buf = vec![0u8; current_w as usize * current_h as usize * 4];
                    let mut applied_volume = volume;
                    let mut applied_pause = false;

                    while !stop.load(Ordering::Relaxed) {
                        // Apply between frames rather than mid-render, and only
                        // when it actually changed.
                        // Resize between frames. mpv scales to whatever size
                        // it is asked for, so following the pane means never
                        // paying to render pixels that get thrown away - and
                        // never capping a 1440p stream at 720p either.
                        if source.is_none() {
                            let dimension = |key| {
                                player.property(key).and_then(|v| v.parse::<u32>().ok())
                            };
                            if let (Some(w), Some(h)) = (dimension("width"), dimension("height")) {
                                if w > 0 && h > 0 {
                                    source = Some((w, h));
                                }
                            }
                        }

                        let (mut want_w, mut want_h) =
                            unpack_size(target.load(Ordering::Relaxed));
                        if let Some((source_w, source_h)) = source {
                            want_w = want_w.min(source_w);
                            want_h = want_h.min(source_h);
                        }
                        if (want_w, want_h) != (current_w, current_h) && want_w > 0 && want_h > 0 {
                            current_w = want_w;
                            current_h = want_h;
                            buf = vec![0u8; current_w as usize * current_h as usize * 4];
                        }

                        let want_pause = paused.load(Ordering::Relaxed);
                        if want_pause != applied_pause {
                            if !want_pause {
                                // Resuming from a pause on a live stream would
                                // otherwise continue from where it stopped,
                                // leaving the viewer permanently behind.
                                let _ = player.seek_to_live();
                            }
                            if let Err(e) = player.set_paused(want_pause) {
                                eprintln!("video: could not pause: {e}");
                            }
                            applied_pause = want_pause;
                        }

                        let wanted = volume_level.load(Ordering::Relaxed);
                        if wanted != applied_volume {
                            if let Err(e) = player.set_volume(wanted) {
                                eprintln!("video: could not set volume: {e}");
                            }
                            applied_volume = wanted;
                        }

                        if !player.wait_for_frame(Duration::from_millis(200)) {
                            continue;
                        }
                        if let Err(e) = player.render_bgra(current_w, current_h, &mut buf) {
                            eprintln!("video: render failed: {e}");
                            break;
                        }

                        // GPUI reads RenderImage as BGRA even though the buffer
                        // type is named Rgba, so the bytes go in unswapped. This
                        // looks like a bug and is not one.
                        let Some(image) = RgbaImage::from_raw(current_w, current_h, buf.clone())
                        else {
                            eprintln!("video: buffer did not match {current_w}x{current_h}");
                            break;
                        };
                        let frame = Arc::new(RenderImage::new(smallvec![Frame::new(image)]));

                        *latest.lock().unwrap() = Some(frame);

                        // Full channel means the UI has not consumed the last
                        // wake yet; it will see this frame when it gets there.
                        let _ = tx.try_send(());
                    }
                }
            })?;

        Ok((
            Self {
                latest,
                stop,
                target,
                paused,
                volume: volume_level,
                thread: Some(thread),
            },
            rx,
        ))
    }

    /// A handle the UI can use to report the pane size each layout pass.
    pub fn size_handle(&self) -> SizeHandle {
        SizeHandle(self.target.clone())
    }

    /// Pause or resume. Resuming jumps back to the live edge.
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// Change playback volume (0-100). Takes effect on the next frame.
    pub fn set_volume(&self, percent: u8) {
        self.volume.store(percent.min(100), Ordering::Relaxed);
    }

    pub fn volume(&self) -> u8 {
        self.volume.load(Ordering::Relaxed)
    }

    /// The newest frame, if one has arrived.
    pub fn latest_frame(&self) -> Option<Arc<RenderImage>> {
        self.latest.lock().unwrap().clone()
    }
}

impl Drop for VideoStream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            // The worker checks `stop` between frames, so this returns within
            // one wait_for_frame timeout.
            let _ = thread.join();
        }
    }
}
