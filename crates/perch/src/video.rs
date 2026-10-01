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
use std::time::{Duration, Instant};

use futures::channel::mpsc;
use gpui::RenderImage;
use image::{Frame, RgbaImage};
use mpv_frames::{Config, EndReason, Event, Player};
use smallvec::smallvec;
use streamlink::Playlist;

use crate::seek_bar::Timeline;
use crate::vod::{self, Extent, Recording};

/// Upper bound on render size. 1440p is the highest Twitch tier, so anything
/// beyond this is scaling up, which measured as the single most expensive thing
/// this pipeline can do.
pub const MAX_RENDER_WIDTH: u32 = 2560;
pub const MAX_RENDER_HEIGHT: u32 = 1440;

/// Whether to ask mpv to decode on the GPU. On, now that it has been measured.
///
/// The standing argument against it was that the software render path needs
/// frames in system memory, so hardware decoding has to copy them back, and the
/// copy was assumed to cost more than it saved. It does not. Measured on one
/// 936p60 stream, two 3.5-minute runs of the same channel in the same window,
/// steady state only, from `cpu_log`:
///
///     hwdec=no             146.6% of one core
///     hwdec=d3d11va-copy    97.1%              -34%
///
///     worker (mpv decode)   54.9 -> 38.1
///     (unnamed) (driver)    34.7 -> 11.2
///     main (UI thread)      45.2 -> 35.5
///     renders/s            119.9 -> 120.0      unchanged
///
/// The readback is real and shows up in `mpv-render`; it is simply much smaller
/// than decoding the frame on the CPU. Note the driver threads fell furthest,
/// which the readback argument did not predict at all.
///
/// It is quality-neutral, which is why it is worth taking: the same bitstream
/// through a fixed-function decoder yields the same frames. `auto-copy` also
/// falls back to software on its own when a codec or driver cannot do it, so
/// the worst case is what this used to do unconditionally - and `video.rs` logs
/// which one actually engaged, because asking is not getting.
///
/// `PERCH_HWDEC=0` turns it off, for a machine where the GPU decoder misbehaves.
fn hwdec_requested() -> bool {
    !matches!(
        std::env::var("PERCH_HWDEC").as_deref(),
        Ok("0") | Ok("off") | Ok("no")
    )
}

fn to_millis(secs: f64) -> u64 {
    (secs.max(0.0) * 1000.0).round() as u64
}

fn from_millis(millis: u64) -> f64 {
    millis as f64 / 1000.0
}

fn pack_size(width: u32, height: u32) -> u64 {
    ((width as u64) << 32) | height as u64
}

fn unpack_size(packed: u64) -> (u32, u32) {
    ((packed >> 32) as u32, packed as u32)
}

/// What a stream plays, which decides what a pause means and whether there
/// is a position to report.
#[derive(Debug, Clone, PartialEq)]
pub enum Playback {
    /// A broadcast as it happens, served on loopback by streamlink. Resuming
    /// from a pause jumps back to the live edge, and there is no position
    /// worth reporting.
    Live { url: String },
    /// A recording: its playlist, already read, and how far in to open it.
    /// Position and length are reported, seeks are taken, and a pause is only
    /// a pause.
    ///
    /// The player is never handed the playlist's URL. Its demuxer cannot seek
    /// the fragmented-MP4 playlists Twitch keeps for most large channels, so
    /// the app positions the recording itself, by rewriting the playlist and
    /// reopening — see `vod`, and `streamlink::playlist` for the full story.
    /// `label` names the files that takes.
    Vod {
        playlist: Playlist,
        start_at: f64,
        label: String,
        /// Where the recording is, published for whoever follows it. The
        /// pane's, not the stream's: see [`PositionHandle`].
        position: PositionHandle,
    },
}

/// Why a stream stopped producing frames.
///
/// Live video has no natural end, so either of these means the pane is now
/// showing a still: whatever is on screen is the last frame that arrived, and
/// nothing will replace it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stopped {
    /// The source ran out, which for a broadcast means it is over.
    Ended,
    /// Playback failed, in mpv's own words.
    Failed(String),
}

impl Stopped {
    /// What one of mpv's end-of-file reasons means for a live stream, or
    /// `None` for the ones that are not the stream stopping.
    ///
    /// A redirect is a playlist being expanded and playback continues through
    /// it. `Stopped` is what this player being torn down looks like from
    /// inside its own render loop, and a pane that is closing does not need
    /// telling. An unknown reason is left alone on client.h's own advice.
    ///
    /// For a recording, `Ended` is not news of a broadcast finishing but of
    /// the recording reaching its end — the same event, read by the pane
    /// according to what it was playing.
    fn from_end(reason: EndReason, error: Option<String>) -> Option<Self> {
        match reason {
            EndReason::Eof => Some(Self::Ended),
            // mpv only fills in `error` for this reason, and not always.
            EndReason::Error => Some(Self::Failed(
                error.unwrap_or_else(|| "playback stopped".to_string()),
            )),
            EndReason::Stopped | EndReason::Redirect | EndReason::Unknown(_) => None,
        }
    }
}

