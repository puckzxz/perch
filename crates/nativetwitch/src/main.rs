//! A Twitch stream and its chat in one native window.
//!
//!     cargo run -p nativetwitch -- <channel> [--volume 0-100]
//!
//! The app runs streamlink itself, so nothing needs a terminal alongside it.

mod chat;
mod video;

use std::path::PathBuf;
use std::sync::Arc;

use chat::ChatView;
use emotes::ImageCache;
use gpui::{
    div, img, prelude::*, px, rgb, size, App, Application, Bounds, Context, Entity, RenderImage,
    SharedString, Task, Window, WindowBounds, WindowOptions,
};
use streamlink::{StreamEvent, StreamSupervisor};
use video::VideoStream;

/// Frames are rendered at this size and scaled to fit by the element.
///
/// This is also what quality selection targets: measurements showed the ratio
/// between source and pane matters more than the pixel count, so the stream is
/// chosen to land on a clean ratio rather than simply "best".
const RENDER_WIDTH: u32 = 1280;
const RENDER_HEIGHT: u32 = 720;

/// mpv opens at 100% otherwise, which is jarring for a window that starts
/// playing the moment it appears.
const DEFAULT_VOLUME: u8 = 10;

/// Emotes are immutable and endlessly repeated, so the cache is worth keeping
/// between runs rather than in a temp dir.
fn image_cache_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("nativetwitch")
        .join("images")
}

// ── Video ────────────────────────────────────────────────────────────

struct VideoView {
    stream: VideoStream,
    /// The frame currently painted, and the one before it. GPUI uploads every
    /// distinct `RenderImage` into its sprite atlas and `RenderImage::new` mints
    /// a fresh id per frame, so without evicting the frame before last the atlas
    /// grows by one frame every frame until VRAM runs out.
    current: Option<Arc<RenderImage>>,
    previous: Option<Arc<RenderImage>>,
    _pump: Task<()>,
}

impl VideoView {
    fn new(
        url: String,
        volume: u8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<Self> {
        let (stream, mut frames) = VideoStream::start(url, RENDER_WIDTH, RENDER_HEIGHT, volume)?;

        // The render thread wakes us; we only ever ask GPUI to repaint.
        let pump = cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;
            while frames.next().await.is_some() {
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });

        Ok(Self {
            stream,
            current: None,
            previous: None,
            _pump: pump,
        })
    }
}

impl Render for VideoView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let Some(frame) = self.stream.latest_frame() else {
            return placeholder("buffering…").into_any_element();
        };

        // Retire the frame before last: it is no longer on screen, so its atlas
        // entry can go. Never drop `current` itself - that one is still painted.
        if let Some(current) = self.current.take() {
            if let Some(previous) = self.previous.take() {
                if previous.id != current.id {
                    let _ = window.drop_image(previous);
                }
            }
            self.previous = Some(current);
        }
        self.current = Some(frame.clone());

        img(frame).size_full().into_any_element()
    }
}

fn placeholder(message: impl Into<SharedString>) -> impl IntoElement {
    div()
        .size_full()
        .bg(rgb(0x0e0e12))
        .flex()
        .items_center()
        .justify_center()
        .text_color(rgb(0x6b6478))
        .child(message.into())
}

// ── Root ─────────────────────────────────────────────────────────────

/// What the video pane should show while there is no stream yet.
enum StreamState {
    Starting,
    Playing(Entity<VideoView>),
    Offline,
    Failed(SharedString),
}

struct RootView {
    channel: String,
    state: StreamState,
    quality: Option<SharedString>,
    chat: Entity<ChatView>,
    _supervisor: StreamSupervisor,
    _pump: Task<()>,
}

