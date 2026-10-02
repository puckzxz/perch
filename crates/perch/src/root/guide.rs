//! The guide over the watch page, as the root runs it: opening and closing
//! it, what its tabs ask the worker for, what a card in it plays and where,
//! and the panel's shell — where it sits, what closes it, and the veil that
//! keeps the panes under it from taking the pointer. What it shows and the
//! rules it follows are worked out, purely, in `crate::guide`.

use gpui::{
    canvas, div, prelude::*, px, Animation, AnimationExt, AnyElement, Context, MouseDownEvent,
    Point, Window,
};
use twitch_api::{Category, LiveStream};

use super::follows::LiveList;
use super::RootView;
use crate::browse::{Action, Listing};
use crate::guide::{self, Opening, Pick, Press, Tab};
use crate::{theme, veil};

impl RootView {
    /// The Guide button on the bar of the pane `key` names, or on the header
    /// of one with no picture: the guide up from that pane, the one a Watch
    /// in it replaces (`Guide::opener`); moved to that pane, if it is up
    /// from another; or put away, if it is up from this one
    /// (`guide::press`). The pane is made the active one too, as a press on
    /// it would have made it, had the button not stopped the press.
    ///
    /// Either way every menu open over a pane closes first — the press
    /// stopped at the button, so a menu anchored in a pane drawn after it
    /// never heard it — and the rest of the press's run is heard by nothing
    /// (`run_guard`): the guide comes up over where the pointer is, and the
    /// second press of a double-click would land on a card in it.
    ///
    /// A guide the page was too short to draw was put away as it went
    /// undrawn (`guide_panel`), so this never spends a press closing a guide
    /// nobody can see.
    pub(super) fn toggle_guide(&mut self, key: &str, cx: &mut Context<Self>) {
        self.take_rest_of_run();
        self.close_menus(cx);
        if guide::press(self.guide.open, self.guide.opener.as_deref(), key) == Press::Close {
            self.close_guide(cx);
            return;
        }
        self.active = Some(key.to_string());
        self.guide.opener = Some(key.to_string());
        let opening = !self.guide.open;
        self.guide.open = true;
        self.sync_guide_buttons(cx);
        if opening {
            self.fill_guide();
            if self.guide.tab == Tab::Recommended {
                self.update_recommended();
            }
        }
        cx.notify();
    }

    /// Put the guide away, if it is up: its ×, the Guide button, `Esc`, a
    /// press outside it, or something in it watched or added. Returns
    /// whether it was up. Its tab and any category open in it stay, for the
    /// next time.
    pub(super) fn close_guide(&mut self, cx: &mut Context<Self>) -> bool {
        let was = self.shut_guide(cx);
        if was {
            cx.notify();
        }
        was
    }

    /// The guide down, with every Guide button told (`sync_guide_buttons`),
    /// and no repaint asked for: for the root's own render, which puts it away
    /// off the watch page and when the page is too short to draw it, as well
    /// as for [`close_guide`](Self::close_guide). Returns whether it was up.
    pub(super) fn shut_guide(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.guide.open {
            return false;
        }
        self.guide.open = false;
        self.sync_guide_buttons(cx);
        true
    }

    /// Tell every player whether the guide is up from its pane
    /// (`Guide::up_from`), for its bar's Guide button
    /// (`VideoView::guide_from_here`): the one writer of that mirror after
    /// `Start`, called each time the guide comes up, moves or goes. A player
    /// told what it already knows does nothing.
    fn sync_guide_buttons(&mut self, cx: &mut Context<Self>) {
        for slot in &self.slots {
            let here = self.guide.up_from(&slot.key);
            if let Some(view) = slot.video() {
                view.update(cx, |video, cx| video.set_guide_from_here(here, cx));
            }
        }
    }