/// A cloneable way to ask a running stream to render at a different size.
///
/// Handed to the UI so a layout pass can report the pane's real size without
/// holding a borrow on the stream itself.
///
/// The stream keeps the handle it was started with (`StartOptions::size`)
/// as its own target, not a copy of the size in it. So two streams given
/// clones of one handle follow one probe: a second player of the same pane,
/// started with the first one's handle, renders at the pane's size from its
/// first frame rather than at whatever size it was started at.
#[derive(Clone)]
pub struct SizeHandle(Arc<AtomicU64>);

impl SizeHandle {
    /// A handle asking for `width` x `height` physical pixels, capped as
    /// [`request`](Self::request) caps it: the size a stream renders at until
    /// its pane is first measured.
    pub fn new(width: u32, height: u32) -> Self {
        let handle = Self(Arc::new(AtomicU64::new(0)));
        handle.request(width, height);
        handle
    }

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

    /// The size asked for last, as `(width, height)`: what the render thread
    /// renders at, between frames.
    fn get(&self) -> (u32, u32) {
        unpack_size(self.0.load(Ordering::Relaxed))
    }
}

/// Where a recording is, shared between the render thread that learns it
/// and whatever follows the pane: the chat replay, which reads it a few
/// times a second to know what to say next, the history, and a link to the
/// moment. The seek bar asks the player on screen, which is the one
/// publishing here.
///
/// Made by whoever opens the pane rather than by the stream, and handed to
/// each stream the pane starts, so a quality change — which is a new player
/// — picks up the position the old one reached rather than starting the bar
/// at zero, and the replay following it sees playback carry on rather than
/// a jump. A stream writes it only while it publishes its position (see
/// [`Positions`]), so a second player of the pane, still getting ready
/// beside the one on screen, moves none of them. Whole
/// milliseconds inside: an atomic cannot hold an `f64`, and nothing reads
/// finer.
#[derive(Clone, Default)]
pub struct PositionHandle(Arc<AtomicU64>);

impl PositionHandle {
    pub fn new() -> Self {
        Self::default()
    }

    /// A handle that says `secs` until a player says otherwise: where a pane
    /// opens its recording. The chat replay follows the handle from the
    /// moment the pane opens, seconds before any player exists, so a pane
    /// picking up three hours in would otherwise load the first minute of
    /// chat, and then the right one.
    pub fn starting_at(secs: f64) -> Self {
        let handle = Self::default();
        handle.set(secs);
        handle
    }

    /// Seconds into the recording. Zero until a player has said otherwise.
    pub fn get(&self) -> f64 {
        from_millis(self.0.load(Ordering::Relaxed))
    }

    fn set(&self, secs: f64) {
        self.0.store(to_millis(secs), Ordering::Relaxed);
    }
}

/// The same handle, not the same value: two panes on one recording each
/// have their own.
impl PartialEq for PositionHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for PositionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("PositionHandle").field(&self.get()).finish()
    }
}

/// Where one player is in its recording, and whether its pane hears it.
///
/// Every position the player learns — where it opens, where mpv says it
/// is, where a seek sends it — is [`report`](Self::report)ed here. It always
/// lands in the player's own handle, which is what the player itself reads
/// (to reopen where it is, to tell a stall from a wait) and what
/// `VideoStream::position` answers with, so the seek bar of the player on
/// screen. It reaches the pane's shared handle, which the chat replay, the
/// history and a link to the moment follow, only while the player
/// publishes. A pane's one player publishes from the start. A second player
/// of the same pane, getting ready beside the one on screen at another
/// rendition, does not until it takes over ([`publish`](Self::publish)):
/// before then, its position moving would reset the replay to a moment
/// nobody is watching, and note that moment in the history.
#[derive(Clone)]
struct Positions {
    /// The pane's, shared with whoever follows it; see [`PositionHandle`].
    shared: PositionHandle,
    /// This player's alone.
    own: PositionHandle,
    publishing: Arc<AtomicBool>,
}