impl RootView {
    fn new(channel: String, volume: u8, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (cache, ready) = ImageCache::new(image_cache_dir()).expect("failed to open image cache");
        let cache = Arc::new(cache);
        let chat = cx.new(|cx| ChatView::new(channel.clone(), cache, ready, window, cx));

        // Quality is chosen for the pane we actually render at, not "best".
        let (supervisor, mut events) = StreamSupervisor::start(channel.clone(), RENDER_HEIGHT);

        let pump = cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;
            while let Some(event) = events.next().await {
                let updated = this.update_in(cx, |this: &mut RootView, window, cx| {
                    match event {
                        StreamEvent::Resolving => this.state = StreamState::Starting,
                        StreamEvent::Ready { url, quality } => {
                            this.quality = Some(quality.into());
                            let view = cx.new(|cx| {
                                VideoView::new(url, volume, window, cx)
                                    .expect("failed to start video")
                            });
                            this.state = StreamState::Playing(view);
                        }
                        StreamEvent::Offline => this.state = StreamState::Offline,
                        StreamEvent::Failed { reason } => {
                            this.state = StreamState::Failed(reason.into())
                        }
                    }
                    cx.notify();
                });
                if updated.is_err() {
                    break;
                }
            }
        });

        Self {
            channel,
            state: StreamState::Starting,
            quality: None,
            chat,
            _supervisor: supervisor,
            _pump: pump,
        }
    }

    fn title_bar(&self) -> impl IntoElement {
        let subtitle: SharedString = match (&self.state, &self.quality) {
            (StreamState::Playing(_), Some(quality)) => quality.clone(),
            (StreamState::Playing(_), None) => "live".into(),
            (StreamState::Starting, _) => "starting…".into(),
            (StreamState::Offline, _) => "offline".into(),
            (StreamState::Failed(_), _) => "error".into(),
        };

        div()
            .w_full()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .bg(rgb(0x1b1822))
            .border_b_1()
            .border_color(rgb(0x2e2939))
            .child(
                div()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0xf2eff7))
                    .child(SharedString::from(self.channel.clone())),
            )
            .child(div().text_xs().text_color(rgb(0x948ca5)).child(subtitle))
    }
}

impl Render for RootView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let video = match &self.state {
            StreamState::Playing(view) => view.clone().into_any_element(),
            StreamState::Starting => placeholder("starting stream…").into_any_element(),
            StreamState::Offline => {
                placeholder(format!("{} is offline", self.channel)).into_any_element()
            }
            StreamState::Failed(reason) => placeholder(reason.clone()).into_any_element(),
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x131118))
            .child(self.title_bar())
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    // min_w_0 and overflow_hidden are load-bearing: without them
                    // the flex item will not shrink below the video's intrinsic
                    // width and the chat pane is pushed off-screen.
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .overflow_hidden()
                            .child(video),
                    )
                    .child(
                        div()
                            .w(px(340.))
                            .h_full()
                            .flex_none()
                            .bg(rgb(0x1b1822))
                            .border_l_1()
                            .border_color(rgb(0x2e2939))
                            .py_2()
                            .child(self.chat.clone()),
                    ),
            )
    }
}

// ── Entry point ──────────────────────────────────────────────────────

fn main() {
    let mut args = std::env::args().skip(1);
    let mut channel = None;
    let mut volume = DEFAULT_VOLUME;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--volume" => {
                volume = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(DEFAULT_VOLUME)
                    .min(100);
            }
            other => channel = Some(other.trim_start_matches('#').to_string()),
        }
    }

    let Some(channel) = channel else {
        eprintln!("usage: nativetwitch <channel> [--volume 0-100]");
        eprintln!();
        eprintln!("Runs streamlink itself; no terminal needed alongside.");
        eprintln!("Set TWITCH_AUTH_TOKEN to unlock subscriber-only qualities and suppress ads.");
        std::process::exit(2);
    };

    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1600.), px(900.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };

        cx.open_window(options, |window, cx| {
            cx.new(|cx| RootView::new(channel.clone(), volume, window, cx))
        })
        .expect("failed to open window");

        cx.activate(true);
    });
}
