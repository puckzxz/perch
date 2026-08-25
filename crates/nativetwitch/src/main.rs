//! A Twitch stream and its chat in one native window.
//!
//! This binary is currently the video spike: it proves libmpv frames can be
//! painted as a first-class GPUI element, composited with normal UI, in one
//! window. Auth, the follow list and real chat come next.
//!
//!     cargo run -p nativetwitch -- <url-or-path>

mod chat;
mod video;

use std::sync::Arc;

use gpui::{
    div, img, prelude::*, px, rgb, size, App, Application, Bounds, Context, RenderImage, Task,
    Window, WindowBounds, WindowOptions,
};

use chat::ChatView;
use video::VideoStream;

/// Frames are rendered at this size regardless of window size, then scaled to
/// fit by the element. Matching this to the source resolution matters a lot:
/// measurements showed arbitrary scale ratios cost more CPU than native 1:1,
/// and upscaling costs several times more again.
const RENDER_WIDTH: u32 = 1280;
const RENDER_HEIGHT: u32 = 720;

/// mpv opens at 100% otherwise, which is jarring for a window that starts
/// playing the moment it appears.
const DEFAULT_VOLUME: u8 = 10;

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
        let (stream, mut frames) =
            VideoStream::start(url, RENDER_WIDTH, RENDER_HEIGHT, volume)?;

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
            return div()
                .size_full()
                .bg(rgb(0x0e0e12))
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(0x6b6478))
                .child("connecting…")
                .into_any_element();
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

struct RootView {
    video: gpui::Entity<VideoView>,
    chat: Option<gpui::Entity<ChatView>>,
}

impl Render for RootView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_row()
            .size_full()
            .bg(rgb(0x131118))
            // Video and chat are siblings in one window - the whole point of the
            // project. Nothing here is a separate OS window.
            // min_w_0 + overflow_hidden are load-bearing: without them the flex
            // item refuses to shrink below the video's intrinsic width, and the
            // chat pane gets pushed off-screen at anything under fullscreen.
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_hidden()
                    .child(self.video.clone()),
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
                    .map(|pane| match self.chat.clone() {
                        Some(chat) => pane.child(chat),
                        None => pane.px_3().text_color(rgb(0x6b6478)).child(
                            "no channel given - pass one as the second argument for chat",
                        ),
                    }),
            )
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(url) = args.next() else {
        eprintln!("usage: nativetwitch <url-or-path> [twitch-channel] [--volume 0-100]");
        eprintln!();
        eprintln!("For a Twitch channel, run streamlink as a byte source first:");
        eprintln!("  streamlink --player-external-http --player-external-http-port 18080 \\");
        eprintln!("      --player-external-http-interface 127.0.0.1 twitch.tv/<channel> best");
        eprintln!("then pass http://127.0.0.1:18080/ <channel>");
        std::process::exit(2);
    };

    // Positional channel, then an optional --volume N.
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
            other => channel = Some(other.to_string()),
        }
    }

    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1600.), px(900.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };

        cx.open_window(options, |window, cx| {
            let video = cx.new(|cx| {
                VideoView::new(url.clone(), volume, window, cx).expect("failed to start video")
            });
            let chat = channel
                .clone()
                .map(|name| cx.new(|cx| ChatView::new(name, window, cx)));
            cx.new(|_| RootView { video, chat })
        })
        .expect("failed to open window");

        cx.activate(true);
    });
}
