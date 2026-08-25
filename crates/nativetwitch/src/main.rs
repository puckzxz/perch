//! A Twitch stream and its chat in one native window.
//!
//!     cargo run -p nativetwitch -- [channel] [--volume 0-100]
//!
//! With no channel, the last one is reopened. The app runs streamlink itself,
//! so nothing needs a terminal alongside it.

mod chat;
mod follows;
mod video;

use std::path::PathBuf;
use std::sync::Arc;

use chat::ChatView;
use emotes::ImageCache;
use follows::{FollowsEvent, FollowsService};
use gpui::{
    div, img, prelude::*, px, rgb, size, App, Application, Bounds, Context, Entity, RenderImage,
    SharedString, Task, Window, WindowBounds, WindowOptions,
};
use settings::{QualityPreference, Settings};
use streamlink::{StreamEvent, StreamOptions, StreamSupervisor};
use twitch_api::LiveStream;
use video::VideoStream;

/// Frames are rendered at this size and scaled to fit by the element.
///
/// This is also what quality selection targets: measurements showed the ratio
/// between source and pane matters more than the pixel count, so the stream is
/// chosen to land on a clean ratio rather than simply "best".
const RENDER_WIDTH: u32 = 1280;
const RENDER_HEIGHT: u32 = 720;

const SIDEBAR_WIDTH: f32 = 260.0;
const THUMBNAIL_WIDTH: u32 = 320;
const THUMBNAIL_HEIGHT: u32 = 180;

/// Emotes and thumbnails are reproducible, so they live in local app data
/// rather than roaming with settings.
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
    /// Takes an already-started stream so a failure to open can be shown in the
    /// window rather than panicking inside entity construction.
    fn from_stream(
        stream: VideoStream,
        mut frames: futures::channel::mpsc::Receiver<()>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let pump = cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;
            while frames.next().await.is_some() {
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });

        Self {
            stream,
            current: None,
            previous: None,
            _pump: pump,
        }
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

enum StreamState {
    Idle,
    Starting,
    Playing(Entity<VideoView>),
    Offline,
    Failed(SharedString),
}

/// Where sign-in has got to. Only ever advances forward.
enum SignIn {
    NeedsClientId,
    Connecting,
    AwaitingCode {
        user_code: SharedString,
        verification_uri: SharedString,
    },
    SignedIn(SharedString),
    Error(SharedString),
}

struct RootView {
    settings: Settings,
    settings_path: PathBuf,
    cache: Arc<ImageCache>,

    channel: Option<String>,
    state: StreamState,
    quality: Option<SharedString>,
    chat: Option<Entity<ChatView>>,
    supervisor: Option<StreamSupervisor>,
    _stream_pump: Option<Task<()>>,

    follows: Vec<LiveStream>,
    sign_in: SignIn,
    _follows: FollowsService,
    _follows_pump: Task<()>,
    _cache_pump: Task<()>,
}

impl RootView {
    fn new(
        channel: Option<String>,
        volume_override: Option<u8>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings_path = settings::default_path();
        let mut settings = Settings::load(&settings_path).unwrap_or_else(|e| {
            eprintln!("settings: {e}; using defaults");
            Settings::default()
        });
        if let Some(volume) = volume_override {
            settings.volume = volume;
        }

        let (cache, mut cache_ready) =
            ImageCache::new(image_cache_dir()).expect("failed to open image cache");
        let cache = Arc::new(cache);

        // One pump for every image consumer. Repainting the root repaints its
        // children, so the sidebar and chat both pick up new images.
        let cache_pump = cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;
            while cache_ready.next().await.is_some() {
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });

