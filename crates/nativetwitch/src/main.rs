//! A Twitch stream and its chat in one native window.
//!
//!     cargo run -p nativetwitch -- [channel] [--volume 0-100]
//!
//! With no channel, the last one is reopened. The app runs streamlink itself,
//! so nothing needs a terminal alongside it.

mod chat;
mod follows;
mod settings_view;
mod video;
mod video_view;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chat::ChatView;
use emotes::ImageCache;
use follows::{FollowsEvent, FollowsService};
use gpui::{
    div, img, prelude::*, px, rgb, rgba, size, AnyView, App, Application, Bounds, Context, Entity, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, SharedString, Task, Window,
    WindowBackgroundAppearance, WindowBounds, WindowOptions,
};
use settings::{QualityPreference, Settings};
use settings_view::{SettingsEvent, SettingsPanel};
use streamlink::{StreamEvent, StreamOptions, StreamSupervisor};
use twitch_api::LiveStream;
use video::VideoStream;
use video_view::{VideoEvent, VideoView};

/// Starting render size. The video pane measures itself on the first layout
/// pass and the render thread follows it from then on, so this only decides
/// what the first frame or two look like.
const RENDER_WIDTH: u32 = 1280;
const RENDER_HEIGHT: u32 = 720;

/// Chrome above the video pane, subtracted when estimating its height for
/// quality selection. Approximate on purpose: quality buckets by resolution, so
/// being a few pixels out never changes the answer.
const CHROME_HEIGHT: f32 = 44.0;

const SIDEBAR_WIDTH: f32 = 260.0;
/// Keeps the chat pane usable at both extremes: narrower than this and emotes
/// wrap every other word, wider and the video gets squeezed for no gain.
const CHAT_WIDTH_RANGE: std::ops::RangeInclusive<f32> = 220.0..=640.0;
const THUMBNAIL_WIDTH: u32 = 320;
const THUMBNAIL_HEIGHT: u32 = 180;

/// How long a "went live" toast stays up.
const TOAST_LIFETIME: Duration = Duration::from_secs(8);

