//! Twitch in one native window.
//!
//!     nativetwitch [channel...] [--volume 0-100]
//!
//! Two pages: browse what you follow, and watch up to four of them at once.
//! The watch page is deliberately bare — players and their chats, nothing else
//! — because chrome you stare past for three hours should not be there.

mod browse;
mod chat;
mod follows;
mod layout;
mod settings_view;
mod theme;
mod video;
mod video_view;
mod watch;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use browse::SignIn;
use chat::ChatView;
use emotes::ImageCache;
use follows::{FollowsEvent, FollowsService};
use gpui::{
    div, prelude::*, px, size, AnyView, App, Application, Bounds, Context, ElementId, Entity,
    MouseButton, MouseMoveEvent, MouseUpEvent, SharedString, Task, TitlebarOptions, Window,
    WindowBounds, WindowOptions,
};
use settings::{QualityPreference, Settings};
use settings_view::{SettingsEvent, SettingsPanel};
use streamlink::{StreamEvent, StreamOptions, StreamSupervisor};
use twitch_api::LiveStream;
use video::VideoStream;
use video_view::{VideoEvent, VideoView};
use watch::{Slot, StreamState, MAX_PANES};

pub const APP_NAME: &str = "nativetwitch";

/// Starting render size. Each pane measures itself on the first layout pass and
/// its render thread follows from then on, so this only decides what the first
/// frame or two look like.
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

struct RootView {
    settings: Settings,
    settings_path: PathBuf,
    cache: Arc<ImageCache>,

    page: Page,
    slots: Vec<Slot>,

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
        channels: Vec<String>,
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
            slots: Vec::new(),
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

