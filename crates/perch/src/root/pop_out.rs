//! A pane in a window of its own, on top of every other app: moving a pane's
//! picture there and back, and the window itself.
//!
//! The pane stays where it was. It keeps its place in `RootView::slots`, its
//! number, its chat and its header in the main window, whose cell says the
//! picture is elsewhere and offers it back (`watch::Showing::Elsewhere`).
//! Only the player moves, and it is the same player: one `VideoView`, drawn
//! by one window at a time, which `crate::stage` decides and
//! `VideoView::set_place` carries out. Its sound, its position and its
//! stream are untouched by the move. Its quality follows the window it is
//! drawn in, upwards only and swapped in place as anywhere else: a pane out
//! is measured by its own window (`PoppedOut::height`), and one brought
//! back by its cell again.
//!
//! A pane goes out from its header's icon, its bar's More, `P`, the
//! palette, or its tile in the mini player, whose bar also pops out every
//! pane it shows, each into a window of its own ([`pop_out_all_shown`]).
//! It comes back from its pop-out's Bring back, the `Bring back` on its
//! cell, its header's icon, `P` in either window, or the palette.
//!
//! [`pop_out_all_shown`]: RootView::pop_out_all_shown
//!
//! Three rules hold everything here up, each because gpui fails otherwise:
//!
//! - **A pop-out opens and closes only through `cx.defer`.** `open_window`
//!   draws the new window before it returns (gpui's app.rs:943-978), and a
//!   pop-out's render reads the root ([`PopOut`]), which inside a root update
//!   is leased and cannot be read (entity_map.rs:182-184). And a window
//!   cannot update itself while it is being updated, which is what closing
//!   one from its own button would be. A deferred call runs once the update
//!   in hand is over and every window is back in place.
//! - **Nothing in a pop-out calls the root directly.** [`to_root`] is the
//!   only way, and it hands the root the *main* window. Root methods measure
//!   the main window's body for quality and bind the stream pumps they start
//!   to it; handed a pop-out, they would measure the wrong window and bind a
//!   pump to one about to close, stranding a streamlink.
//! - **Closing the main window closes every pop-out** (`main`), or the app
//!   would go on as a player with nothing around it.
//!
//! On top through the platform (`os_window::keep_on_top`), since gpui makes
//! no window topmost on Windows; and so only on Windows, until a Mac has
//! shown what gpui's panels do there ([`offered`]). The pop-out's own keys
//! are `keys::CONTEXT_POPOUT`'s.

use gpui::{
    div, prelude::*, px, size, AnyView, AnyWindowHandle, App, Bounds, Context, DisplayId,
    ElementId, Entity, FocusHandle, Pixels, SharedString, Subscription, TitlebarOptions,
    WeakEntity, Window, WindowBounds, WindowKind, WindowOptions,
};

use super::title_bar::Platform;
use super::RootView;
use crate::video_view::VideoView;
use crate::watch::{self, PaneAction, Showing};
use crate::{controls, keys, layout, motion, os_window, theme, APP_NAME};

/// What the root keeps for a pane in a window of its own, beside the pane:
/// the payload of `crate::stage::Stage`.
pub(super) struct PoppedOut {
    /// The window, once it has opened: `None` from the moment the pane is
    /// popped until the deferred open has run, which is where it comes from.
    pub(super) window: Option<AnyWindowHandle>,
    /// The pop-out's focus, which its window keeps and its player hands the
    /// keys back to (`VideoView::root_focus`). Every window has its own focus
    /// and its own key dispatch, and a handle no element of a window tracks
    /// reaches only that window's outermost node, where none of the pop-out's
    /// keys are bound. Also what tells this pop-out's window from one opened
    /// for an earlier pop-out of the same pane.
    pub(super) focus: FocusHandle,
    /// Where its window is: where it opened, then wherever it is moved or
    /// resized to, as its window reports it ([`PopOut`]'s bounds observer).
    /// What the next pop-out is placed clear of, and where the next one may
    /// open once this one closes (`layout::pop_out_bounds`).
    ///
    /// The window's restore bounds (`Window::window_bounds`), not
    /// `Window::bounds`, so it means what a pop-out opened at it will:
    /// `WindowBounds::Windowed` is placed in the workspace's coordinates,
    /// while `bounds` is the client area in the screen's — off by a taskbar
    /// on the top or the left, and by the frame gpui hides — so a pop-out
    /// reopened at its own `bounds` crept across the screen, a taskbar's
    /// width per close. The main window saves the same thing (`main`).
    pub(super) bounds: Bounds<Pixels>,
    /// How tall the window draws the picture, in that window's own physical
    /// pixels: what the pane's quality is chosen against while it is out
    /// (`RootView::pane_height_for`), since the grid's cell it keeps in the
    /// main window is not what it is drawn in. Its first guess is the bounds
    /// it opens at, at the main window's scale, the display it opens on;
    /// then whatever its window reports ([`PopOut`]'s bounds observer), at
    /// that window's own scale, which on another monitor may not be the main
    /// window's.
    pub(super) height: f32,
}

