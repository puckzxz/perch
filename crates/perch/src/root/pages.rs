//! The two pages, assembled: the browse page with its tab strip and lists,
//! and the watch page with its grid of panes. Each is one function that hands
//! the root's state to the module that draws it, and each is only its own
//! column — the title bar over it and the rail beside it are the root's, drawn
//! once for both.

use std::path::PathBuf;

use gpui::{canvas, div, prelude::*, px, Context, Div, IntoElement, Stateful, Window};

use super::follows::LiveList;
use super::RootView;
use crate::assets::Icon;
use crate::browse::Tab;
use crate::chat_display::ChatDisplay;
use crate::controls::{self, Variant};
use crate::keys::Hint;
use crate::watch::{NextUp, PaneInfo, Showing, Slot};
use crate::{browse, channel_page, following, layout, theme, watch};

impl RootView {
    pub(super) fn browse_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // The body's width: the browse page has the whole of it.
        let room = layout::Room {
            width: self.body(window).width,
        };
        // Whether the follows are what the page is showing — Following with
        // nothing taking it over — which is when resting the pointer on the
        // page holds them where they stand, live and offline alike.
        let following = self.following_shown();
        if following {
            // Its Pinned in the rail's order, sorted as the rail sorts it as
            // it is drawn, which a folded rail is not (`order_pins`); and
            // its Recommended asked for, which the rail asks for only while
            // it is open (`ask_recommended`).
            self.order_pins();
            self.ask_recommended();
        } else {
            self.hold_live(LiveList::Following, false, cx);
        }
        // Whether the channel whose page is open is on right now, which is
        // what its bar offers beside the recordings: the stream, or the chat.
        let channel_live = self
            .discovery
            .channel
            .as_ref()
            .is_some_and(|page| self.stream_info(&page.login).is_some());
        // When that channel, being off, says it is on next: asked about the
        // first time its page is drawn (`root::schedule`).
        let channel_next = match self.discovery.channel.as_ref() {
            Some(page) if !channel_live => {
                let login = page.login.clone();
                self.ask_schedule(&login);
                self.schedule_words(&login)
            }
            _ => None,
        };
        let standing = channel_page::Standing {
            live: channel_live,
            next: channel_next,
            // And whether it is pinned, which its bar's pin says the other
            // of; see `prefs::set_pinned`.
            pinned: self
                .discovery
                .channel
                .as_ref()
                .is_some_and(|page| self.settings.is_pinned(&page.login)),
        };

        let lists = following::Lists {
            live: &self.follows,
            offline: &self.following_offline,
            asked: &self.pane_streams.streams,
            pinned: &self.pin_order,
            recommended: &self.recommended.shown,
            last_live: &self.last_live,
            offline_open: self.following_offline_open,
            history: &self.history,
            sign_in: &self.sign_in,
            follows_loaded: self.follows_loaded,
        };
        let body = browse::page(
            lists,
            &self.discovery,
            room,
            &self.cache,
            self.can_add(),
            &self.scrolls,
            standing,
            |this: &mut RootView, action, window, cx| this.on_browse_action(action, window, cx),
            cx,
        );
        let body = self
            .holding(LiveList::Following, following, body, cx)
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

