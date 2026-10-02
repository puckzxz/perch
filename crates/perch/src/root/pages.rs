//! The two pages, assembled: the browse page with its tab strip and lists,
//! and the watch page with its grid of panes. Each is one function that hands
//! the root's state to the module that draws it, and each is only its own
//! column — the title bar over it and the rail beside it are the root's, drawn
//! once for both.

use std::path::PathBuf;

use gpui::{canvas, div, prelude::*, px, Context, Div, IntoElement, Stateful, Window};
use gpui_component::input::Input;

use super::follows::LiveList;
use super::RootView;
use crate::browse::{Place, Tab};
use crate::watch::{NextUp, PaneInfo, Showing, Slot};
use crate::{browse, channel_page, layout, theme, watch};

impl RootView {
    pub(super) fn browse_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // The body's width, and room at the foot of every list for the mini
        // player while it is up, so the last row can scroll out from under it.
        let room = layout::Room {
            width: self.body(window).width,
            bottom: if self.mini_player_shows() {
                layout::mini_reserve(self.mini_slots().len())
            } else {
                0.0
            },
        };
        // Whether the follows are what the page is showing — Home with
        // nothing taking it over — which is when resting the pointer on the
        // page holds them where they stand, live and offline alike.
        let home = matches!(self.discovery.place(), Place::Tab(Tab::Home));
        if !home {
            self.hold_live(LiveList::Home, false, cx);
        }
        // Whether the channel whose page is open is on right now, which is
        // what its bar offers beside the recordings: the stream, or the chat.
        let channel_live = self
            .discovery
            .channel
            .as_ref()
            .is_some_and(|page| self.stream_info(&page.login).is_some());