/// Whether the pop-out is offered on `platform`: only where gpui's source
/// says it can stay on top and answer the pointer, which is Windows, and
/// where the spike that brought it in was checked live (see the handoff's
/// pop-out trap). On macOS gpui's
/// `PopUp` is a panel that hides itself whenever the app is not the active
/// one, by AppKit's default, and gpui counts the pointer over a window only
/// while it is active (window.rs:1727-1737); a `Normal` window there is not
/// on top at all. Until a Mac shows otherwise, nothing offers it there, and
/// the code that would draw it only compiles.
pub(super) fn offered(platform: Platform) -> bool {
    match platform {
        Platform::Windows => true,
        Platform::MacOs | Platform::Other => false,
    }
}

/// Whether the pop-out is offered on this platform; see [`offered`]. Asked
/// by everything that offers it, the player's More menu included, through
/// `root::pop_out_offered`.
pub(crate) fn offered_here() -> bool {
    offered(Platform::CURRENT)
}

/// Run `f` on the root, with the main window: the only way anything in a
/// pop-out reaches the root.
///
/// Deferred, because the pop-out may be the window being updated — a key or
/// its should-close — and then through the main window, because every root
/// method that takes a `Window` means the main one: quality is chosen
/// against the main window's body, and the stream pumps a restart starts are
/// bound to it. Nothing happens once the main window or the root is gone.
fn to_root(
    root: WeakEntity<RootView>,
    main: AnyWindowHandle,
    cx: &mut App,
    f: impl FnOnce(&mut RootView, &mut Window, &mut Context<RootView>) + 'static,
) {
    cx.defer(move |cx| {
        let _ = main.update(cx, |_, window, cx| {
            let _ = root.update(cx, |root, cx| f(root, window, cx));
        });
    });
}

/// A pop-out window's view: the pane's player, or a word while it has none,
/// and the window's own focus and keys. One per popped pane, by key.
///
/// It owns nothing of the pane's. The player is read from the root on every
/// draw (`RootView::pop_out_video`), which answers only while the pane is
/// popped — so this window can never draw a player the main window is
/// drawing — and reading the root makes every change to it redraw this
/// window too: a restart's new player, a stall that brings the pane home.
pub(super) struct PopOut {
    /// The pane, by key, as everything else names a pane.
    key: String,
    root: WeakEntity<RootView>,
    /// The main window, which [`to_root`] hands the root.
    main: AnyWindowHandle,
    /// This window's focus; see `PoppedOut::focus`.
    focus: FocusHandle,
    /// Takes the focus back if it is ever lost, as the root does in the main
    /// window: with nothing focused, no key reaches anything.
    _focus_lost: Subscription,
    /// Tells the root where this window is, and how tall it draws the
    /// picture, whenever it moves or is resized; see `PoppedOut::bounds`
    /// and `PoppedOut::height`.
    _bounds: Subscription,
}

impl PopOut {
    fn new(
        key: String,
        root: WeakEntity<RootView>,
        main: AnyWindowHandle,
        focus: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let _focus_lost = cx.on_focus_lost(window, |this: &mut Self, window, _cx| {
            this.focus.focus(window);
        });
        let _bounds = cx.observe_window_bounds(window, |this: &mut Self, window, cx| {
            let bounds = window.window_bounds().get_bounds();
            // Measured here, in this window, at its own scale: the root only
            // ever has the main window in hand (`to_root`), and on another
            // monitor the two scales can differ.
            let height = f32::from(window.viewport_size().height) * window.scale_factor();
            let (key, focus) = (this.key.clone(), this.focus.clone());
            to_root(this.root.clone(), this.main, cx, move |root, window, cx| {
                root.pop_out_moved(&key, &focus, bounds, height, window, cx)
            });
        });
        window.focus(&focus);
        Self {
            key,
            root,
            main,
            focus,
            _focus_lost,
            _bounds,
        }
    }

