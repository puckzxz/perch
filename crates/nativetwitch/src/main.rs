//! Twitch in one native window.
//!
//!     nativetwitch [channel] [--volume 0-100]
//!
//! Two pages: browse what you follow, and watch one of them. The watch page is
//! deliberately bare — a player and its chat, nothing else — because chrome you
//! stare past for three hours is chrome that should not be there.

mod browse;
mod chat;
mod follows;
mod settings_view;
mod theme;
mod video;
mod video_view;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use browse::SignIn;
use chat::ChatView;
use emotes::ImageCache;
use follows::{FollowsEvent, FollowsService};
use gpui::{
    div, prelude::*, px, size, AnyView, App, Application, Bounds, Context, Entity, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, SharedString, Task, TitlebarOptions, Window,
    WindowBounds, WindowOptions,
};
use settings::{QualityPreference, Settings};
use settings_view::{SettingsEvent, SettingsPanel};
use streamlink::{StreamEvent, StreamOptions, StreamSupervisor};
use twitch_api::LiveStream;
use video::VideoStream;
use video_view::{VideoEvent, VideoView};

pub const APP_NAME: &str = "nativetwitch";

/// Starting render size. The video pane measures itself on the first layout
/// pass and the render thread follows it from then on, so this only decides
/// what the first frame or two look like.
const RENDER_WIDTH: u32 = 1280;
const RENDER_HEIGHT: u32 = 720;

/// How long a "went live" toast stays up.
const TOAST_LIFETIME: Duration = Duration::from_secs(8);

/// Emotes and thumbnails are reproducible, so they live in local app data
/// rather than roaming with settings.
fn image_cache_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(APP_NAME)
        .join("images")
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Page {
    Browse,
    Watch,
}

enum StreamState {
    Starting,
    Playing(Entity<VideoView>),
    Offline,
    Failed(SharedString),
}

struct RootView {
    settings: Settings,
    settings_path: PathBuf,
    cache: Arc<ImageCache>,

    page: Page,

    channel: Option<String>,
    /// A quality chosen from the player controls, overriding the saved
    /// preference until the next channel change.
    quality_override: Option<String>,
    state: StreamState,
    chat: Option<Entity<ChatView>>,
    supervisor: Option<StreamSupervisor>,
    _stream_pump: Option<Task<()>>,

    follows: Vec<LiveStream>,
    /// Who was live at the last poll, so newly-live channels can be told apart
    /// from ones that were already streaming. Without this every poll would
    /// re-announce everybody.
    known_live: HashSet<String>,
    sign_in: SignIn,
    _follows: FollowsService,
    _follows_pump: Task<()>,