        let (service, mut events) = FollowsService::start(settings_path.clone());
        let follows_pump = cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;
            while let Some(event) = events.next().await {
                let ok = this.update(cx, |this: &mut RootView, cx| {
                    match event {
                        FollowsEvent::NeedsClientId => this.sign_in = SignIn::NeedsClientId,
                        FollowsEvent::AwaitingCode {
                            user_code,
                            verification_uri,
                        } => {
                            this.sign_in = SignIn::AwaitingCode {
                                user_code: user_code.into(),
                                verification_uri: verification_uri.into(),
                            }
                        }
                        FollowsEvent::SignedIn { login } => {
                            this.sign_in = SignIn::SignedIn(login.into())
                        }
                        FollowsEvent::Streams(streams) => this.follows = streams,
                        FollowsEvent::Error(reason) => this.sign_in = SignIn::Error(reason.into()),
                    }
                    cx.notify();
                });
                if ok.is_err() {
                    break;
                }
            }
        });

        let mut view = Self {
            settings,
            settings_path,
            cache,
            channel: None,
            state: StreamState::Idle,
            quality: None,
            chat: None,
            supervisor: None,
            _stream_pump: None,
            follows: Vec::new(),
            sign_in: SignIn::Connecting,
            _follows: service,
            _follows_pump: follows_pump,
            _cache_pump: cache_pump,
        };

        if let Some(channel) = channel.or_else(|| view.settings.last_channel.clone()) {
            view.open_channel(channel, window, cx);
        }
        view
    }

    /// Switch to `channel`, tearing down whatever was playing.
    ///
    /// Dropping the old supervisor is what stops the previous streamlink, so
    /// switching channels never leaves a process behind.
    fn open_channel(&mut self, channel: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.channel.as_deref() == Some(channel.as_str()) {
            return;
        }

        self.supervisor = None;
        self._stream_pump = None;
        self.state = StreamState::Starting;
        self.quality = None;
        self.channel = Some(channel.clone());

        self.chat = Some(cx.new(|cx| ChatView::new(channel.clone(), self.cache.clone(), window, cx)));

        let quality = match &self.settings.quality {
            QualityPreference::Auto => None,
            QualityPreference::Fixed(name) => Some(name.clone()),
        };
        let options = StreamOptions {
            quality,
            auth_token: self.settings.credentials.auth_token.clone(),
        };

        let (supervisor, mut events) =
            StreamSupervisor::start(channel.clone(), RENDER_HEIGHT, options);
        let volume = self.settings.volume;

        self._stream_pump = Some(cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;
            while let Some(event) = events.next().await {
                let ok = this.update_in(cx, |this: &mut RootView, window, cx| {
                    match event {
                        StreamEvent::Resolving => this.state = StreamState::Starting,
                        StreamEvent::Ready { url, quality } => {
                            this.quality = Some(quality.into());
                            match VideoStream::start(url, RENDER_WIDTH, RENDER_HEIGHT, volume) {
                                Ok((stream, frames)) => {
                                    let view = cx.new(|cx| {
                                        VideoView::from_stream(stream, frames, window, cx)
                                    });
                                    this.state = StreamState::Playing(view);
                                }
                                Err(e) => {
                                    this.state = StreamState::Failed(e.to_string().into())
                                }
                            }
                        }
                        StreamEvent::Offline => this.state = StreamState::Offline,
                        StreamEvent::Failed { reason } => {
                            this.state = StreamState::Failed(reason.into())
                        }
                    }
                    cx.notify();
                });
                if ok.is_err() {
                    break;
                }
            }
        }));
        self.supervisor = Some(supervisor);

        // Remember for next launch. A failure here is not worth interrupting
        // playback over.
        self.settings.last_channel = Some(channel);
        if let Err(e) = self.settings.save(&self.settings_path) {
            eprintln!("settings: could not save: {e}");
        }
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let header = div()
            .px_3()
            .py_2()
            .text_xs()
            .text_color(rgb(0x6b6478))
            .child(match &self.sign_in {
                SignIn::SignedIn(login) => {
                    SharedString::from(format!("following · {login}"))
                }
                SignIn::Connecting => "connecting…".into(),
                SignIn::NeedsClientId => "not signed in".into(),
                SignIn::AwaitingCode { .. } => "waiting for you…".into(),
                SignIn::Error(_) => "sign-in problem".into(),
            });

        let mut list = div()
            .id("follows")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col();

        match &self.sign_in {
            SignIn::AwaitingCode {
                user_code,
                verification_uri,
            } => {
                list = list.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .p_3()
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(0x948ca5))
                                .child("Open this page and enter the code:"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(0xb392ff))
                                .child(verification_uri.clone()),
                        )
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(rgb(0xf2eff7))
                                .child(user_code.clone()),
                        ),
                );
            }
            SignIn::NeedsClientId => {
                list = list.child(
                    div().p_3().text_xs().text_color(rgb(0x948ca5)).child(
                        "To see who you follow, create an application at dev.twitch.tv and put its Client ID in settings.json.",
                    ),
                );
            }
            SignIn::Error(reason) => {
                list = list.child(
                    div()
                        .p_3()
                        .text_xs()
                        .text_color(rgb(0xf98b7f))
                        .child(reason.clone()),
                );
            }
            _ if self.follows.is_empty() => {
                list = list.child(
                    div()
                        .p_3()
                        .text_xs()
                        .text_color(rgb(0x6b6478))
                        .child("nobody you follow is live"),
                );
            }
            _ => {}
        }

        for (index, stream) in self.follows.iter().enumerate() {
            let login = stream.user_login.clone();
            let selected = self.channel.as_deref() == Some(login.as_str());
            let thumb = self
                .cache
                .get_or_request(&twitch_api::thumbnail(
                    &stream.thumbnail_url,
                    THUMBNAIL_WIDTH,
                    THUMBNAIL_HEIGHT,
                ));

            let mut entry = div()
                .id(("stream", index))
                .flex()
                .flex_col()
                .gap_1()
                .px_2()
                .py_2()
                .cursor_pointer()
                .hover(|style| style.bg(rgb(0x252031)))
                .on_click(cx.listener(move |this, _event, window, cx| {
                    this.open_channel(login.clone(), window, cx);
                }));

            if selected {
                entry = entry.bg(rgb(0x2a2140));
            }
            if let Some(path) = thumb {
                entry = entry.child(img(path).w_full().rounded_sm());
            }

            entry = entry
                .child(
                    div()
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_xs()
                        .text_color(rgb(0xf2eff7))
                        .child(SharedString::from(stream.display_name.clone())),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(0x948ca5))
                        .child(SharedString::from(format!(
                            "{} · {}",
                            stream.game_name,
                            format_viewers(stream.viewer_count)
                        ))),
                );

            list = list.child(entry);
        }

        div()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(rgb(0x17141e))
            .border_r_1()
            .border_color(rgb(0x2e2939))
            .child(header)
            .child(list)
    }

    fn title_bar(&self) -> impl IntoElement {
        let (name, subtitle): (SharedString, SharedString) = match (&self.channel, &self.state) {
            (Some(channel), StreamState::Playing(_)) => (
                channel.clone().into(),
                self.quality.clone().unwrap_or_else(|| "live".into()),
            ),
            (Some(channel), StreamState::Starting) => (channel.clone().into(), "starting…".into()),
            (Some(channel), StreamState::Offline) => (channel.clone().into(), "offline".into()),
            (Some(channel), StreamState::Failed(_)) => (channel.clone().into(), "error".into()),
            _ => ("nativetwitch".into(), "pick a channel".into()),
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
                    .child(name),
            )
            .child(div().text_xs().text_color(rgb(0x948ca5)).child(subtitle))
    }
}