    /// The pane's player, while the pane is popped and playing.
    fn video(&self, cx: &App) -> Option<Entity<VideoView>> {
        self.root.upgrade()?.read(cx).pop_out_video(&self.key)
    }

    /// Ask the root for `action` on this pane, as the pane's own controls
    /// would, through [`to_root`].
    fn ask_root(&self, action: PaneAction, cx: &mut App) {
        let key = self.key.clone();
        to_root(self.root.clone(), self.main, cx, move |root, window, cx| {
            root.on_pane_action(&key, action, window, cx)
        });
    }

    fn on_toggle_playback(
        &mut self,
        _: &keys::TogglePlayback,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(video) = self.video(cx) {
            video.update(cx, |video, cx| video.toggle_playback(cx));
        }
    }

    fn on_toggle_mute(
        &mut self,
        _: &keys::ToggleMute,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(video) = self.video(cx) {
            video.update(cx, |video, cx| video.toggle_mute(window, cx));
        }
    }

    fn on_volume_up(&mut self, _: &keys::VolumeUp, window: &mut Window, cx: &mut Context<Self>) {
        self.nudge_volume(keys::VOLUME_STEP, window, cx);
    }

    fn on_volume_down(
        &mut self,
        _: &keys::VolumeDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.nudge_volume(-keys::VOLUME_STEP, window, cx);
    }

    /// The level, with this window: the slider the player keeps takes no
    /// notice of which window moves it.
    fn nudge_volume(&mut self, delta: i16, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(video) = self.video(cx) {
            video.update(cx, |video, cx| video.nudge_volume(delta, window, cx));
        }
    }

    fn on_seek_back(&mut self, _: &keys::SeekBack, _window: &mut Window, cx: &mut Context<Self>) {
        self.seek_by(-keys::SEEK_STEP, cx);
    }

    fn on_seek_forward(
        &mut self,
        _: &keys::SeekForward,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.seek_by(keys::SEEK_STEP, cx);
    }

    fn seek_by(&mut self, delta: f64, cx: &mut Context<Self>) {
        if let Some(video) = self.video(cx) {
            video.update(cx, |video, cx| video.seek_by(delta, cx));
        }
    }

    /// `P` in the pop-out brings the pane back, as Bring back does.
    fn on_toggle_pop_out(
        &mut self,
        _: &keys::TogglePopOut,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ask_root(PaneAction::PopIn, cx);
    }

    /// `Ctrl+W` in the pop-out closes the pane, as its Close does.
    fn on_close_pane(&mut self, _: &keys::ClosePane, _window: &mut Window, cx: &mut Context<Self>) {
        self.ask_root(PaneAction::Close, cx);
    }
}

impl Render for PopOut {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = self.root.upgrade();
        let video = root
            .as_ref()
            .and_then(|root| root.read(cx).pop_out_video(&self.key));
        let (pictured, covered) = video.as_ref().map_or((false, false), |video| {
            let video = video.read(cx);
            (video.has_picture(), video.covers())
        });
        // A word until the picture covers the window, under the player, as
        // the pane and the tile keep one: the player draws nothing before
        // its first frame and leaves saying what is happening to whoever
        // holds it (`VideoView::render`). With no player — a restart from
        // the settings sheet — and with one whose first frame has not
        // decoded or not finished fading in, the pane is starting.
        let word = match &root {
            Some(root) if !covered => root.read(cx).pop_out_word(&self.key),
            _ => None,
        };

