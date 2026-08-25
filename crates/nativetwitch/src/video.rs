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

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::channel::mpsc;
use gpui::RenderImage;
use image::{Frame, RgbaImage};
use mpv_frames::{Config, Player};
use smallvec::smallvec;

/// A running stream. Dropping this stops the render thread and tears down mpv.
pub struct VideoStream {
    latest: Arc<Mutex<Option<Arc<RenderImage>>>>,
    stop: Arc<AtomicBool>,
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
        let (mut tx, rx) = mpsc::channel::<()>(1);

        let thread = std::thread::Builder::new()
            .name("mpv-render".into())
            .spawn({
                let latest = latest.clone();
                let stop = stop.clone();
                let volume_level = volume_level.clone();
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

                    let mut buf = vec![0u8; width as usize * height as usize * 4];
                    let mut applied_volume = volume;

                    while !stop.load(Ordering::Relaxed) {
                        // Apply between frames rather than mid-render, and only
                        // when it actually changed.
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
                        if let Err(e) = player.render_bgra(width, height, &mut buf) {
                            eprintln!("video: render failed: {e}");
                            break;
                        }

                        // GPUI reads RenderImage as BGRA even though the buffer
                        // type is named Rgba, so the bytes go in unswapped. This
                        // looks like a bug and is not one.
                        let Some(image) = RgbaImage::from_raw(width, height, buf.clone()) else {
                            eprintln!("video: frame buffer did not match {width}x{height}");
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
                volume: volume_level,
                thread: Some(thread),
            },
            rx,
        ))
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