fn format_viewers(count: u64) -> String {
    if count >= 1000 {
        format!("{:.1}k viewers", count as f64 / 1000.0)
    } else {
        format!("{count} viewers")
    }
}

impl Render for RootView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let video = match &self.state {
            StreamState::Playing(view) => view.clone().into_any_element(),
            StreamState::Starting => placeholder("starting stream…").into_any_element(),
            StreamState::Idle => placeholder("pick a channel").into_any_element(),
            StreamState::Offline => placeholder(format!(
                "{} is offline",
                self.channel.as_deref().unwrap_or("channel")
            ))
            .into_any_element(),
            StreamState::Failed(reason) => placeholder(reason.clone()).into_any_element(),
        };

        let chat = match &self.chat {
            Some(chat) => chat.clone().into_any_element(),
            None => div().into_any_element(),
        };

        div()
            .flex()
            .flex_row()
            .size_full()
            .bg(rgb(0x131118))
            .child(self.sidebar(cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(self.title_bar())
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .flex_row()
                            // min_w_0 and overflow_hidden are load-bearing:
                            // without them the flex item will not shrink below
                            // the video's intrinsic width and chat is pushed
                            // off-screen.
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
                                    .w(px(self.settings.chat_width))
                                    .h_full()
                                    .flex_none()
                                    .bg(rgb(0x1b1822))
                                    .border_l_1()
                                    .border_color(rgb(0x2e2939))
                                    .py_2()
                                    .child(chat),
                            ),
                    ),
            )
    }
}

// ── Entry point ──────────────────────────────────────────────────────

fn main() {
    let mut args = std::env::args().skip(1);
    let mut channel = None;
    let mut volume = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--volume" => volume = args.next().and_then(|v| v.parse::<u8>().ok()).map(|v| v.min(100)),
            "--help" | "-h" => {
                eprintln!("usage: nativetwitch [channel] [--volume 0-100]");
                eprintln!();
                eprintln!("With no channel, the last one is reopened.");
                eprintln!("Settings live at {}", settings::default_path().display());
                std::process::exit(0);
            }
            other => channel = Some(other.trim_start_matches('#').to_string()),
        }
    }

    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1700.), px(900.)), cx);
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