/// Emotes and thumbnails are reproducible, so they live in local app data
/// rather than roaming with settings.
fn image_cache_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("nativetwitch")
        .join("images")
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

    /// The settings panel, present only while open.
    settings_panel: Option<Entity<SettingsPanel>>,

    /// Chat-pane width at the moment a drag started, plus where it started.
    /// `None` when not dragging.
    resize: Option<(f32, f32)>,

    follows: Vec<LiveStream>,
    /// Who was live at the last poll, so newly-live channels can be told apart
    /// from ones that were already streaming. Without this every poll would
    /// re-announce everybody.
    known_live: HashSet<String>,
    toasts: Vec<(u64, SharedString)>,
    next_toast: u64,
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
                    this.apply_follows_event(event, cx);
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
            settings_panel: None,
            resize: None,
            state: StreamState::Idle,
            quality: None,
            chat: None,
            supervisor: None,
            _stream_pump: None,
            follows: Vec::new(),
            known_live: HashSet::new(),
            toasts: Vec::new(),
            next_toast: 0,
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

        // Pick quality for the pane we will actually render into, in physical
        // pixels. Using a constant here was what capped everyone at 720p
        // regardless of window size or display.
        let scale = window.scale_factor();
        let pane_height = ((f32::from(window.viewport_size().height) - CHROME_HEIGHT) * scale)
            .round()
            .clamp(180.0, video::MAX_RENDER_HEIGHT as f32) as u32;

        let (supervisor, mut events) =
            StreamSupervisor::start(channel.clone(), pane_height, options);
        let volume = self.settings.volume;

        self._stream_pump = Some(cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;
            while let Some(event) = events.next().await {
                let ok = this.update_in(cx, |this: &mut RootView, window, cx| {
                    match event {
                        StreamEvent::Resolving => this.state = StreamState::Starting,
                        StreamEvent::Ready { url, quality } => {
                            this.quality = Some(SharedString::from(quality.clone()));
                            match VideoStream::start(url, RENDER_WIDTH, RENDER_HEIGHT, volume) {
                                Ok((stream, frames)) => {
                                    let label = SharedString::from(quality);
                                    let view = cx.new(|cx| {
                                        VideoView::from_stream(stream, frames, label, window, cx)
                                    });
                                    // Volume changed from the overlay controls
                                    // should outlive this stream.
                                    cx.subscribe(&view, |this: &mut RootView, _, event, cx| {
                                        let VideoEvent::VolumeChanged(volume) = event;
                                        this.settings.volume = *volume;
                                        if let Err(e) = this.settings.save(&this.settings_path) {
                                            eprintln!("settings: could not save: {e}");
                                        }
                                        cx.notify();
                                    })
                                    .detach();
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
            entry = match thumb {
                Some(path) => entry.child(img(path).w_full().rounded_sm()),
                // A sized placeholder rather than nothing, so entries do not
                // jump as thumbnails arrive.
                None => entry.child(
                    div()
                        .w_full()
                        .h(px(SIDEBAR_WIDTH * 9.0 / 16.0 - 16.0))
                        .rounded_sm()
                        .bg(rgb(0x231e30)),
                ),
            };

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
            .bg(rgba(0x17141ee6))
            .border_r_1()
            .border_color(rgb(0x2e2939))
            .child(header)
            .child(list)
    }

    fn title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
            .bg(rgba(0x1b1822e6))
            .border_b_1()
            .border_color(rgb(0x2e2939))
            .child(
                div()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0xf2eff7))
                    .child(name),
            )
            .child(div().text_xs().text_color(rgb(0x948ca5)).child(subtitle))
            .child(div().flex_1())
            .child(
                div()
                    .id("open-settings")
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .text_color(rgb(0x948ca5))
                    .hover(|style| style.bg(rgb(0x2e2939)).text_color(rgb(0xf2eff7)))
                    .child("settings")
                    .on_click(
                        cx.listener(|this, _event, window, cx| this.toggle_settings(window, cx)),
                    ),
            )
    }
}

impl RootView {
    fn sign_in_status(&self) -> SharedString {
        match &self.sign_in {
            SignIn::SignedIn(login) => format!("Signed in as {login}").into(),
            SignIn::Connecting => "Connecting...".into(),
            SignIn::NeedsClientId => "Not signed in - add a Client ID above.".into(),
            SignIn::AwaitingCode { user_code, .. } => {
                format!("Enter {user_code} at twitch.tv/activate").into()
            }
            SignIn::Error(reason) => reason.clone(),
        }
    }

    fn toggle_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_panel.take().is_some() {
            cx.notify();
            return;
        }

        let panel = cx.new(|cx| {
            SettingsPanel::new(self.settings.clone(), self.sign_in_status(), window, cx)
        });
        cx.subscribe_in(
            &panel,
            window,
            |this: &mut RootView, _, event, window, cx| {
                match event {
                    SettingsEvent::Dismissed => this.settings_panel = None,
                    SettingsEvent::Saved(updated) => {
                        let client_id_changed =
                            this.settings.credentials.client_id != updated.credentials.client_id;
                        let stream_changed = this.settings.quality != updated.quality
                            || this.settings.credentials.auth_token
                                != updated.credentials.auth_token;

                        this.settings = (**updated).clone();
                        if let Err(e) = this.settings.save(&this.settings_path) {
                            eprintln!("settings: could not save: {e}");
                        }
                        this.settings_panel = None;

                        // Apply immediately rather than asking for a restart,
                        // which is the entire reason this panel exists.
                        if client_id_changed {
                            this.restart_follows(window, cx);
                        }
                        if stream_changed {
                            if let Some(channel) = this.channel.clone() {
                                this.channel = None;
                                this.open_channel(channel, window, cx);
                            }
                        }
                    }
                }
                cx.notify();
            },
        )
        .detach();

        self.settings_panel = Some(panel);
        cx.notify();
    }

    /// Restart sign-in after the client id changes.
    fn restart_follows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sign_in = SignIn::Connecting;
        self.follows.clear();
        self.known_live.clear();

        let (service, mut events) = FollowsService::start(self.settings_path.clone());
        self._follows_pump = cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;
            while let Some(event) = events.next().await {
                let ok = this.update(cx, |this: &mut RootView, cx| {
                    this.apply_follows_event(event, cx);
                });
                if ok.is_err() {
                    break;
                }
            }
        });
        self._follows = service;
    }

    fn apply_follows_event(&mut self, event: FollowsEvent, cx: &mut Context<Self>) {
        match event {
            FollowsEvent::NeedsClientId => self.sign_in = SignIn::NeedsClientId,
            FollowsEvent::AwaitingCode {
                user_code,
                verification_uri,
            } => {
                self.sign_in = SignIn::AwaitingCode {
                    user_code: user_code.into(),
                    verification_uri: verification_uri.into(),
                }
            }
            FollowsEvent::SignedIn { login } => self.sign_in = SignIn::SignedIn(login.into()),
            FollowsEvent::Streams(streams) => self.on_streams(streams, cx),
            FollowsEvent::Error(reason) => self.sign_in = SignIn::Error(reason.into()),
        }
        cx.notify();
    }

    /// Take a fresh follows list and announce anyone who just came online.
    ///
    /// The first poll seeds the known set silently: on launch everyone is
    /// "newly" live, and eight toasts at once would be worse than none.
    fn on_streams(&mut self, streams: Vec<LiveStream>, cx: &mut Context<Self>) {
        let now_live: HashSet<String> =
            streams.iter().map(|s| s.user_login.clone()).collect();

        if !self.known_live.is_empty() {
            let mut newly: Vec<&LiveStream> = streams
                .iter()
                .filter(|s| !self.known_live.contains(&s.user_login))
                .collect();
            newly.sort_by(|a, b| b.viewer_count.cmp(&a.viewer_count));
            for stream in newly {
                self.toast(
                    format!("{} went live · {}", stream.display_name, stream.game_name),
                    cx,
                );
            }
        }

        self.known_live = now_live;
        self.follows = streams;
        cx.notify();
    }

    fn toast(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        let id = self.next_toast;
        self.next_toast += 1;
        self.toasts.push((id, text.into()));

        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TOAST_LIFETIME).await;
            let _ = this.update(cx, |this: &mut RootView, cx| {
                this.toasts.retain(|(existing, _)| *existing != id);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn toast_stack(&self) -> impl IntoElement {
        let mut stack = div()
            .absolute()
            .top_4()
            .right_4()
            .flex()
            .flex_col()
            .gap_2()
            .items_end();

        for (_, text) in &self.toasts {
            stack = stack.child(
                div()
                    .px_4()
                    .py_3()
                    .rounded_md()
                    .bg(rgb(0x241d38))
                    .border_l_2()
                    .border_color(rgb(0xb392ff))
                    .shadow_lg()
                    .text_xs()
                    .text_color(rgb(0xf2eff7))
                    .child(text.clone()),
            );
        }
        stack
    }

    /// A drag handle between video and chat.
    ///
    /// Deliberately wider than it looks: a 1px visual line with an 8px hit area,
    /// because a hairline target is miserable to grab.
    fn chat_divider(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("chat-resize")
            .w(px(8.))
            .h_full()
            .flex_none()
            .flex()
            .justify_center()
            .cursor_col_resize()
            .group("divider")
            .child(
                div()
                    .w(px(1.))
                    .h_full()
                    .bg(rgb(0x2e2939))
                    .group_hover("divider", |style| style.bg(rgb(0xb392ff))),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                    this.resize = Some((f32::from(event.position.x), this.settings.chat_width));
                    cx.notify();
                }),
            )
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some((start_x, start_width)) = self.resize else {
            return;
        };
        // Chat is on the right, so dragging left widens it.
        let delta = start_x - f32::from(event.position.x);
        let width = (start_width + delta).clamp(*CHAT_WIDTH_RANGE.start(), *CHAT_WIDTH_RANGE.end());
        if width != self.settings.chat_width {
            self.settings.chat_width = width;
            cx.notify();
        }
    }

    fn on_mouse_up(&mut self, _event: &MouseUpEvent, cx: &mut Context<Self>) {
        if self.resize.take().is_some() {
            // Persist only when the drag ends, not on every pixel.
            if let Err(e) = self.settings.save(&self.settings_path) {
                eprintln!("settings: could not save: {e}");
            }
            cx.notify();
        }
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
            .relative()
            .flex()
            .flex_row()
            .size_full()
            .bg(rgb(0x131118))
            .on_mouse_move(cx.listener(|this, event, _window, cx| this.on_mouse_move(event, cx)))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event, _window, cx| this.on_mouse_up(event, cx)),
            )
            .child(self.sidebar(cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(self.title_bar(cx))
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
                            .child(self.chat_divider(cx))
                            .child(
                                div()
                                    .w(px(self.settings.chat_width))
                                    .h_full()
                                    .flex_none()
                                    .bg(rgba(0x1b1822e6))
                                    .py_2()
                                    .child(chat),
                            ),
                    ),
            )
            .child(self.toast_stack())
            .children(self.settings_panel.clone())
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
        // Must come before any gpui-component widget is constructed.
        gpui_component::init(cx);

        let bounds = Bounds::centered(None, size(px(1700.), px(900.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            // Panels are painted with alpha below, so the compositor's blur
            // shows through them. Where the platform ignores this, the panels
            // simply composite over the opaque window and nothing looks wrong.
            window_background: WindowBackgroundAppearance::Blurred,
            ..Default::default()
        };

        cx.open_window(options, |window, cx| {
            let root = cx.new(|cx| RootView::new(channel.clone(), volume, window, cx));
            // Root is required as the window's first child so overlay
            // layers have somewhere to render.
            cx.new(|cx| gpui_component::Root::new(AnyView::from(root), window, cx))
        })
        .expect("failed to open window");

        cx.activate(true);
    });
}