impl Positions {
    /// A player's positions, reaching `shared` only once published.
    fn new(shared: PositionHandle) -> Self {
        Self {
            shared,
            own: PositionHandle::new(),
            publishing: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The player is at `secs`: always its own, and the pane's while it
    /// publishes.
    fn report(&self, secs: f64) {
        self.own.set(secs);
        if self.publishing.load(Ordering::SeqCst) {
            self.shared.set(secs);
        }
    }

    /// From now on the pane hears this player, starting with where it is.
    ///
    /// The flag first, then the hand-over: a report racing it on the render
    /// thread either sees the flag and writes the pane itself, or wrote its
    /// own handle before the hand-over read it. At worst the pane is one
    /// report behind for a frame, and the next report puts it right.
    fn publish(&self) {
        self.publishing.store(true, Ordering::SeqCst);
        self.shared.set(self.own.get());
    }

    /// Where this player is, published or not.
    fn get(&self) -> f64 {
        self.own.get()
    }
}

/// How a stream starts: everything about it but what it plays.
pub struct StartOptions {
    /// The render size to follow, shared with whatever measures the pane;
    /// see [`SizeHandle`]. A new pane's is [`SizeHandle::new`]; a second
    /// player of a pane already on screen is handed that pane's.
    pub size: SizeHandle,
    /// The level mpv opens at, 0-100: what the user chose, or nothing for a
    /// pane Mute all is holding.
    pub volume: u8,
    /// Whether it opens paused. Applied on the render thread's first pass,
    /// before it waits for a frame, the way every later pause is.
    pub paused: bool,
    /// Whether its position reaches the pane from the start; see
    /// [`Positions`]. A pane's one player does; a player started beside it
    /// does not until it takes over.
    pub publish: bool,
}

/// A running stream. Dropping this stops the render thread and tears down mpv.
pub struct VideoStream {
    latest: Arc<Mutex<Option<Arc<RenderImage>>>>,
    stop: Arc<AtomicBool>,
    /// Render size in physical pixels: the handle the stream was started
    /// with, kept rather than copied, so whoever else holds it — a second
    /// player of the same pane — follows the same probe. Packed into one
    /// atomic so width and height can never be read from different frames,
    /// which would allocate a buffer that matches neither.
    target: SizeHandle,
    /// Paused state, applied between frames like volume.
    paused: Arc<AtomicBool>,
    /// Written by the UI, read by the render thread between frames.
    ///
    /// An atomic rather than a channel because volume is a *level*, not an
    /// event: if the user drags a slider, only the final value matters and
    /// intermediate ones can be dropped without anyone noticing.
    volume: Arc<AtomicU8>,
    /// Set once, when the stream stops for good; see [`Stopped`].
    ///
    /// A slot rather than a channel of its own, and read on the same wake the
    /// frames use: the UI is already listening there, and one more `try_send`
    /// after the slot is filled means the next wake it receives - whether this
    /// one or the frame already queued ahead of it - finds the answer.
    stopped: Arc<Mutex<Option<Stopped>>>,
    /// The stream's own resolution, packed like `target`, once mpv has decoded
    /// a frame; zero until then. The UI reads its aspect to size a stacked
    /// pane's video box, so a 4:3 or a vertical stream gets a box its shape
    /// rather than a 16:9 one with bars inside it.
    source: Arc<AtomicU64>,
    /// Seconds into a recording, as mpv last reported them — counted from the
    /// start of the recording, not of the file the player happens to be
    /// reading. Only ever written for [`Playback::Vod`]; a live stream has no
    /// position anybody wants. The pane hears it only while this player
    /// publishes; see [`Positions`].
    positions: Positions,
    /// A seek the UI has asked for and the render thread has not yet applied.
    ///
    /// A slot rather than a queue, like volume: only the last target matters,
    /// and a scrub that lands twice in one frame should not seek twice.
    seek: Arc<Mutex<Option<f64>>>,
    /// Whether this is a broadcast as it happens.
    live: bool,
    /// A recording's length, for the seek bar; `None` on a live stream.
    extent: Option<Extent>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl VideoStream {
    /// Start playing `playback`, as `options` say: at the size its handle
    /// asks for, at its volume, paused or not, and with its position reaching
    /// the pane or kept to itself.
    ///
    /// Returns the stream plus a channel that fires once per new frame. The
    /// channel carries no data - the frame itself lives in the slot, so a
    /// missed notification just means the UI coalesces two frames into one.
    /// The same channel carries the news that the stream has stopped, or
    /// never started: a player mpv could not open is reported as
    /// [`Stopped::Failed`], so its pane offers to try again rather than
    /// saying it is starting for good.
    pub fn start(
        options: StartOptions,
        playback: Playback,
    ) -> anyhow::Result<(Self, mpsc::Receiver<()>)> {
        let StartOptions {
            size: target,
            volume,
            paused: start_paused,
            publish,
        } = options;
        let latest: Arc<Mutex<Option<Arc<RenderImage>>>> = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let volume_level = Arc::new(AtomicU8::new(volume));
        let paused = Arc::new(AtomicBool::new(start_paused));
        let source_size = Arc::new(AtomicU64::new(0));
        let stopped: Arc<Mutex<Option<Stopped>>> = Arc::new(Mutex::new(None));
        let positions = match &playback {
            Playback::Live { .. } => Positions::new(PositionHandle::new()),
            // Said now rather than when the player reports: for the seconds
            // it takes to open, the bar and the replay would otherwise sit on
            // wherever the last player left the handle.
            Playback::Vod {
                position, start_at, ..
            } => {
                let positions = Positions::new(position.clone());
                positions.report(*start_at);
                positions
            }
        };
        if publish {
            positions.publish();
        }
        let seek: Arc<Mutex<Option<f64>>> = Arc::new(Mutex::new(None));
        let live = matches!(playback, Playback::Live { .. });
        let extent = match &playback {
            Playback::Live { .. } => None,
            Playback::Vod { playlist, .. } => Some(Extent::new(playlist)),
        };
        let (mut tx, rx) = mpsc::channel::<()>(1);

        // One `ImageId` for the whole stream, minted here rather than per frame.
        //
        // GPUI keys its sprite atlas on the id `RenderImage::new` takes from a
        // global counter, and a key it has not seen is a new tile: on Windows a
        // CreateTexture2D plus a shader resource view, and a Release of the pair
        // the frame before. That is 14.7 MB built and thrown away sixty times a
        // second for a maximised 1440p pane. Holding the id lets the UI
        // overwrite the tile in place instead — see `Window::update_image`.
        //
        // The id has to come *from* GPUI's counter rather than be invented.
        // `ImageId` is a bare `usize`, so a number we picked could collide with
        // a real image — an emote, an avatar, a thumbnail — and the two would
        // share one atlas tile. A 1x1 throwaway is the cheapest way to draw one
        // legitimately.
        let image_id = RenderImage::new(smallvec![Frame::new(RgbaImage::new(1, 1))]).id;

        let thread = std::thread::Builder::new()
            .name("mpv-render".into())
            .spawn({
                let latest = latest.clone();
                let stop = stop.clone();
                let volume_level = volume_level.clone();
                let target = target.clone();
                let paused = paused.clone();
                let source_size = source_size.clone();
                let stopped = stopped.clone();
                let positions = positions.clone();
                let seek = seek.clone();
                let extent = extent.clone();
                move || {
                    // What to open, and how. A recording is opened on a
                    // playlist of the app's own making; see `vod`.
                    let (url, extra, mut recording) = match playback {
                        Playback::Live { url } => (url, Vec::new(), None),
                        Playback::Vod {
                            playlist,
                            start_at,
                            label,
                            ..
                        } => {
                            let extent = extent.expect("a recording has an extent");
                            match Recording::open(playlist, &label, start_at, extent) {
                                Ok((recording, path, within)) => {
                                    let mut extra = Recording::mpv_options();
                                    extra.push(("start".to_string(), format!("{within:.3}")));
                                    (path.to_string_lossy().into_owned(), extra, Some(recording))
                                }
                                Err(e) => {
                                    eprintln!("video: could not write the playlist: {e}");
                                    *stopped.lock().unwrap() = Some(Stopped::Failed(format!(
                                        "could not write the playlist: {e}"
                                    )));
                                    let _ = tx.try_send(());
                                    return;
                                }
                            }
                        }
                    };
                    let config = Config {
                        audio: true,
                        hwdec: hwdec_requested(),
                        volume,
                        extra,
                    };
                    let player = match Player::open_with(&url, config) {
                        Ok(p) => p,
                        Err(e) => {
                            eprintln!("video: could not open {url}: {e}");
                            if let Some(recording) = recording {
                                recording.finish();
                            }
                            // Said, as a stop is: the pane is waiting on this
                            // channel for its first frame, and a thread that
                            // just ended left it saying "Starting…" for good.
                            *stopped.lock().unwrap() = Some(Stopped::Failed(format!(
                                "could not open the player: {e}"
                            )));
                            let _ = tx.try_send(());
                            return;
                        }
                    };
                    if let Some(recording) = &recording {
                        recording.keep_growing(stop.clone());
                    }

                    // Everything this loop wants to know about the stream
                    // arrives as an event rather than being asked for. This is
                    // the thread that calls `mpv_render_context_render`, and
                    // libmpv's render.h forbids it any synchronous client call:
                    // the core can be waiting on a render while the render
                    // thread waits on the core, which mpv resolves with a
                    // timeout and a dropped frame. Observing is the shape the
                    // header declares safe, and it is cheaper besides - the
                    // drop counter used to be polled once a second whether or
                    // not it had moved.
                    for name in [
                        "width",
                        "height",
                        "hwdec-current",
                        "decoder-frame-drop-count",
                    ] {
                        if let Err(e) = player.observe_property(name) {
                            eprintln!("video: could not observe {name}: {e}");
                        }
                    }
                    // Only a recording has a position worth the traffic:
                    // `time-pos` changes on every frame, and a live pane
                    // would pay for sixty strings a second to learn nothing.
                    // mpv's `duration` is deliberately not read: it is the
                    // length of the file the player is reading, which starts
                    // wherever the last reposition put it, so the seek bar
                    // takes its length from the playlist — see `vod::Extent`.
                    if recording.is_some() {
                        // `path` is how a reposition learns its file has
                        // taken over — see `Recording::now_playing`.
                        for name in ["time-pos", "path"] {
                            if let Err(e) = player.observe_property(name) {
                                eprintln!("video: could not observe {name}: {e}");
                            }
                        }
                    }

                    // Source resolution, learned from mpv once the first frame
                    // decodes. Rendering above it means mpv upscales on the CPU,
                    // which measured at 117-196% of a core - far and away the
                    // most expensive thing this pipeline can do. Staying at or
                    // below source and letting the GPU stretch the last bit is
                    // effectively free.
                    let mut source: Option<(u32, u32)> = None;
                    let (mut source_w, mut source_h) = (None, None);
                    let (mut current_w, mut current_h) = target.get();
                    let mut applied_volume = volume;
                    // mpv opens playing, so a stream started paused differs
                    // from this on the first pass, which pauses it before
                    // it waits for a frame.
                    let mut applied_pause = false;
                    let mut last_drops = 0u64;
                    // What getting a recording going again needs to know:
                    // when it was paused, when it last moved, and whether it
                    // has ever moved — a player still opening has not
                    // stalled. See `vod::reopen_on_resume` and `vod::stalled`.
                    let mut paused_since: Option<Instant> = None;
                    let mut moved_at = Instant::now();
                    let mut has_moved = false;

                    'frames: while !stop.load(Ordering::Relaxed) {
                        for event in player.poll_events() {
                            match event {
                                Event::PropertyChange { name, value } => match name.as_str() {
                                    "width" => source_w = value.and_then(|v| v.parse().ok()),
                                    "height" => source_h = value.and_then(|v| v.parse().ok()),
                                    // Where a recording is: the player's
                                    // position in its file, plus where in the
                                    // recording that file starts. mpv says so
                                    // only when it changes, so each of these
                                    // is also the picture moving.
                                    "time-pos" => {
                                        let secs = value.and_then(|v| v.parse::<f64>().ok());
                                        if let (Some(secs), Some(recording)) = (secs, &recording) {
                                            if let Some(at) = recording.position(secs) {
                                                positions.report(at);
                                                moved_at = Instant::now();
                                                has_moved = true;
                                            }
                                        }
                                    }
                                    "path" => {
                                        if let (Some(path), Some(recording)) =
                                            (value, &mut recording)
                                        {
                                            recording.now_playing(&path);
                                        }
                                    }
                                    // Asking for hardware decoding is not the
                                    // same as getting it: `auto-copy` falls
                                    // back to software whenever the codec, the
                                    // driver or the build cannot do it, and it
                                    // does so silently. Without this line a
                                    // measurement of "hwdec on" could be a
                                    // measurement of nothing having changed.
                                    // mpv reports the property's current
                                    // value on observing it, which before the
                                    // first decode is no value at all; the
                                    // real answer follows once a decoder is
                                    // chosen, so an absent one is not news.
                                    "hwdec-current" => {
                                        if let Some(actual) = value {
                                            eprintln!(
                                                "video: hwdec requested={}, active={actual}",
                                                hwdec_requested()
                                            );
                                            crate::cpu_log::note_hwdec(&actual);
                                        }
                                    }
                                    // `decoder-frame-drop-count` is the one
                                    // that means the machine could not keep
                                    // up, which is the difference between a
                                    // busy CPU and a picture that suffered.
                                    // Published as this player's own delta,
                                    // not its total: every pane has its own
                                    // mpv counting from zero, so totals into
                                    // one shared slot would subtract one
                                    // stream's count from another's.
                                    // `saturating_sub` because mpv resets the
                                    // counter on a restart, and `seek_to_live`
                                    // on unpause is a restart.
                                    "decoder-frame-drop-count" => {
                                        if let Some(total) =
                                            value.and_then(|v| v.parse::<u64>().ok())
                                        {
                                            crate::cpu_log::note_dropped(
                                                total.saturating_sub(last_drops),
                                            );
                                            last_drops = total;
                                        }
                                    }
                                    _ => {}
                                },
                                // The one place the end of a broadcast is
                                // visible. streamlink's external HTTP server
                                // runs in its continuous mode, so it stays up
                                // waiting for the next request rather than
                                // exiting when the stream ends - which means
                                // its supervisor reports nothing, and without
                                // this the last frame simply stays on screen
                                // and reads as a pause.
                                Event::EndFile { reason, error } => {
                                    if let Some(reason) = Stopped::from_end(reason, error) {
                                        eprintln!("video: stream stopped: {reason:?}");
                                        *stopped.lock().unwrap() = Some(reason);
                                        let _ = tx.try_send(());
                                        // Nothing more will decode. Leaving
                                        // tears mpv down here rather than
                                        // waiting on frames that cannot
                                        // arrive.
                                        break 'frames;
                                    }
                                }
                                Event::Shutdown => {
                                    eprintln!("video: mpv shut down");
                                    break 'frames;
                                }
                                _ => {}
                            }
                        }
                        if source.is_none() {
                            if let (Some(w), Some(h)) = (source_w, source_h) {
                                if w > 0 && h > 0 {
                                    source = Some((w, h));
                                    source_size.store(pack_size(w, h), Ordering::Relaxed);
                                    eprintln!("video: source is {w}x{h}");
                                }
                            }
                        }

                        // Resize between frames. mpv scales to whatever size
                        // it is asked for, so following the pane means never
                        // paying to render pixels that get thrown away - and
                        // never capping a 1440p stream at 720p either.
                        let (mut want_w, mut want_h) = target.get();
                        if let Some((source_w, source_h)) = source {
                            want_w = want_w.min(source_w);
                            want_h = want_h.min(source_h);
                        }
                        if (want_w, want_h) != (current_w, current_h) && want_w > 0 && want_h > 0 {
                            current_w = want_w;
                            current_h = want_h;
                        }
                        // What the CPU log needs to make two sessions
                        // comparable: mpv scales to whatever the pane asks for,
                        // so this is the largest single thing that moves the
                        // cost between one run and the next.
                        crate::cpu_log::note_render_size(current_w, current_h);

                        // Apply between frames rather than mid-render, and only
                        // when it actually changed. Both setters queue the
                        // change for mpv's own thread; see `Player::set_paused`.
                        let want_pause = paused.load(Ordering::Relaxed);
                        if want_pause != applied_pause {
                            if !want_pause && live {
                                // Resuming from a pause on a live stream would
                                // otherwise continue from where it stopped,
                                // leaving the viewer permanently behind. On a
                                // recording, where it stopped is the point.
                                let _ = player.seek_to_live();
                            }
                            if let Some(recording) = &mut recording {
                                if want_pause {
                                    paused_since = Some(Instant::now());
                                } else if let Some(since) = paused_since.take() {
                                    // Its connection may have died while it
                                    // waited, and a dead one can hang the
                                    // demuxer for good; a reopen costs a
                                    // second. Queued ahead of the unpause, so
                                    // mpv takes the new file first.
                                    if vod::reopen_on_resume(since.elapsed()) {
                                        let at = positions.get();
                                        eprintln!(
                                            "video: resuming after {}s paused by reopening at {at:.1}",
                                            since.elapsed().as_secs()
                                        );
                                        if let Err(e) = recording.reposition(&player, at) {
                                            eprintln!("video: could not reposition: {e}");
                                        }
                                    }
                                    moved_at = Instant::now();
                                }
                            }
                            if let Err(e) = player.set_paused(want_pause) {
                                eprintln!("video: could not pause: {e}");
                            }
                            applied_pause = want_pause;
                        }

                        // A seek, taken between frames like everything else.
                        // Taken out from under the lock before the work: the
                        // UI writes the slot from a click handler, and a
                        // reposition writes a file.
                        let wanted_seek = seek.lock().unwrap().take();
                        if let (Some(secs), Some(recording)) = (wanted_seek, &mut recording) {
                            // Said now rather than when the new file reports
                            // its first position, so the bar does not sit on
                            // the old one while the player reopens.
                            positions.report(secs);
                            if let Err(e) = recording.reposition(&player, secs) {
                                eprintln!("video: could not reposition: {e}");
                            }
                            moved_at = Instant::now();
                            // A seek while paused opened fresh connections,
                            // so the pause that counts starts again here.
                            if paused_since.is_some() {
                                paused_since = Some(Instant::now());
                            }
                        }

                        // A recording that should be playing and has sat
                        // still for a while is got going again the same way,
                        // whatever stopped it — a connection that died
                        // mid-segment is the one seen, but the remedy does
                        // not depend on the cause.
                        if let Some(recording) = &mut recording {
                            let at = positions.get();
                            if !applied_pause
                                && has_moved
                                && vod::stalled(moved_at.elapsed(), at, &recording.timeline())
                            {
                                eprintln!(
                                    "video: the recording has not moved for {}s; reopening at {at:.1}",
                                    moved_at.elapsed().as_secs()
                                );
                                if let Err(e) = recording.reposition(&player, at) {
                                    eprintln!("video: could not reposition: {e}");
                                }
                                moved_at = Instant::now();
                            }
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
                        // A buffer per frame, given away rather than copied out
                        // of. Reusing one meant `RgbaImage` had to take a clone,
                        // because the next render would overwrite whatever the
                        // UI was still holding - a 7.9 MB alloc *and* a 7.9 MB
                        // memcpy every frame at 1080p. Handing the buffer over
                        // pays only the alloc, and the allocator hands back the
                        // block the last frame just released, so the pages stay
                        // warm. Measured against real renders: 2.8-3.9 ms per
                        // frame down to 2.3-2.4, with the frame-to-frame spread
                        // much tighter, which is what the slot cares about.
                        //
                        // Sized here rather than on resize, so the size in hand
                        // is always the one just rendered at.
                        let mut buf = vec![0u8; current_w as usize * current_h as usize * 4];
                        if let Err(e) = player.render_bgra(current_w, current_h, &mut buf) {
                            eprintln!("video: render failed: {e}");
                            break;
                        }

                        // GPUI reads RenderImage as BGRA even though the buffer
                        // type is named Rgba, so the bytes go in unswapped. This
                        // looks like a bug and is not one.
                        let Some(image) = RgbaImage::from_raw(current_w, current_h, buf) else {
                            eprintln!("video: buffer did not match {current_w}x{current_h}");
                            break;
                        };
                        let mut frame = RenderImage::new(smallvec![Frame::new(image)]);
                        // Overwrite the id `new` just took from the global
                        // counter with the stream's own; see `image_id`.
                        frame.id = image_id;

                        *latest.lock().unwrap() = Some(Arc::new(frame));

                        // Full channel means the UI has not consumed the last
                        // wake yet; it will see this frame when it gets there.
                        let _ = tx.try_send(());
                    }

                    // The player first: its demuxer holds the playlist it is
                    // reading, and the file cannot go until it lets go.
                    drop(player);
                    if let Some(recording) = recording {
                        recording.finish();
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
                stopped,
                source: source_size,
                positions,
                seek,
                live,
                extent,
                thread: Some(thread),
            },
            rx,
        ))
    }

    /// How long the recording is, for the seek bar. `None` on a live stream,
    /// which is also what hides the bar.
    pub fn timeline(&self) -> Option<Timeline> {
        self.extent.as_ref().map(Extent::timeline)
    }

    /// Seconds into a recording, as mpv last reported. Zero on a live stream,
    /// which reports none. This player's own, whether or not its pane hears
    /// it yet; see [`Positions`].
    pub fn position(&self) -> f64 {
        self.positions.get()
    }

    /// Ask a recording to jump to `secs`. Applied between frames, like volume;
    /// a second request before the first is applied replaces it. Ignored on a
    /// live stream, which has nowhere to go.
    pub fn seek_to(&self, secs: f64) {
        if self.live {
            return;
        }
        *self.seek.lock().unwrap() = Some(secs.max(0.0));
    }

    /// The stream's resolution, once known. Width over height is the shape a
    /// pane should give its video box.
    pub fn source_size(&self) -> Option<(u32, u32)> {
        match unpack_size(self.source.load(Ordering::Relaxed)) {
            (0, _) | (_, 0) => None,
            size => Some(size),
        }
    }

    /// A handle the UI can use to report the pane size each layout pass: the
    /// one the stream was started with.
    pub fn size_handle(&self) -> SizeHandle {
        self.target.clone()
    }

    /// Pause or resume. Resuming jumps back to the live edge.
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// Change playback volume (0-100). Takes effect on the next frame.
    ///
    /// Write-only: what mpv hears is not what the user chose while Mute all
    /// holds a pane silent, so the level worth reading lives in the view's
    /// `Loudness`, not here.
    pub fn set_volume(&self, percent: u8) {
        self.volume.store(percent.min(100), Ordering::Relaxed);
    }

    /// Why the stream stopped, if it has, taken so it is reported once.
    pub fn take_stopped(&self) -> Option<Stopped> {
        self.stopped.lock().unwrap().take()
    }

    /// The newest frame, if one has arrived.
    pub fn latest_frame(&self) -> Option<Arc<RenderImage>> {
        self.latest.lock().unwrap().clone()
    }
}

impl Drop for VideoStream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Deliberately not joined. Every drop site is on the UI thread - a pane
        // closing, a quality switch, leaving the watch page with the miniplayer
        // off - and the worker is inside `wait_for_frame` for up to its timeout
        // and then inside `mpv_terminate_destroy`, which waits for mpv's own
        // threads to wind down: the demuxer's network read, the audio output
        // closing its device. A join here put all of that on the window's
        // frame loop. The thread sees `stop` on its next pass, drops the
        // player itself, and retires; nothing it holds is needed in order.
        drop(self.thread.take());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reason is the whole point of reading mpv's end-of-file event: a
    /// pane closing produces one too, and reporting that as the broadcast
    /// ending would put "ended the stream" on a pane the user just closed.
    #[test]
    fn only_a_real_stop_ends_the_stream() {
        assert_eq!(
            Stopped::from_end(EndReason::Eof, None),
            Some(Stopped::Ended)
        );
        assert_eq!(
            Stopped::from_end(EndReason::Error, Some("loading failed".into())),
            Some(Stopped::Failed("loading failed".into()))
        );
        // mpv promises the text only when it has one.
        assert!(matches!(
            Stopped::from_end(EndReason::Error, None),
            Some(Stopped::Failed(_))
        ));
        for quiet in [
            EndReason::Stopped,
            EndReason::Redirect,
            EndReason::Unknown(9),
        ] {
            assert_eq!(Stopped::from_end(quiet, None), None, "{quiet:?}");
        }
    }

    /// A pane at 100 s, and a player of it that has not been published: one
    /// getting ready beside the player on screen.
    fn unpublished() -> (PositionHandle, Positions) {
        let pane = PositionHandle::starting_at(100.0);
        let player = Positions::new(pane.clone());
        (pane, player)
    }

    /// Where an unpublished player opens, where mpv says it is and where a
    /// seek sends it all stay its own: the chat replay and the history go on
    /// following the player on screen.
    #[test]
    fn a_private_position_reaches_nobody_until_published() {
        let (pane, player) = unpublished();
        for secs in [104.0, 250.5, 0.0] {
            player.report(secs);
            assert_eq!(pane.get(), 100.0, "the pane heard {secs}");
        }
    }

    /// Taking over hands the pane where the player is at that moment, not
    /// where it next reports, so the replay does not sit on the old player's
    /// place until mpv next speaks.
    #[test]
    fn publishing_hands_over_where_the_player_is() {
        let (pane, player) = unpublished();
        player.report(104.25);
        player.publish();
        assert_eq!(pane.get(), 104.25);
    }

    /// Once published, every report reaches the pane, a step back included.
    #[test]
    fn a_published_position_follows_every_report() {
        let (pane, player) = unpublished();
        player.publish();
        for secs in [101.0, 102.5, 60.0] {
            player.report(secs);
            assert_eq!(pane.get(), secs);
        }
    }

    /// The player reads its own position, published or not: reopening where
    /// it is after a long pause, and telling a stall from a wait, are about
    /// this player's place in its file, not the pane's.
    #[test]
    fn the_player_reads_its_own_position_even_unpublished() {
        let (pane, player) = unpublished();
        player.report(250.0);
        assert_eq!(player.get(), 250.0);
        assert_eq!(pane.get(), 100.0);
        player.publish();
        player.report(251.0);
        assert_eq!(player.get(), 251.0);
    }

    /// A stream keeps the handle it is given, so streams given clones of one
    /// handle follow one probe: the pane's measure reaches both, and a second
    /// player started with the first one's handle starts at the size the
    /// pane already asked for rather than at the size a new pane starts at.
    #[test]
    fn streams_given_one_size_handle_follow_one_target() {
        let pane = SizeHandle::new(1280, 720);
        let first = pane.clone();
        pane.request(1600, 900);
        let second = first.clone();
        assert_eq!(
            second.get(),
            (1600, 900),
            "the second starts at the pane's size"
        );
        pane.request(960, 540);
        assert_eq!(first.get(), (960, 540));
        assert_eq!(second.get(), (960, 540));
        // And a new one is capped as every request is.
        assert_eq!(
            SizeHandle::new(7680, 4320).get(),
            (MAX_RENDER_WIDTH, MAX_RENDER_HEIGHT)
        );
    }
}