    settings_panel: Option<Entity<SettingsPanel>>,
    toasts: Vec<(u64, SharedString)>,
    next_toast: u64,
    /// Chat size at the moment a drag started, plus where it started.
    resize: Option<(f32, f32)>,
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
        // children, so browse cards and chat emotes both pick up new images.
        let cache_pump = cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;
            while cache_ready.next().await.is_some() {
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });

        let (service, follows_pump) = Self::spawn_follows(settings_path.clone(), window, cx);

        let mut view = Self {
            settings,
            settings_path,
            cache,
            page: Page::Browse,
            channel: None,
            quality_override: None,
            state: StreamState::Starting,
            chat: None,
            supervisor: None,
            _stream_pump: None,
            follows: Vec::new(),
            known_live: HashSet::new(),
            sign_in: SignIn::Connecting,
            _follows: service,
            _follows_pump: follows_pump,
            settings_panel: None,
            toasts: Vec::new(),
            next_toast: 0,
            resize: None,
            _cache_pump: cache_pump,
        };

        // Only a channel named on the command line opens a stream. Launching
        // straight into whatever was on last time means the app starts costing
        // CPU and bandwidth before anyone has asked it to.
        if let Some(channel) = channel {
            view.open_channel(channel, window, cx);
        }
        view
    }

    fn spawn_follows(
        settings_path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (FollowsService, Task<()>) {
        let (service, mut events) = FollowsService::start(settings_path);
        let pump = cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;
            while let Some(event) = events.next().await {
                if this
                    .update(cx, |this: &mut RootView, cx| {
                        this.apply_follows_event(event, cx)
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        (service, pump)
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
        let now_live: HashSet<String> = streams.iter().map(|s| s.user_login.clone()).collect();

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

    /// Switch to `channel`, tearing down whatever was playing.
    ///
    /// Dropping the old supervisor is what stops the previous streamlink, so
    /// switching channels never leaves a process behind.
    fn open_channel(&mut self, channel: String, window: &mut Window, cx: &mut Context<Self>) {
        let same = self.channel.as_deref() == Some(channel.as_str());
        if same {
            // Already playing it; just come back to the watch page, which also
            // takes the player out of its muted background mode.
            self.go_watch(cx);
            return;
        }
        self.page = Page::Watch;

        self.supervisor = None;
        self._stream_pump = None;
        self.state = StreamState::Starting;
        if self.channel.as_deref() != Some(channel.as_str()) {
            self.quality_override = None;
        }
        self.channel = Some(channel.clone());
        self.chat =
            Some(cx.new(|cx| ChatView::new(channel.clone(), self.cache.clone(), window, cx)));

        let quality = self.quality_override.clone().or(match &self.settings.quality {
            QualityPreference::Auto => None,
            QualityPreference::Fixed(name) => Some(name.clone()),
        });
        let options = StreamOptions {
            quality,
            auth_token: self.settings.credentials.auth_token.clone(),
        };

        // Pick quality for the pane we will actually render into, in physical
        // pixels, rather than a constant that capped everyone at 720p.
        let scale = window.scale_factor();
        let pane_height = (f32::from(window.viewport_size().height) * scale)
            .round()
            .clamp(180.0, video::MAX_RENDER_HEIGHT as f32) as u32;

        let (supervisor, mut events) =
            StreamSupervisor::start(channel.clone(), pane_height, options);
        let volume = self.settings.volume;

        self._stream_pump = Some(cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;
            while let Some(event) = events.next().await {
                let ok = this.update_in(cx, |this: &mut RootView, window, cx| {
                    this.apply_stream_event(event, volume, window, cx)
                });
                if ok.is_err() {
                    break;
                }
            }
        }));
        self.supervisor = Some(supervisor);

        self.settings.last_channel = Some(channel);
        if let Err(e) = self.settings.save(&self.settings_path) {
            eprintln!("settings: could not save: {e}");
        }
    }

    fn apply_stream_event(
        &mut self,
        event: StreamEvent,
        volume: u8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            StreamEvent::Resolving => self.state = StreamState::Starting,
            StreamEvent::Ready {
                url,
                quality,
                available,
            } => {
                // Uptime comes from the follows list; a channel opened by name
                // simply has none, which is better than inventing one.
                let started_at = self
                    .channel
                    .as_deref()
                    .and_then(|channel| {
                        self.follows.iter().find(|s| s.user_login == channel)
                    })
                    .map(|s| s.started_at.clone());

                match VideoStream::start(url, RENDER_WIDTH, RENDER_HEIGHT, volume) {
                    Ok((stream, frames)) => {
                        let label = SharedString::from(quality);
                        let view = cx.new(|cx| {
                            VideoView::from_stream(
                                stream, frames, label, available, started_at, window, cx,
                            )
                        });
                        cx.subscribe_in(
                            &view,
                            window,
                            |this: &mut RootView, _, event, window, cx| match event {
                                VideoEvent::VolumeChanged(volume) => {
                                    this.settings.volume = *volume;
                                    if let Err(e) = this.settings.save(&this.settings_path) {
                                        eprintln!("settings: could not save: {e}");
                                    }
                                    cx.notify();
                                }
                                VideoEvent::QualityRequested(name) => {
                                    // A one-off override for this session, not a
                                    // saved preference: picking 480p once to save
                                    // bandwidth should not change every future
                                    // stream.
                                    this.quality_override = Some(name.clone());
                                    if let Some(channel) = this.channel.take() {
                                        this.open_channel(channel, window, cx);
                                    }
                                }
                            },
                        )
                        .detach();
                        self.state = StreamState::Playing(view);
                    }
                    Err(e) => self.state = StreamState::Failed(e.to_string().into()),
                }
            }
            StreamEvent::Offline => self.state = StreamState::Offline,
            StreamEvent::Failed { reason } => self.state = StreamState::Failed(reason.into()),
        }
        cx.notify();
    }

    /// Leave the watch page, keeping the stream as a muted thumbnail.
    ///
    /// The thumbnail is genuinely cheaper, not just visually smaller: render
    /// size follows the element, so a 280px miniplayer decodes into a 280px
    /// buffer rather than a full-pane one.
    fn go_browse(&mut self, cx: &mut Context<Self>) {
        self.page = Page::Browse;
        if let StreamState::Playing(view) = &self.state {
            view.update(cx, |video, cx| video.set_background(true, cx));
        }
        cx.notify();
    }

    fn go_watch(&mut self, cx: &mut Context<Self>) {
        self.page = Page::Watch;
        if let StreamState::Playing(view) = &self.state {
            view.update(cx, |video, cx| video.set_background(false, cx));
        }
        cx.notify();
    }

    /// Stop the stream entirely and forget the channel.
    fn close_stream(&mut self, cx: &mut Context<Self>) {
        // Dropping the supervisor is what stops streamlink; dropping the view
        // stops mpv.
        self.supervisor = None;
        self._stream_pump = None;
        self.state = StreamState::Starting;
        self.chat = None;
        self.channel = None;
        self.quality_override = None;
        self.page = Page::Browse;
        cx.notify();
    }

    /// A muted thumbnail of whatever is playing, shown while browsing.
    fn miniplayer(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let StreamState::Playing(video) = &self.state else {
            return None;
        };
        let channel = self.channel.clone()?;

        Some(
            div()
                .absolute()
                .bottom_4()
                .right_4()
                .w(px(280.))
                .rounded_md()
                .overflow_hidden()
                .bg(theme::player_bg())
                .border_1()
                .border_color(theme::border())
                .shadow_lg()
                .group("mini")
                .child(
                    div()
                        .id("mini-video")
                        .h(px(158.))
                        .w_full()
                        .cursor_pointer()
                        .child(video.clone())
                        .on_click(cx.listener(|this, _event, _window, cx| this.go_watch(cx))),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .bg(theme::surface())
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .text_color(theme::text_muted())
                                .child(SharedString::from(format!("{channel} · muted"))),
                        )
                        .child(
                            div()
                                .id("mini-close")
                                .px_2()
                                .rounded_sm()
                                .text_xs()
                                .text_color(theme::text_dim())
                                .cursor_pointer()
                                .hover(|style| {
                                    style.bg(theme::hover()).text_color(theme::danger())
                                })
                                .child("stop")
                                .on_click(
                                    cx.listener(|this, _event, _window, cx| this.close_stream(cx)),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }

    fn toggle_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_panel.take().is_some() {
            cx.notify();
            return;
        }

        let panel = cx.new(|cx| {
            SettingsPanel::new(self.settings.clone(), self.sign_in.summary(), window, cx)
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
                            this.sign_in = SignIn::Connecting;
                            this.follows.clear();
                            this.known_live.clear();
                            let (service, pump) =
                                Self::spawn_follows(this.settings_path.clone(), window, cx);
                            this._follows = service;
                            this._follows_pump = pump;
                        }
                        if stream_changed {
                            if let Some(channel) = this.channel.take() {
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

    // ── Chat sizing ──────────────────────────────────────────────────

    /// Chat sits beside the video on a wide window and below it on a tall one,
    /// which is what a vertical monitor wants without anyone toggling a setting.
    fn is_portrait(&self, window: &Window) -> bool {
        let size = window.viewport_size();
        f32::from(size.width) / f32::from(size.height).max(1.0) < theme::PORTRAIT_ASPECT
    }

    fn chat_divider(&self, portrait: bool, cx: &mut Context<Self>) -> impl IntoElement {
        // A 1px line with an 8px hit area: hairline targets are miserable to
        // grab, but a visible 8px bar would be a permanent piece of chrome.
        let base = div().id("chat-resize").flex_none().group("divider");
        let base = if portrait {
            base.h(px(8.))
                .w_full()
                .cursor_row_resize()
                .flex()
                .items_center()
        } else {
            base.w(px(8.))
                .h_full()
                .cursor_col_resize()
                .flex()
                .justify_center()
        };

        let line = if portrait {
            div().h(px(1.)).w_full()
        } else {
            div().w(px(1.)).h_full()
        };

        base.child(
            line.bg(theme::border())
                .group_hover("divider", |style| style.bg(theme::accent())),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                let (origin, current) = if portrait {
                    (f32::from(event.position.y), this.settings.chat_height)
                } else {
                    (f32::from(event.position.x), this.settings.chat_width)
                };
                this.resize = Some((origin, current));
                cx.notify();
            }),
        )
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, window: &Window, cx: &mut Context<Self>) {
        let Some((origin, start)) = self.resize else {
            return;
        };
        if self.is_portrait(window) {
            // Chat is below, so dragging up makes it taller.
            let delta = origin - f32::from(event.position.y);
            let height = (start + delta).clamp(theme::CHAT_HEIGHT_MIN, theme::CHAT_HEIGHT_MAX);
            if height != self.settings.chat_height {
                self.settings.chat_height = height;
                cx.notify();
            }
        } else {
            let delta = origin - f32::from(event.position.x);
            let width = (start + delta).clamp(theme::CHAT_WIDTH_MIN, theme::CHAT_WIDTH_MAX);
            if width != self.settings.chat_width {
                self.settings.chat_width = width;
                cx.notify();
            }
        }
    }

    fn on_mouse_up(&mut self, _event: &MouseUpEvent, cx: &mut Context<Self>) {
        if self.resize.take().is_some() {
            // Persist when the drag ends, not on every pixel.
            if let Err(e) = self.settings.save(&self.settings_path) {
                eprintln!("settings: could not save: {e}");
            }
            cx.notify();
        }
    }

    fn settings_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("open-settings")
            .px_3()
            .py_1()
            .rounded_sm()
            .text_xs()
            .text_color(theme::text_muted())
            .cursor_pointer()
            .hover(|style| style.bg(theme::hover()).text_color(theme::text()))
            .child("settings")
            .on_click(cx.listener(|this, _event, window, cx| this.toggle_settings(window, cx)))
    }

    // ── Pages ────────────────────────────────────────────────────────

    fn browse_page(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let header = div()
            .w_full()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .px_5()
            .py_3()
            .border_b_1()
            .border_color(theme::border())
            .child(
                div()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(theme::text())
                    .child(APP_NAME),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme::text_dim())
                    .child(self.sign_in.summary()),
            )
            .child(div().flex_1())
            .children(self.channel.clone().map(|channel| {
                div()
                    .id("resume")
                    .px_3()
                    .py_1()
                    .rounded_sm()
                    .text_xs()
                    .text_color(theme::text_muted())
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::hover()).text_color(theme::text()))
                    .child(SharedString::from(format!("back to {channel}")))
                    .on_click(cx.listener(|this, _event, _window, cx| this.go_watch(cx)))
            }))
            .child(self.settings_button(cx));

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::bg())
            .relative()
            .child(header)
            .child(browse::page(
                &self.follows,
                &self.sign_in,
                &self.cache,
                |this: &mut RootView, channel, window, cx| this.open_channel(channel, window, cx),
                cx,
            ))
            .children(self.miniplayer(cx))
    }

    fn watch_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let portrait = self.is_portrait(window);

        let video = match &self.state {
            StreamState::Playing(view) => view.clone().into_any_element(),
            StreamState::Starting => message("starting stream…").into_any_element(),
            StreamState::Offline => message(format!(
                "{} is offline",
                self.channel.as_deref().unwrap_or("channel")
            ))
            .into_any_element(),
            StreamState::Failed(reason) => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme::danger())
                .child(reason.clone())
                .into_any_element(),
        };

        let chat_pane = div()
            .flex_none()
            .bg(theme::surface())
            .map(|pane| {
                if portrait {
                    pane.h(px(self.settings.chat_height)).w_full()
                } else {
                    pane.w(px(self.settings.chat_width)).h_full()
                }
            })
            .children(self.chat.clone());

        let video_pane = div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .bg(theme::player_bg())
            .relative()
            .group("watch")
            .child(video)
            // Navigation lives over the video and fades in on hover, so a
            // window you stare at for hours has no permanent chrome.
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .flex()
                    .flex_row()
                    .gap_1()
                    .p_2()
                    .opacity(0.0)
                    .group_hover("watch", |style| style.opacity(1.0))
                    .child(
                        div()
                            .id("back")
                            .px_3()
                            .py_1()
                            .rounded_sm()
                            .bg(theme::surface_raised())
                            .text_xs()
                            .text_color(theme::text())
                            .cursor_pointer()
                            .hover(|style| style.bg(theme::accent_dim()))
                            .child("follows")
                            .on_click(cx.listener(|this, _event, _window, cx| this.go_browse(cx))),
                    )
                    .child(self.settings_button(cx)),
            );

        div()
            .size_full()
            .flex()
            .bg(theme::bg())
            .map(|root| {
                if portrait {
                    root.flex_col()
                } else {
                    root.flex_row()
                }
            })
            .child(video_pane)
            .child(self.chat_divider(portrait, cx))
            .child(chat_pane)
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
                    .bg(theme::surface_raised())
                    .border_l_2()
                    .border_color(theme::accent())
                    .shadow_lg()
                    .text_xs()
                    .text_color(theme::text())
                    .child(text.clone()),
            );
        }
        stack
    }
}