        // `track_focus`, not an id: an id here would namespace every element
        // id beneath it, the player's included, as the root's would.
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(theme::player_bg())
            .text_color(theme::text())
            .track_focus(&self.focus)
            .key_context(keys::CONTEXT_POPOUT)
            .on_action(cx.listener(Self::on_toggle_playback))
            .on_action(cx.listener(Self::on_toggle_mute))
            .on_action(cx.listener(Self::on_volume_up))
            .on_action(cx.listener(Self::on_volume_down))
            .on_action(cx.listener(Self::on_seek_back))
            .on_action(cx.listener(Self::on_seek_forward))
            .on_action(cx.listener(Self::on_toggle_pop_out))
            .on_action(cx.listener(Self::on_close_pane))
            .when_some(word, |pop_out, (word, starting)| {
                let word = div()
                    .text_size(px(theme::TEXT_META))
                    .text_color(theme::text_dim())
                    .child(word);
                // Breathing while it starts, like the pane and the tile: a
                // still word reads as a hang.
                let word = if starting {
                    motion::waiting(ElementId::from("pop-out-starting"), word).into_any_element()
                } else {
                    word.into_any_element()
                };
                pop_out.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(word),
                )
            })
            // Until there is a picture, nothing else gives the window a
            // handle; the player's own is under its bar, drawn only with
            // the picture (`VideoView::render`).
            .when(!pictured, |pop_out| {
                pop_out.child(controls::drag_layer("pop-out-drag", window, cx))
            })
            .children(video)
    }
}

/// How a pop-out's window is made: `Normal`, which is the only kind that
/// both resizes and keeps a taskbar entry on Windows, and put on top
/// afterwards (`os_window::keep_on_top`); transparent where a title bar
/// would be, as the main window is, so the title names it to the taskbar and
/// Alt+Tab and nothing is drawn for it; movable, which a caption needs to
/// drag at all; no minimise, since a pop-out minimised is a pane nobody can
/// see; and never smaller than its bar can hold.
fn pop_out_options(
    title: SharedString,
    bounds: Bounds<Pixels>,
    display_id: Option<DisplayId>,
) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some(title),
            appears_transparent: true,
            traffic_light_position: None,
        }),
        kind: WindowKind::Normal,
        is_movable: true,
        is_resizable: true,
        is_minimizable: false,
        display_id,
        window_min_size: Some(size(
            px(theme::POP_OUT_MIN_WIDTH),
            px(theme::POP_OUT_MIN_HEIGHT),
        )),
        ..Default::default()
    }
}

impl RootView {
    /// Move the picture of the pane `key` names into a window of its own,
    /// on top of every other app. `window` is the main one, whatever asked:
    /// the pop-out opens on its display.
    ///
    /// Only where the pop-out is offered ([`offered`]), for a pane with a
    /// player, and not for one already out. The pane is popped first and its
    /// player told (`restage`), so the main window has stopped drawing it
    /// before the pop-out first draws it; the window opens after, deferred.
    pub(super) fn pop_out(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !offered_here() || self.stage.is_popped(key) {
            return;
        }
        let Some(slot) = self.slots.iter().find(|slot| slot.key == key) else {
            return;
        };
        let Some(view) = slot.video() else {
            return;
        };
        let aspect = view
            .read(cx)
            .source_aspect()
            .unwrap_or(layout::VIDEO_ASPECT);
        let title = SharedString::from(format!("{} · {APP_NAME}", self.display_name(slot)));

        let display = window.display(cx);
        let area = display
            .as_ref()
            .map_or_else(|| window.bounds(), |display| display.bounds());
        // Named on Windows only, as the main window's is (`main`): there gpui
        // keeps bounds only on the display it is handed, and macOS would read
        // them relative to it instead.
        let display_id = display
            .filter(|_| cfg!(windows))
            .map(|display| display.id());
        let open: Vec<Bounds<Pixels>> =
            self.stage.iter().map(|(_, popped)| popped.bounds).collect();
        let bounds = layout::pop_out_bounds(area, aspect, &open, self.pop_out_last);

        let focus = cx.focus_handle();
        self.stage.pop_out(
            key,
            PoppedOut {
                window: None,
                focus: focus.clone(),
                bounds,
                height: f32::from(bounds.size.height) * window.scale_factor(),
            },
        );
        self.restage(cx);
        self.open_pop_out(key.to_string(), title, bounds, display_id, focus, cx);
        // Its quality is its window's now, which may be taller than the cell
        // it left: a pop-out opened where a large one last closed. Once the
        // window has opened and said how tall it really is, through the one
        // settle every resize waits out.
        self.on_window_resized(window, cx);
    }

    /// Pop out every pane the mini player shows, each into a window of its
    /// own, stacked clear of one another (`layout::pop_out_bounds`): the mini
    /// bar's `Pop out`, and `P` on the browse page. The mini player goes
    /// once none is left at home (`mini_player_shows`). A pane with no
    /// player to move stays where it is.
    pub(super) fn pop_out_all_shown(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let shown: Vec<String> = self
            .mini_slots()
            .into_iter()
            .filter(|slot| slot.video().is_some())
            .map(|slot| slot.key.clone())
            .collect();
        for key in shown {
            self.pop_out(&key, window, cx);
        }
    }