    /// One of the guide's tab pills; see `Guide::show_tab`.
    fn show_guide_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.guide.show_tab(tab);
        self.fill_guide();
        // Seen here with the rail folded away, the recommendations may be
        // asked for now, as unfolding the rail asks (`ask_recommended`).
        if tab == Tab::Recommended {
            self.update_recommended();
        }
        cx.notify();
    }

    /// Ask for whatever the guide shows, if it is up and that list is empty
    /// and nobody has asked for it yet (`Guide::first_page_due`): through
    /// `fetch`, as the browse page asks, so the list is waited on by its key
    /// and a sign-in still to come is said on it. Called as it opens, as its
    /// tab or category changes, and when sign-in lands or the worker gives
    /// up, as `fill_shown` is for the browse page.
    pub(super) fn fill_guide(&mut self) {
        if !self.guide.open {
            return;
        }
        if let Some(request) = self.guide.first_page_due(&self.discovery) {
            self.fetch(request);
        }
    }

    /// Something pressed in the guide, as the browse page's controls send
    /// it: a card's Watch or `+ Add`, a category, the way back from one, or
    /// Load more. The keys come back to the root first, as for a press on a
    /// pane: the panel blocks the pointer from the root's own focus.
    fn on_guide_action(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window);
        match action {
            Action::Watch(login) => self.guide_pick(Pick::Watch, login, window, cx),
            Action::Add(login) => self.guide_pick(Pick::Add, login, window, cx),
            // Each puts a different list under the pointer, whose second
            // press of a double-click would land on a card or a category in
            // it, so neither hears the rest of its run (`run_guard`).
            Action::OpenCategory(category) => {
                self.take_rest_of_run();
                self.open_guide_category(category, cx);
            }
            Action::CloseCategory => {
                self.take_rest_of_run();
                self.guide.leave_category();
                cx.notify();
            }
            Action::LoadMore => {
                if let Some(request) = self.guide.next_page(&self.discovery) {
                    self.fetch(request);
                    cx.notify();
                }
            }
            // Following's empty state while signed out, as on Home.
            Action::OpenSettings => self.toggle_settings(window, cx),
            // None of the guide's controls send these: its cards offer no
            // past broadcasts, and it has no search, shelves or history.
            Action::Search(_)
            | Action::CloseSearch
            | Action::OpenChannel { .. }
            | Action::CloseChannel
            | Action::ShowShelf(_)
            | Action::WatchVideo(_)
            | Action::AddVideo(_)
            | Action::ForgetVideo(_)
            | Action::ClearHistory
            | Action::ShowTab(_)
            | Action::SetPinned { .. } => {}
        }
    }

    /// A card's Watch or `+ Add` for `login`, carried out where
    /// `guide::opening` says — in place of the pane the guide was opened
    /// from (`guide::watch_target`), or beside the panes — and the guide put
    /// away after, unless `guide::closes_after` says the pick played nothing.
    ///
    /// The rest of the press's run is heard by nothing (`run_guard`): the
    /// pick lands on the release, and the guide gone takes it out from under
    /// the pointer, so the second press of a double-click on a card — which
    /// people often give one — would land on the pane under it, whose own
    /// double-click is fullscreen.
    fn guide_pick(
        &mut self,
        pick: Pick,
        login: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.take_rest_of_run();
        let panes = self.slots.len();
        let open_already = self.slot_index(&login).is_some();
        let active = self
            .active_slot()
            .map(|index| self.slots[index].key.as_str());
        let target = guide::watch_target(self.guide.opener.as_deref(), active, |key| {
            self.slot_index(key).is_some()
        })
        .map(str::to_string);
        match guide::opening(pick, target.as_deref()) {
            Opening::InPlace(key) => {
                let key = key.to_string();
                self.watch_in_place(&key, login, window, cx);
            }
            Opening::Beside => self.on_browse_action(Action::Add(login), window, cx),
            Opening::Alone => self.on_browse_action(Action::Watch(login), window, cx),
        }
        if guide::closes_after(pick, panes, open_already) {
            self.close_guide(cx);
        }
    }

    /// Play `channel` live in place of the pane `key` names: the swap a
    /// stopped pane's last broadcast and `LIVE` take (`replace_with_channel`),
    /// which keeps the pane's place, its sound's hold and the keys, takes no
    /// step on the trail, and chooses the pane `channel` is already in
    /// instead, if it is in one. Noted as watched, and a new seed for the
    /// recommendations, as opening a channel is (`open_channel`).
    fn watch_in_place(
        &mut self,
        key: &str,
        channel: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings.note_watched(&channel) {
            self.save_settings(cx);
        }
        self.replace_with_channel(key, channel, window, cx);
        self.update_recommended();
    }

    /// A category picked under Categories: its streams, inside the guide,
    /// from the top. While the browse page has the same category open, the
    /// guide starts from what that page already has, cursor and all, since
    /// every answer for it now lands in both (`TwitchEvent::CategoryStreams`)
    /// and two copies that started apart would append one page onto the
    /// other's.
    fn open_guide_category(&mut self, category: Category, cx: &mut Context<Self>) {
        self.guide.scrolls.category.set_offset(Point::default());
        let shared = self
            .discovery
            .open
            .as_ref()
            .is_some_and(|open| open.id == category.id);
        self.guide.streams = if shared {
            Listing {
                items: self.discovery.streams.items.clone(),
                next: self.discovery.streams.next.clone(),
            }
        } else {
            Listing::default()
        };
        self.guide.category = Some(category);
        self.fill_guide();
        cx.notify();
    }

    /// The rail's recommendations as the guide draws them: from whichever
    /// list of streams has each, picture and all, or else from what the
    /// rail's answer said, with no picture (`guide::as_stream`), each with
    /// the words that say which channel led to it.
    fn guide_recommended(&self) -> Vec<(LiveStream, String)> {
        self.recommended
            .shown
            .iter()
            .map(|suggestion| {
                let stream = self
                    .stream_info(&suggestion.channel.login)
                    .cloned()
                    .unwrap_or_else(|| guide::as_stream(&suggestion.channel));
                (stream, suggestion.reason.clone())
            })
            .collect()
    }

    /// What the guide leaves behind on a frame it is not drawn: no veil over
    /// the panes, and no hold on the follows from a probe that is not
    /// painted and so cannot let go itself. From the root's render on every
    /// frame of the browse page, and from [`guide_panel`](Self::guide_panel)
    /// while it is put away.
    pub(super) fn guide_not_drawn(&mut self, cx: &mut Context<Self>) {
        veil::lift(cx);
        self.hold_live(LiveList::Guide, false, cx);
    }

    /// The guide's panel, over the foot of the watch page, while it is up and
    /// the page has room for it (`guide::fits`), `guide::panel` big.
    ///
    /// It blocks the pointer over its own area and nowhere else, so the
    /// panes above it are pressed, pointed at and scrolled as ever. Under it,
    /// the panes' own probes would still count the pointer as theirs — a
    /// probe sees through anything — so it measures itself into the veil
    /// every frame (`veil::cover`), and every pane probe asks the veil;
    /// a move of it asks for one more frame, for the probes that ran before
    /// it to read where it is now. The follows and the recommendations hold
    /// still under the pointer here as in the rail and on Home, through the
    /// same probe (`holding`), whose hover listener also wakes a repaint as
    /// the pointer comes onto the panel or leaves it, which the panes'
    /// listeners under it cannot hear.
    ///
    /// A press anywhere outside it puts it away — but not one on the
    /// settings sheet or the palette, which are drawn over it — heard in the
    /// capture phase, so whatever was pressed still hears it too. A press in
    /// it takes the keys back for the root, which the panel hides the press
    /// from. It rises into place as the bar's menus do, mounted only while
    /// up, so a one-shot animation is enough.
    ///
    /// A guide the page is too short for (`guide::fits`), the window made
    /// smaller under it, is put away rather than left up undrawn, where the
    /// next `Esc` or Guide press would be spent closing it.
    pub(super) fn guide_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let body = self.body(window);
        if !self.guide.open || !guide::fits(body) {
            self.shut_guide(cx);
            self.guide_not_drawn(cx);
            return None;
        }
        let panel = guide::panel(body);
        // Built only for the tab that shows it: the root draws every frame
        // a picture plays, and this clones a stream per recommendation.
        let recommended = if self.guide.tab == Tab::Recommended {
            self.guide_recommended()
        } else {
            Vec::new()
        };
        let contents = guide::contents(
            &self.guide,
            guide::Lists {
                follows: &self.follows,
                follows_loaded: self.follows_loaded,
                sign_in: &self.sign_in,
                recommended: &recommended,
                recommended_refused: self.recommended.refused(),
                discovery: &self.discovery,
            },
            panel,
            window.is_window_hovered(),
            &self.cache,
            |this: &mut RootView, tab, _window, cx| this.show_guide_tab(tab, cx),
            // Its ×: the guide gone from under the pointer, like a pick.
            |this: &mut RootView, _window, cx| {
                this.take_rest_of_run();
                this.close_guide(cx);
            },
            |this: &mut RootView, action, window, cx| this.on_guide_action(action, window, cx),
            cx,
        );
        let holds = self.guide.shows().holds();
        let contents = self
            .holding(LiveList::Guide, holds, contents, cx)
            .size_full();

        // A move asks for the frame the probes read it in with
        // `request_animation_frame`, not a notify, which from prepaint asks
        // for none (HANDOFF.md, "A notify from a probe asks for no frame").
        let probe = canvas(
            move |bounds, window, cx| {
                if veil::cover(bounds, window, cx) {
                    window.request_animation_frame();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        let inset = panel.inset;
        Some(
            div()
                .absolute()
                .left(px(inset))
                .bottom(px(inset))
                .w(px(panel.width))
                .h(px(panel.height))
                .occlude()
                .capture_any_mouse_down(cx.listener(|this, _: &MouseDownEvent, window, _cx| {
                    this.focus.focus(window);
                }))
                .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                    if !this.modal_open() {
                        this.close_guide(cx);
                    }
                }))
                .rounded(px(theme::RADIUS_LG))
                .overflow_hidden()
                .bg(theme::bg())
                .border_1()
                .border_color(theme::border())
                .shadow_lg()
                .child(contents)
                .child(probe)
                .with_animation(
                    "guide-rise",
                    Animation::new(theme::MOTION_ENTER).with_easing(theme::ease_enter()),
                    move |panel, delta| {
                        panel
                            .opacity(delta)
                            .bottom(px(inset - guide::RISE * (1.0 - delta)))
                    },
                )
                .into_any_element(),
        )
    }
}