fn message(text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_color(theme::text_dim())
        .child(text.into())
}

impl Render for RootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = match self.page {
            Page::Browse => self.browse_page(cx).into_any_element(),
            Page::Watch => self.watch_page(window, cx).into_any_element(),
        };

        div()
            .relative()
            .size_full()
            .bg(theme::bg())
            .text_color(theme::text())
            .on_mouse_move(
                cx.listener(|this, event, window, cx| this.on_mouse_move(event, window, cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event, _window, cx| this.on_mouse_up(event, cx)),
            )
            .child(page)
            .child(self.toast_stack())
            .children(self.settings_panel.clone())
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut channel = None;
    let mut volume = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--volume" => {
                volume = args
                    .next()
                    .and_then(|v| v.parse::<u8>().ok())
                    .map(|v| v.min(100))
            }
            "--help" | "-h" => {
                eprintln!("usage: {APP_NAME} [channel] [--volume 0-100]");
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

        let bounds = Bounds::centered(None, size(px(1600.), px(920.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(APP_NAME.into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        cx.open_window(options, |window, cx| {
            let root = cx.new(|cx| RootView::new(channel.clone(), volume, window, cx));
            // Root is required as the window's first child so overlay layers
            // have somewhere to render.
            cx.new(|cx| gpui_component::Root::new(AnyView::from(root), window, cx))
        })
        .expect("failed to open window");

        cx.activate(true);
    });
}