    /// Bring every popped pane back: `P` on the browse page while the mini
    /// player has nothing to pop out (`mini_pop_out_offered`). Each comes
    /// back as [`pop_in`](Self::pop_in) brings one.
    pub(super) fn pop_in_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let popped: Vec<String> = self.stage.iter().map(|(key, _)| key.to_string()).collect();
        for key in popped {
            self.pop_in(&key, window, cx);
        }
    }

    /// Bring the pane `key` names back from its window, as the pane the keys
    /// talk to: Bring back, the cell's own `Bring back`, the header's icon,
    /// `P` in either window, and the window's own close. `window` is the
    /// main one, whatever asked.
    ///
    /// To somewhere it is drawn. Off the watch page with the mini player
    /// off, the main window draws no pane at home, and `restage` stops one
    /// that would be there with nothing drawing it; so a pane brought back
    /// then brings the watch page up with it. Bring back is asking to see
    /// the pane, not to stop it.
    ///
    /// Chosen as it comes (`choose`), so while another pane has the watch
    /// page, the pane brought back takes it rather than coming home to be
    /// drawn nowhere.
    ///
    /// Its quality is chosen for its cell again: the cell may be taller than
    /// the window it played in, and moving up is a swap in place, with no
    /// black (`sync_quality`).
    pub(super) fn pop_in(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.stage.is_popped(key) {
            return;
        }
        if !self.draws_home_panes() {
            self.go_watch(cx);
        }
        self.come_home(key, cx);
        self.choose(key, window, cx);
        self.sync_quality(window, cx);
        cx.notify();
    }

    /// Take the pane `key` names off the stage and close its window, if it
    /// has one, with no window of the caller's needed: for a pane brought
    /// back, one about to be replaced in place, and a window that could not
    /// open.
    pub(super) fn come_home(&mut self, key: &str, cx: &mut Context<Self>) {
        if let Some(popped) = self.stage.pop_in(key) {
            self.let_pop_out_go(popped, cx);
        }
        self.restage(cx);
    }

    /// Close every pop-out, for the main window closing. Each comes off the
    /// stage too, so a window still on its way to opening is closed the
    /// moment it arrives (`pop_out_opened`).
    pub(crate) fn close_pop_outs(&mut self, cx: &mut Context<Self>) {
        let panes = super::panes::pane_keys(&self.slots);
        for (_, popped) in self.stage.retain(&panes, |_| false) {
            close_window(popped.window, cx);
        }
    }

    /// A pop-out off the stage: its window closed, and where it was kept
    /// for the next pop-out this session (`layout::pop_out_bounds`).
    pub(super) fn let_pop_out_go(&mut self, popped: PoppedOut, cx: &mut Context<Self>) {
        self.pop_out_last = Some(popped.bounds);
        close_window(popped.window, cx);
    }

    /// Where the pop-out `focus` was made for has moved to, or been resized
    /// to, and how tall it draws the picture, in its own physical pixels.
    /// The bounds are kept for placing the next one clear of it, and for
    /// opening one there once it closes. A change of height chooses the
    /// pane's quality again, for its window, once the run of changes has
    /// settled — the one settle a resize of the main window waits out too
    /// (`on_window_resized`, with the main window, which is what `window`
    /// is). Upwards only, as ever, so a pop-out made small keeps what it has.
    /// Nothing for a window left over from an earlier pop-out of the same
    /// pane, which is on its way out.
    fn pop_out_moved(
        &mut self,
        key: &str,
        focus: &FocusHandle,
        bounds: Bounds<Pixels>,
        height: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(popped) = self.stage.popped_mut(key) else {
            return;
        };
        if popped.focus != *focus {
            return;
        }
        popped.bounds = bounds;
        if popped.height.round() != height.round() {
            popped.height = height;
            self.on_window_resized(window, cx);
        }
    }

    /// The player of the pane `key` names, for its pop-out to draw: only
    /// while the pane is popped, so the pop-out and the main window
    /// (`video_in_main`) can never both have it, and only while it plays.
    fn pop_out_video(&self, key: &str) -> Option<Entity<VideoView>> {
        if !self.stage.is_popped(key) {
            return None;
        }
        self.slots
            .iter()
            .find(|slot| slot.key == key)?
            .video()
            .cloned()
    }

    /// What a pop-out says until its picture covers it, and whether that is
    /// the pane starting: the pane's own word, as a mini-player tile would
    /// say it.
    fn pop_out_word(&self, key: &str) -> Option<(&'static str, bool)> {
        if !self.stage.is_popped(key) {
            return None;
        }
        let slot = self.slots.iter().find(|slot| slot.key == key)?;
        // As the pane would read at home with nothing covering it: asked
        // only then, so a player that is there reads as starting.
        let showing = watch::showing(slot, false, false);
        Some((showing.word()?, matches!(showing, Showing::Starting { .. })))
    }

    /// Open the pop-out for the pane `key` names, deferred; see the module.
    /// What it built is handed back to [`pop_out_opened`](Self::pop_out_opened)
    /// to keep, or to close if the pane has come home meanwhile; a window
    /// that would not open brings the pane straight back, and says so.
    fn open_pop_out(
        &self,
        key: String,
        title: SharedString,
        bounds: Bounds<Pixels>,
        display_id: Option<DisplayId>,
        focus: FocusHandle,
        cx: &mut Context<Self>,
    ) {
        let root = cx.entity().downgrade();
        let main = self.main_window;
        cx.defer(move |cx| {
            let build = {
                let key = key.clone();
                let root = root.clone();
                let focus = focus.clone();
                move |window: &mut Window, cx: &mut App| {
                    let view = cx
                        .new(|cx| PopOut::new(key.clone(), root.clone(), main, focus, window, cx));
                    // The window's own close — Alt+F4, the taskbar — brings
                    // the pane back rather than closing the window under it:
                    // the root closes it, as it closes every pop-out. Once
                    // the root is gone there is nothing to come back to.
                    window.on_window_should_close(cx, move |_window, cx| {
                        if root.upgrade().is_none() {
                            return true;
                        }
                        let key = key.clone();
                        to_root(root.clone(), main, cx, move |root, window, cx| {
                            root.pop_in(&key, window, cx)
                        });
                        false
                    });
                    os_window::keep_on_top(window);
                    // gpui-component's widgets — the volume slider — look for
                    // its `Root` at the top of their window.
                    cx.new(|cx| gpui_component::Root::new(AnyView::from(view), window, cx))
                }
            };
            match cx.open_window(pop_out_options(title, bounds, display_id), build) {
                Ok(handle) => {
                    let handle = AnyWindowHandle::from(handle);
                    let kept =
                        root.update(cx, |this, cx| this.pop_out_opened(&key, &focus, handle, cx));
                    if kept.is_err() {
                        let _ = handle.update(cx, |_, window, _cx| window.remove_window());
                    }
                }
                Err(e) => {
                    let _ = root.update(cx, |this, cx| {
                        this.come_home(&key, cx);
                        this.toast(format!("Couldn't open a window: {e}"), cx);
                    });
                }
            }
        });
    }

    /// The pop-out `focus` was made for has opened as `window`: kept, if the
    /// pane `key` names is still out with that focus and no window yet, and
    /// closed otherwise — the pane came home, or was popped out again, while
    /// the window was on its way.
    fn pop_out_opened(
        &mut self,
        key: &str,
        focus: &FocusHandle,
        window: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        let kept = match self.stage.popped_mut(key) {
            Some(popped) if popped.focus == *focus && popped.window.is_none() => {
                popped.window = Some(window);
                true
            }
            _ => false,
        };
        if !kept {
            close_window(Some(window), cx);
        }
    }
}

/// Close a pop-out's window, deferred; see the module. Nothing for a pane
/// whose window has not opened yet: that window closes itself on arrival
/// (`RootView::pop_out_opened`).
fn close_window(window: Option<AnyWindowHandle>, cx: &mut App) {
    let Some(window) = window else {
        return;
    };
    cx.defer(move |cx| {
        let _ = window.update(cx, |_, window, _cx| window.remove_window());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On top and dragged by its picture is something only the Windows
    /// backend can do; elsewhere nothing offers it.
    #[test]
    fn the_pop_out_is_offered_only_where_it_was_proven() {
        assert!(offered(Platform::Windows));
        assert!(!offered(Platform::MacOs));
        assert!(!offered(Platform::Other));
    }
}