        // Only channels named on the command line open a stream. Launching
        // straight into whatever was on last time means the app starts costing
        // CPU and bandwidth before anyone has asked it to.
        for (index, channel) in channels.into_iter().take(MAX_PANES).enumerate() {
            view.open_channel(channel, index == 0, window, cx);
        }
        view
    }

    // ── Follows ──────────────────────────────────────────────────────

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

    // ── Streams ──────────────────────────────────────────────────────

    fn slot_index(&self, channel: &str) -> Option<usize> {
        self.slots.iter().position(|slot| slot.channel == channel)
    }

    /// Open `channel`. With `solo`, it becomes the only pane; otherwise it is
    /// added alongside whatever is already playing.
    fn open_channel(
        &mut self,
        channel: String,
        solo: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.page = Page::Watch;

        if self.slot_index(&channel).is_some() {
            // Already open, so switch to it rather than restarting it. Solo
            // closes the others; dropping them stops their streamlink and mpv.
            if solo {
                self.slots.retain(|slot| slot.channel == channel);
            }
            self.set_background(false, cx);
            cx.notify();
            return;
        }

        if solo {
            self.slots.clear();
        } else if self.slots.len() >= MAX_PANES {
            self.toast(format!("already watching {MAX_PANES} streams"), cx);
            return;
        }

        let chat = cx.new(|cx| ChatView::new(channel.clone(), self.cache.clone(), window, cx));
        self.slots.push(Slot {
            channel: channel.clone(),
            quality_override: None,
            state: StreamState::Starting,
            chat,
            supervisor: None,
            pump: None,
        });

        self.start_stream(channel, window, cx);
        self.set_background(false, cx);

        // A record of what was last watched, even though launch no longer
        // reopens it automatically.
        if let Some(first) = self.slots.first() {
            self.settings.last_channel = Some(first.channel.clone());
            if let Err(e) = self.settings.save(&self.settings_path) {
                eprintln!("settings: could not save: {e}");
            }
        }
    }

    /// Start, or restart, streamlink for an existing slot.
    fn start_stream(&mut self, channel: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.slot_index(&channel) else {
            return;
        };

        let quality = self.slots[index]
            .quality_override
            .clone()
            .or(match &self.settings.quality {
                QualityPreference::Auto => None,
                QualityPreference::Fixed(name) => Some(name.clone()),
            });
        let options = StreamOptions {
            quality,
            auth_token: self.settings.credentials.auth_token.clone(),
        };

        // Quality targets the pane this stream will actually render into, in
        // physical pixels. With several panes the window is shared, so each one
        // asks for proportionally less.
        let scale = window.scale_factor();
        let aspect = window_aspect(window);
        let (rows, _) = layout::grid_shape(self.slots.len().max(1), aspect);
        let pane_height = (f32::from(window.viewport_size().height) * scale / rows as f32)
            .round()
            .clamp(180.0, video::MAX_RENDER_HEIGHT as f32) as u32;

        let (supervisor, mut events) =
            StreamSupervisor::start(channel.clone(), pane_height, options);
        let volume = self.settings.volume;

        let pump = cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;
            while let Some(event) = events.next().await {
                let channel = channel.clone();
                let ok = this.update_in(cx, |this: &mut RootView, window, cx| {
                    this.apply_stream_event(&channel, event, volume, window, cx)
                });
                if ok.is_err() {
                    break;
                }
            }
        });

        self.slots[index].supervisor = Some(supervisor);
        self.slots[index].pump = Some(pump);
    }

    fn apply_stream_event(
        &mut self,
        channel: &str,
        event: StreamEvent,
        volume: u8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Look the slot up by channel rather than by a captured index: panes
        // close while streams are still starting, and an index would go stale.
        let Some(index) = self.slot_index(channel) else {
            return;
        };

        match event {
            StreamEvent::Resolving => self.slots[index].state = StreamState::Starting,
            StreamEvent::Ready {
                url,
                quality,
                available,
            } => {
                let started_at = self
                    .follows
                    .iter()
                    .find(|s| s.user_login == channel)
                    .map(|s| s.started_at.clone());

                match VideoStream::start(url, RENDER_WIDTH, RENDER_HEIGHT, volume) {
                    Ok((stream, frames)) => {
                        let label = SharedString::from(quality);
                        let view = cx.new(|cx| {
                            VideoView::from_stream(
                                stream, frames, label, available, started_at, window, cx,
                            )
                        });
                        let owner = channel.to_string();
                        cx.subscribe_in(
                            &view,
                            window,
                            move |this: &mut RootView, _, event, window, cx| match event {
                                VideoEvent::VolumeChanged(volume) => {
                                    // Panes keep independent volume; the most
                                    // recent choice is only a default for the
                                    // next stream opened.
                                    this.settings.volume = *volume;
                                    if let Err(e) = this.settings.save(&this.settings_path) {
                                        eprintln!("settings: could not save: {e}");
                                    }
                                    cx.notify();
                                }
                                VideoEvent::QualityRequested(name) => {
                                    if let Some(index) = this.slot_index(&owner) {
                                        this.slots[index].quality_override = Some(name.clone());
                                        this.slots[index].state = StreamState::Starting;
                                        this.start_stream(owner.clone(), window, cx);
                                        cx.notify();
                                    }
                                }
                            },
                        )
                        .detach();
                        self.slots[index].state = StreamState::Playing(view);
                    }
                    Err(e) => self.slots[index].state = StreamState::Failed(e.to_string().into()),
                }
            }
            StreamEvent::Offline => self.slots[index].state = StreamState::Offline,
            StreamEvent::Failed { reason } => {
                self.slots[index].state = StreamState::Failed(reason.into())
            }
        }
        cx.notify();
    }

    fn close_slot(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.slots.len() {
            // Dropping the slot stops its streamlink and its mpv.
            self.slots.remove(index);
        }
        if self.slots.is_empty() {
            self.page = Page::Browse;
        }
        cx.notify();
    }

    /// Mute or unmute every pane at once, for moving between pages.
    fn set_background(&mut self, background: bool, cx: &mut Context<Self>) {
        for slot in &self.slots {
            if let Some(view) = slot.video() {
                view.update(cx, |video, cx| video.set_background(background, cx));
            }
        }
    }

    /// Leave the watch page, keeping streams as muted thumbnails.
    ///
    /// Thumbnails are genuinely cheaper, not just smaller: render size follows
    /// the element, so a small miniplayer decodes into a small buffer.
    fn go_browse(&mut self, cx: &mut Context<Self>) {
        self.page = Page::Browse;
        self.set_background(true, cx);
        cx.notify();
    }

    fn go_watch(&mut self, cx: &mut Context<Self>) {
        if self.slots.is_empty() {
            return;
        }
        self.page = Page::Watch;
        self.set_background(false, cx);
        cx.notify();
    }

    fn stop_all(&mut self, cx: &mut Context<Self>) {
        self.slots.clear();
        self.page = Page::Browse;
        cx.notify();
    }

    // ── Settings ─────────────────────────────────────────────────────

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
                            let channels: Vec<String> =
                                this.slots.iter().map(|s| s.channel.clone()).collect();
                            for channel in channels {
                                if let Some(index) = this.slot_index(&channel) {
                                    this.slots[index].quality_override = None;
                                    this.slots[index].state = StreamState::Starting;
                                }
                                this.start_stream(channel, window, cx);
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

    fn chat_is_stacked(&self, window: &Window) -> bool {
        let aspect = window_aspect(window);
        let (rows, cols) = layout::grid_shape(self.slots.len().max(1), aspect);
        layout::cell_is_portrait(layout::cell_aspect(aspect, rows, cols))
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, window: &Window, cx: &mut Context<Self>) {
        let Some((origin, start)) = self.resize else {
            return;
        };
        if self.chat_is_stacked(window) {
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

    // ── Chrome ───────────────────────────────────────────────────────

    fn pill(
        &self,
        id: &'static str,
        label: SharedString,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        div()
            .id(ElementId::from(id))
            .px(px(theme::CONTROL_PAD_X))
            .py(px(theme::CONTROL_PAD_Y))
            .rounded_sm()
            .bg(theme::surface_raised())
            .text_size(px(theme::TEXT_LABEL))
            .font_weight(theme::weight_label())
            .line_height(px(theme::LINE_TIGHT))
            .text_color(theme::text_muted())
            .cursor_pointer()
            .hover(|style| style.bg(theme::hover()).text_color(theme::text()))
            .child(label)
            .on_click(cx.listener(move |this, _event, window, cx| on_click(this, window, cx)))
    }

    fn toast_stack(&self) -> impl IntoElement {
        let mut stack = div()
            .absolute()
            .top_4()
            .right_4()
            .flex()
            .flex_col()
            .gap(px(theme::GAP_TIGHT))
            .items_end();

        for (_, text) in &self.toasts {
            stack = stack.child(
                div()
                    .px(px(theme::PANEL_PAD))
                    .py(px(theme::GAP))
                    .rounded_md()
                    .bg(theme::surface_raised())
                    .border_l_2()
                    .border_color(theme::accent())
                    .shadow_lg()
                    .text_size(px(theme::TEXT_META))
                    .line_height(px(theme::LINE_BODY))
                    .text_color(theme::text())
                    .child(text.clone()),
            );
        }
        stack
    }

    /// Muted thumbnails of whatever is playing, shown while browsing.
    fn miniplayers(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if self.slots.is_empty() {
            return None;
        }

        let mut strip = div()
            .absolute()
            .bottom_4()
            .right_4()
            .flex()
            .flex_row()
            .items_end()
            .gap(px(theme::GAP_TIGHT));

        for slot in &self.slots {
            let Some(video) = slot.video() else {
                continue;
            };
            let id = ElementId::from(SharedString::from(format!("mini-{}", slot.channel)));
            strip = strip.child(
                div()
                    .w(px(220.))
                    .rounded_md()
                    .overflow_hidden()
                    .bg(theme::player_bg())
                    .border_1()
                    .border_color(theme::border())
                    .shadow_lg()
                    .child(
                        div()
                            .id(id)
                            .h(px(124.))
                            .w_full()
                            .cursor_pointer()
                            .child(video.clone())
                            .on_click(cx.listener(|this, _event, _window, cx| this.go_watch(cx))),
                    )
                    .child(
                        div()
                            .px(px(theme::ROW_PAD_X))
                            .py(px(theme::CONTROL_PAD_Y))
                            .bg(theme::surface())
                            .text_size(px(theme::TEXT_META))
                            .text_color(theme::text_muted())
                            .child(SharedString::from(format!("{} · muted", slot.channel))),
                    ),
            );
        }

        Some(
            strip
                .child(
                    self.pill("mini-stop", "stop all".into(), cx, |this, _window, cx| {
                        this.stop_all(cx)
                    }),
                )
                .into_any_element(),
        )
    }

    // ── Pages ────────────────────────────────────────────────────────

    fn browse_page(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let watching = self.slots.len();
        let header = div()
            .w_full()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::GAP))
            .px(px(theme::PAGE_PAD))
            .py(px(theme::PANEL_PAD))
            .border_b_1()
            .border_color(theme::border())
            .child(
                div()
                    .text_size(px(theme::TEXT_TITLE))
                    .font_weight(theme::weight_title())
                    .text_color(theme::text())
                    .child(APP_NAME),
            )
            .child(
                div()
                    .text_size(px(theme::TEXT_META))
                    .text_color(theme::text_dim())
                    .child(self.sign_in.summary()),
            )
            .child(div().flex_1())
            .when(watching > 0, |header| {
                header.child(self.pill(
                    "resume",
                    format!("watching {watching}").into(),
                    cx,
                    |this, _window, cx| this.go_watch(cx),
                ))
            })
            .child(
                self.pill("open-settings", "settings".into(), cx, |this, window, cx| {
                    this.toggle_settings(window, cx)
                }),
            );

        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(theme::bg())
            .child(header)
            .child(browse::page(
                &self.follows,
                &self.sign_in,
                &self.cache,
                self.slots.len() < MAX_PANES,
                |this: &mut RootView, channel, window, cx| {
                    this.open_channel(channel, true, window, cx)
                },
                |this: &mut RootView, channel, window, cx| {
                    this.open_channel(channel, false, window, cx)
                },
                cx,
            ))
            .children(self.miniplayers(cx))
    }

    fn watch_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let aspect = window_aspect(window);

        div()
            .size_full()
            .relative()
            .child(watch::page(
                &self.slots,
                aspect,
                self.settings.chat_width,
                self.settings.chat_height,
                |this: &mut RootView, index, _window, cx| this.close_slot(index, cx),
                cx,
            ))
            .child(
                // Page-level navigation, over the grid rather than inside any
                // one pane. Centred at the top because every grid shape has
                // video there; the bottom-left corner belongs to a pane's chat
                // as soon as there is more than one pane.
                div()
                    .absolute()
                    .top(px(theme::GAP_TIGHT))
                    .left_0()
                    .right_0()
                    .flex()
                    .flex_row()
                    .justify_center()
                    .gap(px(theme::GAP_TIGHT))
                    .child(self.pill("back", "follows".into(), cx, |this, _window, cx| {
                        this.go_browse(cx)
                    }))
                    .child(self.pill(
                        "watch-settings",
                        "settings".into(),
                        cx,
                        |this, window, cx| this.toggle_settings(window, cx),
                    )),
            )
    }
}

/// Window width divided by height, guarding against a zero-height window during
/// a minimise.
fn window_aspect(window: &Window) -> f32 {
    let size = window.viewport_size();
    f32::from(size.width) / f32::from(size.height).max(1.0)
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
    let mut channels: Vec<String> = Vec::new();
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
                eprintln!("usage: {APP_NAME} [channel...] [--volume 0-100]");
                eprintln!();
                eprintln!("Name up to {MAX_PANES} channels to open them side by side.");
                eprintln!("With no channel, opens on the follows page.");
                eprintln!("Settings live at {}", settings::default_path().display());
                std::process::exit(0);
            }
            other => channels.push(other.trim_start_matches('#').to_string()),
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
            let root = cx.new(|cx| RootView::new(channels.clone(), volume, window, cx));
            // Root is required as the window's first child so overlay layers
            // have somewhere to render.
            cx.new(|cx| gpui_component::Root::new(AnyView::from(root), window, cx))
        })
        .expect("failed to open window");

        cx.activate(true);
    });
}