        let body = browse::page(
            &self.follows,
            &self.home_offline,
            self.filter.read(cx).value().as_ref(),
            Input::new(&self.filter).cleanable(true).into_any_element(),
            &self.discovery,
            &self.sign_in,
            self.follows_loaded,
            &self.history,
            room,
            &self.cache,
            self.can_add(),
            &self.scrolls,
            channel_live,
            |this: &mut RootView, action, window, cx| this.on_browse_action(action, window, cx),
            cx,
        );
        let body = self
            .holding(LiveList::Home, home, body, cx)
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col();

        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(theme::bg())
            .child(self.tab_strip(cx))
            .child(body)
    }

    /// The browse page's tabs, then Refresh: all that is left of the header
    /// it used to have. The app's name is gone, and the search box and the
    /// sign-in went up into the title bar, which is over both pages. There is
    /// no "watching 2" either: whatever plays while you browse is in the mini
    /// player, whose own controls and pictures go back to it.
    ///
    /// Left-aligned, Refresh included, rather than pushed to the far end: the
    /// toasts arrive at the top-right, and the further from them Refresh sits
    /// the narrower a window has to be before the two meet — see
    /// [`theme::TAB_STRIP_HEIGHT`] for when they do.
    fn tab_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w_full()
            .flex_none()
            // Fixed rather than however tall its contents happen to be; see
            // the constant.
            .h(px(theme::TAB_STRIP_HEIGHT))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::GAP))
            .px(px(theme::PAGE_PAD))
            .border_b_1()
            .border_color(theme::border())
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .gap(px(theme::GAP_TIGHT))
                    .children(Tab::ALL.map(|tab| self.tab_pill(tab, cx))),
            )
            // Beside the tabs because it means "this list", whichever tab or
            // takeover is up; see `refresh`.
            .child(self.pill(
                "refresh",
                if self.refreshing {
                    "Refreshing…".into()
                } else {
                    "Refresh".into()
                },
                cx,
                |this, _window, cx| this.refresh(cx),
            ))
    }

    /// A list of follows, live and offline, with a probe measuring every
    /// frame whether the pointer is over it — measured the way chat measures
    /// its own hold, not taken from `on_hover`, whose value a pointer that
    /// leaves the window without a move never changes. The hover listener is
    /// only there to wake a repaint, so the probe runs again when the pointer
    /// comes or goes.
    /// `active` false says the list is not what this element is showing.
    pub(super) fn holding(
        &self,
        list: LiveList,
        active: bool,
        element: impl IntoElement,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let owner = cx.entity().downgrade();
        let probe = canvas(
            move |bounds, window, cx| {
                let pointed = active
                    && window.is_window_hovered()
                    && bounds.contains(&window.mouse_position());
                owner
                    .update(cx, |this: &mut RootView, cx| {
                        this.hold_live(list, pointed, cx)
                    })
                    .ok();
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        div()
            .id(match list {
                LiveList::Rail => "rail-hold",
                LiveList::Home => "home-hold",
            })
            .relative()
            .on_hover(cx.listener(|_, _: &bool, _window, cx| cx.notify()))
            .child(element)
            .child(probe)
    }

    pub(super) fn watch_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // Home is not on screen, so it holds nothing: its probe is not
        // painted here, and would otherwise leave its last word — the pointer
        // was on the card that opened this page — standing for good.
        self.hold_live(LiveList::Home, false, cx);

        // The panes the page draws, in the order it draws them, and the grid
        // it draws them in: every slot in a cell of its own, or the
        // maximized pane alone in a grid of one, through the same `pane` —
        // the others are drawn as nothing (see `crate::stage`). Both from
        // here, so the page and the root agree about them by construction.
        let slots: Vec<&Slot> = self
            .cells()
            .into_iter()
            .map(|index| &self.slots[index])
            .collect();
        let grid = self.grid(window);
        // Resolved here, for the panes on screen only: `live_info` walks
        // every list the app holds, and doing that per pane per frame inside
        // the page would be the same walk four times over. A pane maximized
        // away is not read at all, its player included, so its frames
        // redraw no window.
        let panes: Vec<PaneInfo> = slots
            .iter()
            .map(|&slot| {
                let showing = self.showing_in_main(slot, cx);
                PaneInfo {
                    player: self.video_in_main(slot).cloned(),
                    showing,
                    // A recording's header speaks for the recording; the live
                    // numbers would be about a different broadcast.
                    live: if slot.is_live() {
                        self.live_info(&slot.channel)
                    } else {
                        None
                    },
                    name: self.display_name(slot).into(),
                    poster: matches!(showing, Showing::Starting { .. })
                        .then(|| self.poster(slot))
                        .flatten(),
                    next: self.next_up(slot, showing),
                    looking: slot.archives.waiting(),
                    start_offered: slot.is_live() && self.start_offered(&slot.channel),
                    pop_out_offered: super::pop_out_offered(),
                }
            })
            .collect();
        // Nothing over the panes but their own controls. A back pill and the
        // rail's button used to float in the top-left corner, over the first
        // pane's header; the rail's button is in the title bar now. What
        // leads off the page is `Esc`, a search from the box up there, and
        // the palette's "Go to" rows. A row in the rail does not: it opens
        // that channel here.
        let active = self
            .active_slot()
            .map(|index| self.slots[index].key.as_str());
        div().size_full().relative().child(watch::page(
            &slots,
            &panes,
            grid,
            self.settings.chat_width,
            self.settings.video_share,
            active,
            self.pane_move.as_deref(),
            window.is_window_hovered(),
            &self.cache,
            |this: &mut RootView, key: &str, action, window, cx| {
                this.on_pane_action(key, action, window, cx)
            },
            |this: &mut RootView, start, window, cx| this.start_resize(start, window, cx),
            |this: &mut RootView, key: &str, inside, cx| {
                // What the header over the picture follows besides the
                // pointer: whether there is a picture to keep clear — one
                // that covers the pane, not a status screen under a player
                // still fading in — and whether a menu on the bar has the
                // space. Looked up by key, as everything a pane sends is.
                let Some(index) = this.slot_index(key) else {
                    return;
                };
                let slot = &this.slots[index];
                // Through the main window's own access to a player, so a
                // pane whose picture is in a window of its own reads as one
                // with nothing to cover, and nothing here reads its player.
                let player = this.video_in_main(slot);
                let picture = player.is_some_and(|view| view.read(cx).covers());
                let menu_open = player.is_some_and(|view| view.read(cx).menu_open());
                let pointed = this.slots[index].point(inside, picture, menu_open);
                // Sticky, unlike `hovered`: a keyboard shortcut has to keep
                // working once the pointer has moved into chat or off the
                // window entirely, and the pane you last looked at is the
                // one you meant. Not while a header is being dragged: the
                // panes it crosses on the way are not being looked at, and
                // the keys stay with the pane dragged, wherever it is let go
                // (`move_pane`).
                if pointed.entered && this.pane_move.is_none() {
                    this.active = Some(this.slots[index].key.clone());
                }
                // Most frames change nothing: the pointer moving within the
                // pane it is already in, the header already where it goes.
                if pointed.changed {
                    cx.notify();
                }
            },
            cx,
        ))
    }

    /// The picture a starting pane is waiting for: the channel's live
    /// preview, which its browse card shows from the same cache entry, or a
    /// recording's own thumbnail. A recording with no picture of its own yet
    /// — Twitch's placeholder while it is being made — borrows its channel's
    /// live preview, since a broadcast still being recorded is still live.
    /// One that has a picture waits for it rather than borrowing meanwhile:
    /// an old broadcast of a channel that is on now would otherwise open
    /// over a picture of what the channel is doing tonight.
    ///
    /// Only for a channel in a Helix list the app has fetched: nothing has
    /// ever fetched a preview for one opened by name, nor for one opened from
    /// the rail's recommendations, whose answers carry none, and a pane with
    /// none waits on black, as before.
    fn poster(&self, slot: &Slot) -> Option<PathBuf> {
        match slot.recording() {
            Some(video) if channel_page::poster_url(video).is_some() => {
                channel_page::video_preview(&self.cache, video)
            }
            _ => self
                .stream_info(&slot.channel)
                .and_then(|stream| browse::stream_preview(&self.cache, stream)),
        }
    }

    /// What a stopped live pane offers next, from its ask's last answer,
    /// which stands while the next ask is out: an offline channel's newest
    /// past broadcast — the first, since they are asked for newest first — or
    /// an ended broadcast's own recording, found by its id inside the moments
    /// around its end ([`channel_page::archive_of`]). With where it was left,
    /// for the card.
    fn next_up<'a>(&'a self, slot: &'a Slot, showing: Showing<'_>) -> Option<NextUp<'a>> {
        let archives = slot.archives.answer()?;
        let video = match showing {
            Showing::Offline => archives.first(),
            Showing::Ended => {
                channel_page::archive_of(archives, slot.broadcast.as_deref(), slot.stalled_at?)
            }
            _ => None,
        }?;
        Some(NextUp {
            video,
            watched: self.history.get(&video.id),
        })
    }
}