    /// The browse page's own strip: back and forward along the trail, the
    /// tabs, then Refresh — all that is left of the header it used to have.
    /// The app's name is gone, and the search box and the sign-in went up
    /// into the title bar, which is over both pages. Back and forward were
    /// the title bar's, beside a way to the first tab; the redesign of 15
    /// October 2026 gave the bar Browse and one search box, and the page its
    /// own way about.
    ///
    /// Left-aligned, Refresh included, rather than pushed to the far end: the
    /// toasts arrive at the top-right, and the further from them Refresh sits
    /// the narrower a window has to be before the two meet — see
    /// [`theme::TAB_STRIP_HEIGHT`] for when they do.
    fn tab_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // Each drawn as waiting while it has nowhere to go: the arrows stay
        // where they are rather than the tabs moving under the pointer, and
        // a press on a waiting one does nothing because there is nothing
        // there to press.
        let back = if self.can_back() {
            controls::icon_button("browse-back", Icon::Back, Variant::Chrome)
                .tooltip(controls::tip(Hint::Back.tooltip("Back")))
                .on_click(cx.listener(|this, _event, window, cx| {
                    this.focus.focus(window);
                    this.go_back(window, cx);
                }))
                .into_any_element()
        } else {
            controls::icon_waiting(Icon::Back).into_any_element()
        };
        let forward = if self.can_forward() {
            controls::icon_button("browse-forward", Icon::Forward, Variant::Chrome)
                .tooltip(controls::tip(Hint::Forward.tooltip("Forward")))
                .on_click(cx.listener(|this, _event, window, cx| {
                    this.focus.focus(window);
                    this.go_forward(window, cx);
                }))
                .into_any_element()
        } else {
            controls::icon_waiting(Icon::Forward).into_any_element()
        };

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
                    .items_center()
                    .gap(px(theme::GAP_TIGHT))
                    .child(back)
                    .child(forward),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(1.0))
                    .h(px(theme::ICON))
                    .bg(theme::border()),
            )
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
                LiveList::Following => "following-hold",
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
        // On the watch page Following is not on screen, so it holds nothing:
        // its probe is not painted, and would otherwise leave its last word —
        // the pointer was on the card that opened this page — standing for
        // good.
        self.hold_live(LiveList::Following, false, cx);

        // A pane that found its channel off says when the channel is on
        // next; its schedule is asked about the first time one is drawn
        // (`root::schedule`), before the panes below borrow the root.
        let offline: Vec<String> = self
            .cells()
            .into_iter()
            .map(|index| &self.slots[index])
            .filter(|slot| self.showing_in_main(slot, cx) == Showing::Offline)
            .map(|slot| slot.channel.clone())
            .collect();
        for login in &offline {
            self.ask_schedule(login);
        }

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
        // A chat options menu whose pane is not drawn on a chat panel any
        // more — closed, its chat hidden by `C`, or another pane given the
        // page — is closed, rather than left to come back up with the panel.
        let menu_drawn = self.chat_menu.as_deref().is_none_or(|open| {
            slots
                .iter()
                .any(|slot| slot.key == open && watch::placement(slot) == watch::Placement::Panel)
        });
        if !menu_drawn {
            self.chat_menu = None;
        }
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
                    chat_menu: (self.chat_menu.as_deref() == Some(slot.key.as_str()))
                        .then(|| ChatDisplay::of(&self.settings)),
                    schedule: (showing == Showing::Offline)
                        .then(|| self.schedule_words(&slot.channel))
                        .flatten()
                        .map(Into::into),
                    avatar: self.pane_avatar(slot),
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
        let stage = div().size_full().relative().child(watch::page(
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
        ));
        stage
    }

    /// The channel's face for a pane's details, at the top of its chat
    /// (`watch::header`): the picture the rail shows for it — a followed
    /// channel's, at the rail's size, or a recommended one's as it came —
    /// through the same cache entry, so it costs no download the rail has
    /// not made. Anyone else has none, and neither does a face not on disk
    /// yet.
    fn pane_avatar(&self, slot: &Slot) -> Option<PathBuf> {
        let login = settings::channel_key(&slot.channel);
        let url = match self.avatars.get(&login) {
            Some(url) => twitch_api::profile_image_at(url, crate::sidebar::AVATAR_REQUEST),
            None => self
                .recommended
                .channel(&login)
                .map(|channel| channel.profile_image_url.clone())
                .filter(|url| !url.is_empty())?,
        };
        self.cache.get_or_request(&url)
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
    /// Only for a channel in a Helix list the app has fetched, or in the
    /// rail's recommendations, which are live when they are listed and whose
    /// preview is at the address any live channel's is
    /// (`recommended::as_stream`), or for one opened by name once its own ask
    /// is answered, since `stream_info` reads `root::pane_streams` last.
    /// Until then a pane with none waits on black.
    fn poster(&self, slot: &Slot) -> Option<PathBuf> {
        match slot.recording() {
            Some(video) if channel_page::poster_url(video).is_some() => {
                channel_page::video_preview(&self.cache, video)
            }
            _ => match self.stream_info(&slot.channel) {
                Some(stream) => browse::stream_preview(&self.cache, stream),
                None => self.recommended.channel(&slot.channel).and_then(|channel| {
                    browse::stream_preview(&self.cache, &crate::recommended::as_stream(channel))
                }),
            },
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
