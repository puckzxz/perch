//! A person's card: who someone in a chat is, and what they have said here.
//!
//! A left press on a speaker's name in a chat line, a resub note's included,
//! or on an `@mention` of a login, opens that person's card in the chat:
//! their picture, their name in their colour and their login where it is
//! another word, `Partner` or `Affiliate`, how old their account is, the
//! badges on their newest line with what each is in words, and up to
//! [`LINES`] of their lines from this chat, newest last, each under its time
//! and drawn as the chat draws it (`ChatView::message_words`). At its foot,
//! `Watch` while they are live and another pane fits, their `Channel page`
//! in Perch while signed in, and `twitch.tv` in the browser. Nothing on it
//! acts on the person: there is no hide, ignore or moderation (HANDOFF,
//! "Product decisions").
//!
//! The lines are found by login among the chat's rows each time the card is
//! drawn ([`recent`], through `RowKind::said`), never kept by index, so a
//! backlog draining from the front never points the card at somebody else's
//! line, and a right press on one opens that row's copy menu by its `seq`.
//! The account is the root's to ask for: Helix, once per login a session,
//! and the open card is told what is known ([`CardFacts`],
//! [`ChatView::set_card_facts`]; `chat_profiles`). Signed out nothing is
//! asked, and the card has what the chat already has, the names and lines.
//!
//! The card is a plain absolute child of the chat's list box, not deferred
//! as the copy menu is, so the palette, the settings sheet and the guide all
//! draw over it. The box has no clip, so [`place`] is what keeps it in: below
//! the pressed line where all of it fits, else on the side with more room,
//! and in a box too short for either it rises from the box's foot, over
//! what is above the box (the pane's header, then its picture or the pane
//! above), which is drawn before the chat, and never down over the next
//! pane, which is drawn after. Where it opened is kept against the list box
//! at the press, so it stays put while the row it came from scrolls away,
//! held inside the box if the box shrinks under it. It holds no rows of its
//! own, and the pointer on it holds the chat's as the pointer anywhere on
//! the chat does (`sync_hold`).
//!
//! It closes on a press anywhere outside it, heard in the capture phase so
//! that the press still lands (a press on another name moves it to that
//! person), on the same name pressed again, on its ×, on `Esc` through
//! `RootView::close_menus`, with the chat put away or hidden
//! (`let_go_hold`), on a replay's seek (`Reset`), and with the pane. While a
//! copy menu opened from one of its lines is up, a press outside the card
//! closes the menu alone, so a menu row hanging past the card's edge does
//! not take the card with it.
//!
//! The same-name toggle needs the name's listener to know that the press on
//! it is the one that has just closed the card, since the card's outside
//! listener runs in the capture phase, before the name hears the press in
//! the bubble phase. So the card notes whose card it closed
//! ([`Cards::press_outside`]), and the name, finding its own login there,
//! leaves it closed. The note is forgotten at the start of every press by
//! the chat's own capture listener, which runs first: a parent's mouse
//! listeners go in before its children are painted (gpui's div.rs,
//! `paint_mouse_listeners`), and the capture phase runs them in that order
//! (window.rs, `dispatch_mouse_event`). That listener is hover-gated, so it
//! is silent for a press on the card itself, which occludes; the card's own
//! capture listener forgets the note then.
//!
//! Its controls act on the press, and only on the first press of a run, as
//! every menu's rows do (HANDOFF, "A menu's rows act on the press"); a test
//! below holds this file to it. `Channel page` leaves the watch page, which
//! with the mini player off stops every pane not in a window of its own
//! (`RootView::retire_homeless`), and Back does not bring them back. So the
//! root decides, at the press, whether going would stop any; if it would,
//! the pill asks instead, `Stop streams and open?` in the accent
//! (`Variant::Confirm`), and only a second press within
//! `theme::CONFIRM_LINGER` goes. A press anywhere else, or the wait running
//! out, puts it back. `Channel page` is offered only signed in: the page
//! asks Helix for its lists with the user's token, and signed out it would
//! say only that, which is no place to stop streams for.

use chrono::Utc;
use gpui::{
    div, img, prelude::*, px, Animation, AnimationExt, AnyElement, App, Context, MouseButton,
    MouseDownEvent, Pixels, Point, ScrollHandle, SharedString, Task, Window,
};
use twitch_chat::ChatMessage;

use super::{ChatView, ChatViewEvent, Row, RowKind};
use crate::assets::Icon;
use crate::chat_badges;
use crate::chat_display::Metrics;
use crate::chat_profiles::{self, Lookup};
use crate::controls::{self, Variant};
use crate::shared_chat;
use crate::theme;

/// How many of a person's lines their card shows, the newest. Enough to
/// see what they are about; every repaint of an open card lays them out,
/// beside the chat's own rows.
pub(super) const LINES: usize = 20;

/// When a typical account was made, for the room the account line keeps
/// while the answer is on its way: its line is drawn unseen, so it takes
/// the lines the answer's will (`card_head`).
const TYPICAL_ACCOUNT: &str = "2016-12-14T12:00:00Z";

/// What the root tells an open card that its chat cannot know itself,
/// mirrored in through [`ChatView::set_card_facts`] when the card opens and
/// whenever it may have changed (`RootView::sync_viewer_cards`).
#[derive(Clone, Debug)]
pub struct CardFacts {
    /// What is known of the person's account; see `chat_profiles`.
    pub profile: Lookup,
    /// Whether `Watch` is offered: they are live by a list the root
    /// already holds, no pane is playing them, and another pane fits.
    pub watchable: bool,
    /// Whether `Channel page` is offered: signed in, since the page asks
    /// Helix for its lists with the user's token and signed out says only
    /// that it needs one.
    pub channel_page: bool,
}

impl Default for CardFacts {
    fn default() -> Self {
        Self {
            profile: Lookup::SignedOut,
            watchable: false,
            channel_page: false,
        }
    }
}

impl CardFacts {
    /// Whether these say what `other` does, so a card told what it already
    /// has does nothing.
    fn same_as(&self, other: &CardFacts) -> bool {
        self.watchable == other.watchable
            && self.channel_page == other.channel_page
            && self.profile.same_as(&other.profile)
    }
}

/// An open card.
pub(super) struct ViewerCard {
    /// Where the name was pressed, against the list box at the press.
    anchor: Point<Pixels>,
    /// Which of this chat's cards it is: what keys its arrival and its
    /// elements, so one opened in place of another starts afresh.
    opened: u64,
    facts: CardFacts,
    /// Where its lines are scrolled to: the newest, when it opens.
    scroll: ScrollHandle,
    /// `Channel page` asking for a second press, with the timer that puts
    /// it back when none comes; dropping it, with the card or on a press
    /// elsewhere, is what cancels the timer.
    confirming: Option<Task<()>>,
}

impl ViewerCard {
    fn new(anchor: Point<Pixels>, opened: u64) -> Self {
        let scroll = ScrollHandle::new();
        // The newest line is nearest to the one whose name was pressed.
        scroll.scroll_to_bottom();
        Self {
            anchor,
            opened,
            facts: CardFacts::default(),
            scroll,
            confirming: None,
        }
    }
}

/// Which card is open, by its person's login, and whose card a press
/// outside it has just closed: the toggle, pure so the order a press is
/// heard in can be tested without a window. `T` is the card.
pub(super) struct Cards<T> {
    open: Option<(String, T)>,
    dismissed: Option<String>,
}

impl<T> Default for Cards<T> {
    fn default() -> Self {
        Self {
            open: None,
            dismissed: None,
        }
    }
}

impl<T> Cards<T> {
    /// A press began in the chat, or on the card: what an earlier press
    /// closed is no longer this one's business.
    pub(super) fn press_began(&mut self) {
        self.dismissed = None;
    }

    /// A press outside the open card: close it, noting whose it was for a
    /// name the same press may land on. Whether one was open.
    pub(super) fn press_outside(&mut self) -> bool {
        match self.open.take() {
            Some((login, _)) => {
                self.dismissed = Some(login);
                true
            }
            None => false,
        }
    }

    /// A press on `login`'s name: open their card, made by `make`, unless
    /// this very press has just closed it (the same name pressed again) or
    /// theirs is open already (their `@mention` in a line on it). Whether a
    /// card was opened.
    pub(super) fn press_name(&mut self, login: &str, make: impl FnOnce() -> T) -> bool {
        let closed_now = self.dismissed.take();
        if closed_now.is_some_and(|closed| closed.eq_ignore_ascii_case(login)) {
            return false;
        }
        if self
            .open
            .as_ref()
            .is_some_and(|(open, _)| open.eq_ignore_ascii_case(login))
        {
            return false;
        }
        self.open = Some((login.to_string(), make()));
        true
    }

    /// Close the open card, if there is one. Whether there was.
    pub(super) fn close(&mut self) -> bool {
        self.open.take().is_some()
    }

    /// The open card, with its person's login.
    pub(super) fn open(&self) -> Option<(&str, &T)> {
        self.open
            .as_ref()
            .map(|(login, card)| (login.as_str(), card))
    }

    pub(super) fn open_mut(&mut self) -> Option<&mut T> {
        self.open.as_mut().map(|(_, card)| card)
    }
}

/// The newest `cap` of `rows` that `login` said, a message of theirs or the
/// note on an event (`RowKind::said`), oldest first. Found by login, in any
/// case, never by index.
pub(super) fn recent<'a>(rows: &'a [Row], login: &str, cap: usize) -> Vec<&'a Row> {
    let mut theirs: Vec<&Row> = rows
        .iter()
        .rev()
        .filter(|row| {
            row.kind
                .said()
                .is_some_and(|message| message.login.eq_ignore_ascii_case(login))
        })
        .take(cap)
        .collect();
    theirs.reverse();
    theirs
}

/// Where a card goes in its list box, in the box's own pixels: its left
/// edge and width, its top or its bottom (the other edge follows its
/// height), and the tallest it may be.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Place {
    pub left: f32,
    pub width: f32,
    pub top: Option<f32>,
    pub bottom: Option<f32>,
    pub max_height: f32,
}

/// Where a card opened by a press at `anchor` goes in a list box of
/// `box_size`, its lines `line` apart.
///
/// [`theme::VIEWER_CARD_WIDTH`] wide, never more than the box less
/// `GAP_TIGHT` a side, its left edge a little before the press and held
/// that far inside either edge. Below the pressed line when all of it,
/// [`theme::VIEWER_CARD_TALLEST`], fits there; otherwise on the side with
/// more room, grown upwards from just above the line on that side, if that
/// room is at least [`theme::VIEWER_CARD_SHORTEST`]; never taller than its
/// room, its lines shrinking and scrolling instead. With neither side that
/// much room, it keeps that least height and stands on the box's foot, so it
/// rises over what is above the box (the pane's header, then its picture
/// where the chat is under it, or the pane above where it is beside it),
/// which is drawn before it, and never falls past its foot into the next
/// pane, which is drawn over it.
///
/// The anchor is held to the box first: it was taken at the press, and the
/// box may have shrunk under it since (the window resized, a pane added),
/// which would otherwise hang the card past the box's foot.
pub(super) fn place(anchor: (f32, f32), box_size: (f32, f32), line: f32) -> Place {
    let (box_width, box_height) = box_size;
    let (x, y) = (anchor.0, anchor.1.clamp(0.0, box_height.max(0.0)));
    let width = theme::VIEWER_CARD_WIDTH
        .min(box_width - 2.0 * theme::GAP_TIGHT)
        .max(0.0);
    let left = (x - theme::GAP_SECTION)
        .min(box_width - theme::GAP_TIGHT - width)
        .max(theme::GAP_TIGHT);

    // Clear of the pressed line by a word's gap, either way.
    let below = y + line / 2.0 + theme::GAP_WORD;
    let above = y - line / 2.0 - theme::GAP_WORD;
    let room_below = box_height - theme::GAP_TIGHT - below;
    let room_above = above - theme::GAP_TIGHT;
    let at = |top: Option<f32>, bottom: Option<f32>, max_height: f32| Place {
        left,
        width,
        top,
        bottom,
        max_height,
    };

    if room_below >= theme::VIEWER_CARD_TALLEST {
        return at(Some(below), None, theme::VIEWER_CARD_TALLEST);
    }
    if room_below.max(room_above) >= theme::VIEWER_CARD_SHORTEST {
        return if room_below >= room_above {
            at(Some(below), None, room_below)
        } else {
            at(
                None,
                Some(box_height - above),
                room_above.min(theme::VIEWER_CARD_TALLEST),
            )
        };
    }
    at(None, Some(theme::GAP_TIGHT), theme::VIEWER_CARD_SHORTEST)
}

/// Whether a card says the login under the name: only where it is another
/// word, not the same one in another case.
fn shows_login(display_name: &str, login: &str) -> bool {
    !display_name.eq_ignore_ascii_case(login)
}

/// The room a row was copied in from, for a Shared Chat line's tag: a
/// message's, or an event's own, which most copied events carry in place of
/// a body (`render_event`).
fn source_room(kind: &RowKind) -> Option<&str> {
    match kind {
        RowKind::Message(message) => message.source_room.as_deref(),
        RowKind::Event(notice) => notice.source_room.as_deref(),
        RowKind::Notice(_) => None,
    }
}

impl ChatView {
    /// The left button's listener on `login`'s name, or on an `@mention` of
    /// them: open their card where the press was, move it to them, or close
    /// it if it is theirs ([`Cards::press_name`]). Only a run's first press,
    /// so a double-click on a name opens its card and leaves it open.
    pub(super) fn name_press(
        login: String,
        cx: &mut Context<Self>,
    ) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
        // By hand rather than through `cx.listener`, as `press_target` is.
        let view = cx.entity().downgrade();
        move |event: &MouseDownEvent, _window: &mut Window, cx: &mut App| {
            if event.click_count > 1 {
                return;
            }
            view.update(cx, |this, cx| this.press_name(&login, event.position, cx))
                .ok();
        }
    }

    fn press_name(&mut self, login: &str, position: Point<Pixels>, cx: &mut Context<Self>) {
        let anchor = position - self.list.viewport_bounds().origin;
        let opened = self.cards_opened + 1;
        if self
            .cards
            .press_name(login, || ViewerCard::new(anchor, opened))
        {
            self.cards_opened = opened;
            cx.emit(ChatViewEvent::CardOpened {
                login: login.to_string(),
            });
        }
        cx.notify();
    }

    /// Whose card is open here, by login: what the root tells it about.
    pub fn card_login(&self) -> Option<&str> {
        self.cards.open().map(|(login, _)| login)
    }

    /// Tell the card open for `login` what the root knows (`tell_card`).
    /// Nothing for a card that is somebody else's by now, or that knows it
    /// already.
    pub fn set_card_facts(&mut self, login: &str, facts: CardFacts, cx: &mut Context<Self>) {
        if self
            .card_login()
            .is_none_or(|open| !open.eq_ignore_ascii_case(login))
        {
            return;
        }
        let Some(card) = self.cards.open_mut() else {
            return;
        };
        if card.facts.same_as(&facts) {
            return;
        }
        card.facts = facts;
        cx.notify();
    }

    /// Close the card, if one is open. Returns whether one was, so `Esc`
    /// can take back the card before it takes you off the page.
    pub fn close_card(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.cards.close() {
            return false;
        }
        cx.notify();
        true
    }

    /// The root's answer to a first press of `Channel page` where going
    /// would stop streams: ask for a second one, for
    /// [`theme::CONFIRM_LINGER`].
    pub fn confirm_leaving(&mut self, cx: &mut Context<Self>) {
        let Some(card) = self.cards.open_mut() else {
            return;
        };
        card.confirming = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(theme::CONFIRM_LINGER).await;
            this.update(cx, |this: &mut ChatView, cx| this.stop_confirming(cx))
                .ok();
        }));
        cx.notify();
    }

    /// Put `Channel page` back to its words: the wait ran out, or a press
    /// landed anywhere but on it.
    fn stop_confirming(&mut self, cx: &mut Context<Self>) {
        if let Some(card) = self.cards.open_mut() {
            if card.confirming.take().is_some() {
                cx.notify();
            }
        }
    }

    /// The open card, for the chat's list box to lay over its rows: see the
    /// module.
    pub(super) fn viewer_card(
        &self,
        metrics: &Metrics,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (login, card) = self.cards.open()?;
        let viewport = self.list.viewport_bounds();
        let place = place(
            (f32::from(card.anchor.x), f32::from(card.anchor.y)),
            (
                f32::from(viewport.size.width),
                f32::from(viewport.size.height),
            ),
            metrics.line,
        );
        let lines = recent(&self.rows, login, LINES);
        let newest = lines.last().and_then(|row| row.kind.said());
        let known = match &card.facts.profile {
            Lookup::Known(user) => Some(user.as_ref()),
            _ => None,
        };
        // As their newest line names them, then as Twitch does, then their
        // login; in their colour from a line, held to read on the card.
        let name = newest
            .map(|message| message.display_name.clone())
            .filter(|name| !name.is_empty())
            .or_else(|| known.map(|user| user.display_name.clone()))
            .unwrap_or_else(|| login.to_string());
        let color = newest
            .map(|message| message.color)
            .or_else(|| self.colors.get(login).copied())
            .map_or_else(theme::text, |color| {
                theme::readable_on(color, theme::surface_raised())
            });

        let head = self.card_head(login, &name, color, card, window, cx);
        let badges = newest.and_then(|message| self.card_badges(message, metrics));
        let well = self.card_lines(&lines, card, metrics, cx);
        let foot = self.card_foot(login, &name, card, cx);

        let opened = card.opened as usize;
        Some(
            div()
                .id(("viewer-card", opened))
                .absolute()
                .left(px(place.left))
                .w(px(place.width))
                .when_some(place.top, |card, top| card.top(px(top)))
                .when_some(place.bottom, |card, bottom| card.bottom(px(bottom)))
                .max_h(px(place.max_height))
                .flex()
                .flex_col()
                .rounded(px(theme::RADIUS_LG))
                .overflow_hidden()
                .bg(theme::surface_raised())
                .border_1()
                .border_color(theme::border())
                .shadow_lg()
                // Over the rows and the pills at their foot, whose links and
                // names would otherwise hear a press on it too.
                .occlude()
                // Every press on it, before anything on it acts: the root's
                // own focus never hears one, and what an earlier press closed
                // is forgotten, as the chat's listener does for a press
                // anywhere else in the chat.
                .capture_any_mouse_down(cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                    this.cards.press_began();
                    cx.emit(ChatViewEvent::ReturnKeys);
                }))
                // A run's first press only: the second of a double-click
                // that opened the card is not a press elsewhere. With a copy
                // menu from one of its lines up, the menu goes alone.
                .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                    if event.click_count == 1 && this.menu.is_none() && this.cards.press_outside() {
                        cx.notify();
                    }
                }))
                .child(head)
                .children(badges)
                .child(well)
                .child(foot)
                // Mounted only while open, so a one-shot is enough, as the
                // copy menu's is; it opens at the pointer, so it fades in
                // without a rise.
                .with_animation(
                    ("viewer-card-arrival", opened),
                    Animation::new(theme::MOTION_ENTER).with_easing(theme::ease_enter()),
                    |card, delta| card.opacity(delta),
                )
                .into_any_element(),
        )
    }

    /// The card's head: the picture, the name and what it is, the login
    /// where it is another word, how old the account is, and the ×. Its own
    /// text sizes, as the copy menu's are, rather than the chat's.
    ///
    /// The picture and the account's line keep their room while the answer
    /// is on its way, so the card does not jump when it lands: the line is
    /// drawn unseen for a typical account ([`TYPICAL_ACCOUNT`]), so it wraps
    /// where the answer's will, which in the head's narrow column is likely
    /// onto a second line. An account with no picture keeps the empty
    /// circle. With no account to show, signed out or none found, both go.
    fn card_head(
        &self,
        login: &str,
        name: &str,
        color: gpui::Hsla,
        card: &ViewerCard,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let size = px(theme::VIEWER_CARD_AVATAR);
        let placeholder = || {
            div()
                .flex_none()
                .size(size)
                .rounded_full()
                .bg(theme::surface())
                .into_any_element()
        };
        let account_line = |created_at: &str| {
            chat_profiles::account_line(created_at, Utc::now())
                .map(|line| div().child(SharedString::from(line)))
        };
        let (avatar, tag, account) = match &card.facts.profile {
            Lookup::Known(user) => {
                let picture = (!user.profile_image_url.is_empty())
                    .then(|| {
                        let url = twitch_api::profile_image_at(
                            &user.profile_image_url,
                            chat_profiles::avatar_request(window.scale_factor()),
                        );
                        self.cache.get_or_request(&url)
                    })
                    .flatten();
                let avatar = match picture {
                    Some(path) => img(path)
                        .image_cache(&self.emote_images)
                        .flex_none()
                        .size(size)
                        .rounded_full()
                        .into_any_element(),
                    None => placeholder(),
                };
                (
                    Some(avatar),
                    chat_profiles::broadcaster_tag(user),
                    account_line(&user.created_at),
                )
            }
            Lookup::Asking => (
                Some(placeholder()),
                None,
                account_line(TYPICAL_ACCOUNT).map(|line| line.invisible()),
            ),
            Lookup::SignedOut | Lookup::Unknown => (None, None, None),
        };

        let close = controls::icon_button("viewer-card-close", Icon::Close, Variant::Quiet)
            .when(self.window_hovered, |button| {
                button.tooltip(controls::tip("Close"))
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                    if event.click_count > 1 {
                        return;
                    }
                    this.close_card(cx);
                    cx.emit(ChatViewEvent::TakeRestOfRun);
                }),
            );

        div()
            .flex_none()
            .flex()
            .flex_row()
            .items_start()
            .gap(px(theme::GAP))
            .p(px(theme::PANEL_PAD))
            .text_size(px(theme::TEXT_META))
            .line_height(px(theme::LINE_TIGHT))
            .children(avatar)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(theme::GAP_TIGHT))
                            .child(
                                div()
                                    .flex_shrink()
                                    .min_w_0()
                                    .text_ellipsis()
                                    .line_clamp(1)
                                    .text_size(px(theme::TEXT_TITLE))
                                    .line_height(px(theme::LINE_BODY))
                                    .font_weight(theme::weight_title())
                                    .text_color(color)
                                    .child(SharedString::from(name.to_string())),
                            )
                            // On the card's raised surface a quiet tag's own
                            // chip would vanish, so it takes the well's.
                            .children(tag.map(|tag| controls::quiet_tag(tag).bg(theme::surface()))),
                    )
                    .when(shows_login(name, login), |column| {
                        column.child(
                            div()
                                .text_color(theme::text_dim())
                                .child(SharedString::from(login.to_string())),
                        )
                    })
                    .children(account.map(|line| line.text_color(theme::text_muted()))),
            )
            .child(close)
    }

    /// The badges on their newest line, each its picture and what it is in
    /// words (`chat_badges::tooltip`, `Subscriber, 14 months`), since a card
    /// is read rather than pointed at. Only those this chat has art for, as
    /// its lines draw: none signed out, and then no row.
    fn card_badges(&self, message: &ChatMessage, metrics: &Metrics) -> Option<gpui::Div> {
        let drawn: Vec<_> = message
            .badges
            .iter()
            .filter_map(|badge| self.badges.art(badge).map(|art| (badge, art)))
            .collect();
        if drawn.is_empty() {
            return None;
        }
        let size = px(metrics.badge);
        let chips = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(theme::GAP_TIGHT))
            .children(drawn.into_iter().map(|(badge, art)| {
                let picture = self.cache.get_or_request(&art.url);
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(theme::GAP_WORD))
                    .px(px(theme::GAP_TIGHT))
                    .py(px(theme::TAG_PAD_Y))
                    .rounded(px(theme::RADIUS))
                    .bg(theme::surface())
                    .text_size(px(theme::TEXT_META))
                    .line_height(px(theme::LINE_TIGHT))
                    .text_color(theme::text_muted())
                    .child(
                        div()
                            .flex_none()
                            .size(size)
                            .when_some(picture, |slot, path| {
                                slot.child(img(path).image_cache(&self.emote_images).size(size))
                            }),
                    )
                    .child(chat_badges::tooltip(badge, art))
            }));
        // A wrapping row goes in a `flex_row`, never straight in the card's
        // column, which would size it to its content (see `render_event`).
        Some(
            div()
                .flex_none()
                .w_full()
                .flex()
                .flex_row()
                .px(px(theme::PANEL_PAD))
                .pb(px(theme::PANEL_PAD))
                .child(chips),
        )
    }

    /// Their lines, in a well the card's width on the chat's own surface,
    /// newest last, at the chat's text size: each its time, a Shared Chat
    /// line's tag, then what it says as the chat draws it, a reply's
    /// `@parent` kept since no line above says it here. What shrinks and
    /// scrolls when the card has less room than its lines; laid out as a
    /// block, so the lines keep their own heights inside it.
    fn card_lines(
        &self,
        lines: &[&Row],
        card: &ViewerCard,
        metrics: &Metrics,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let drawn: Vec<_> = lines
            .iter()
            .filter_map(|row| {
                let message = row.kind.said()?;
                let source = shared_chat::label(source_room(&row.kind), &self.rooms);
                let head = Self::word_line()
                    .child(Self::row_time(row.stamp.clone()))
                    .children(source.map(|label| self.source_tag(row, label)));
                Some(
                    div()
                        .w_full()
                        .flex()
                        .flex_row()
                        .px(px(theme::ROW_PAD_X))
                        .py(px(metrics.row_pad_y))
                        // As on the row itself; see `copy_menu`.
                        .on_mouse_down(MouseButton::Right, Self::row_menu(row.seq, cx))
                        .child(self.message_words(
                            head,
                            row,
                            message,
                            theme::surface(),
                            metrics,
                            false,
                            cx,
                        )),
                )
            })
            .collect();
        let empty = drawn.is_empty();
        div()
            .id("viewer-card-lines")
            .track_scroll(&card.scroll)
            .overflow_y_scroll()
            .flex_shrink()
            .min_h_0()
            .py(px(theme::GAP_WORD))
            .bg(theme::surface())
            .border_t_1()
            .border_b_1()
            .border_color(theme::divider())
            .when(empty, |well| {
                well.child(
                    div()
                        .px(px(theme::ROW_PAD_X))
                        .py(px(theme::GAP_TIGHT))
                        .text_size(px(theme::TEXT_META))
                        .line_height(px(theme::LINE_TIGHT))
                        .text_color(theme::text_dim())
                        .child("No recent messages from them"),
                )
            })
            .children(drawn)
    }

    /// The card's foot: `Watch` while the root says it is offered, `Channel
    /// page` while signed in, or the question it turns into, and
    /// `twitch.tv`. One row of pills that wraps in a narrow chat, each
    /// acting on the press, the first of a run only, with its tooltip while
    /// the pointer is in the window, since the left-hand pane's chat runs to
    /// the window's edge. The tooltips that name the person key their pill's
    /// id on the name, which can change while the card is open (Helix's
    /// display name landing on a mention's card, or a newer line), so a
    /// tooltip up across the change says the new one (HANDOFF, "A tooltip
    /// keeps the words it came up with").
    fn card_foot(
        &self,
        login: &str,
        name: &str,
        card: &ViewerCard,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let hovered = self.window_hovered;

        let watch = card.facts.watchable.then(|| {
            let login = login.to_string();
            controls::pill(
                SharedString::from(format!("viewer-card-watch-{name}")),
                "Watch",
                Variant::Pill,
            )
            .when(hovered, |pill| {
                pill.tooltip(controls::tip(format!(
                    "Watch {name} beside the streams here"
                )))
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                    if event.click_count > 1 {
                        return;
                    }
                    this.close_card(cx);
                    cx.emit(ChatViewEvent::Watch {
                        login: login.clone(),
                    });
                }),
            )
        });

        // Its id follows the question too, so the tooltip says the new words.
        let confirming = card.confirming.is_some();
        let channel = card.facts.channel_page.then(|| {
            let pill = if confirming {
                controls::pill(
                    "viewer-card-channel-confirm",
                    "Stop streams and open?",
                    Variant::Confirm,
                )
                .when(hovered, |pill| {
                    pill.tooltip(controls::tip(
                        "With the mini player off, leaving the watch page stops the streams here",
                    ))
                })
                .on_mouse_down_out(cx.listener(
                    |this, _: &MouseDownEvent, _window, cx| {
                        this.stop_confirming(cx);
                    },
                ))
            } else {
                controls::pill(
                    SharedString::from(format!("viewer-card-channel-{name}")),
                    "Channel page",
                    Variant::Pill,
                )
                .when(hovered, |pill| {
                    pill.tooltip(controls::tip(format!(
                        "Open {name}'s past broadcasts and schedule"
                    )))
                })
            };
            let (login, display_name) = (login.to_string(), name.to_string());
            pill.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, event: &MouseDownEvent, _window, cx| {
                    if event.click_count > 1 {
                        return;
                    }
                    cx.emit(ChatViewEvent::ChannelPage {
                        login: login.clone(),
                        display_name: display_name.clone(),
                        confirmed: confirming,
                    });
                }),
            )
        });

        let twitch = {
            let tip = format!("Open twitch.tv/{login} in your browser");
            let channel = crate::target::Target::Channel(login.to_string());
            controls::pill_with_icon(
                "viewer-card-twitch",
                "twitch.tv",
                Icon::ExternalLink,
                Variant::Pill,
            )
            .when(hovered, |pill| pill.tooltip(controls::tip(tip)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, event: &MouseDownEvent, _window, cx| {
                    if event.click_count > 1 {
                        return;
                    }
                    cx.open_url(&crate::target::link(&channel));
                }),
            )
        };

        // A wrapping row goes in a `flex_row`, never straight in the card's
        // column, which would size it to its content (see `render_event`).
        div()
            .flex_none()
            .w_full()
            .flex()
            .flex_row()
            .p(px(theme::PANEL_PAD))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(theme::GAP_TIGHT))
                    .children(watch)
                    .children(channel)
                    .child(twitch),
            )
    }
}

#[cfg(test)]
mod tests {
    use twitch_chat::message::parse_line;
    use twitch_chat::{ChatMessage, ChatNotice};

    use super::super::{trim_front, Row, RowKind};
    use super::*;

    /// The press heard as gpui hears it: the chat's capture listener, then
    /// the card's outside listener, then the name's.
    #[test]
    fn the_same_name_closes_its_card() {
        let mut cards: Cards<()> = Cards::default();
        cards.press_began();
        assert!(cards.press_name("fisher", || ()));
        assert_eq!(cards.open().map(|(login, _)| login), Some("fisher"));

        // The same name again: the press outside closes it, and the name
        // leaves it closed.
        cards.press_began();
        assert!(cards.press_outside());
        assert!(!cards.press_name("fisher", || ()));
        assert!(cards.open().is_none());

        // Another name moves it.
        cards.press_began();
        assert!(cards.press_name("fisher", || ()));
        cards.press_began();
        assert!(cards.press_outside());
        assert!(cards.press_name("someone", || ()));
        assert_eq!(cards.open().map(|(login, _)| login), Some("someone"));

        // A press elsewhere closes it; the same name on the next press
        // opens it again.
        cards.press_began();
        assert!(cards.press_outside());
        cards.press_began();
        assert!(cards.press_name("someone", || ()));

        // A press in another chat closes it without this chat hearing the
        // press begin; this chat's next press still opens it.
        assert!(cards.press_outside());
        cards.press_began();
        assert!(cards.press_name("someone", || ()));

        // Their own mention, in a line on their card, leaves it open.
        assert!(!cards.press_name("Someone", || ()));
        assert!(cards.open().is_some());
        assert!(cards.close());
        assert!(!cards.close());
    }

    fn message(login: &str, text: &str) -> RowKind {
        let line = format!(":{login}!{login}@{login} PRIVMSG #chan :{text}");
        let line = parse_line(&line).unwrap();
        RowKind::Message(Box::new(ChatMessage::from_irc(&line).unwrap()))
    }

    fn row(kind: RowKind, seq: u64) -> Row {
        Row {
            kind,
            striped: false,
            stamp: "".into(),
            seq,
            deleted: false,
        }
    }

    /// Messages and the notes on events are theirs, Perch's own notices are
    /// nobody's; the newest are kept, oldest first, and still found once the
    /// backlog drains from the front.
    #[test]
    fn recent_lines_are_theirs_newest_last() {
        let subscription = |login: &str| {
            let RowKind::Message(body) = message(login, "three months") else {
                unreachable!()
            };
            RowKind::Event(Box::new(ChatNotice {
                kind: twitch_chat::NoticeKind::Subscription,
                system: format!("{login} subscribed for 3 months."),
                body: Some(*body),
                sent_at: None,
                source_room: None,
                announcement_color: None,
            }))
        };
        let mut rows = vec![
            row(message("fisher", "one"), 1),
            row(message("someone", "not theirs"), 2),
            row(RowKind::Notice("Connected to chan's chat".into()), 3),
            row(subscription("fisher"), 4),
            row(message("fisher", "three"), 5),
            row(message("fisher", "four"), 6),
        ];
        let seqs = |rows: &[&Row]| rows.iter().map(|row| row.seq).collect::<Vec<_>>();
        assert_eq!(seqs(&recent(&rows, "fisher", 10)), [1, 4, 5, 6]);
        assert_eq!(seqs(&recent(&rows, "FISHER", 2)), [5, 6], "the newest two");
        assert!(recent(&rows, "nobody", 10).is_empty());

        assert_eq!(trim_front(&mut rows, 4), 2);
        assert_eq!(seqs(&recent(&rows, "fisher", 10)), [4, 5, 6]);
    }

    /// At the default text size, whose line is 19.
    const LINE: f32 = 19.0;

    #[test]
    fn a_card_opens_below_with_room_for_all_of_it() {
        let place = place((150.0, 100.0), (300.0, 600.0), LINE);
        assert_eq!(place.top, Some(113.5));
        assert_eq!(place.bottom, None);
        assert_eq!(place.max_height, theme::VIEWER_CARD_TALLEST);
    }

    #[test]
    fn a_card_opens_above_near_the_foot() {
        let place = place((150.0, 560.0), (300.0, 600.0), LINE);
        assert_eq!(place.top, None);
        assert_eq!(place.bottom, Some(53.5));
        assert_eq!(place.max_height, theme::VIEWER_CARD_TALLEST);

        // Room on neither side for all of it: the roomier one, no taller
        // than its room.
        let place = super::place((150.0, 250.0), (300.0, 600.0), LINE);
        assert_eq!(place.top, Some(263.5));
        assert_eq!(place.max_height, 600.0 - theme::GAP_TIGHT - 263.5);
        let place = super::place((150.0, 350.0), (300.0, 600.0), LINE);
        assert_eq!(place.bottom, Some(600.0 - 336.5));
        assert_eq!(place.max_height, 336.5 - theme::GAP_TIGHT);
    }

    /// The anchor is the box's at the press; a box that shrinks under it
    /// afterwards (the window resized, a pane added) still holds the card,
    /// which never reaches past the box's foot and rises past its top only
    /// where the box is too short for the least card.
    #[test]
    fn a_card_stays_in_a_box_that_shrank_under_it() {
        let place = place((150.0, 700.0), (300.0, 480.0), LINE);
        assert_eq!(place.top, None);
        assert_eq!(place.bottom, Some(LINE / 2.0 + theme::GAP_WORD));

        for height in [120.0, 200.0, 300.0, 480.0, 800.0] {
            for y in [
                0.0,
                40.0,
                height / 2.0,
                height - 1.0,
                height,
                height + 300.0,
            ] {
                let place = super::place((150.0, y), (300.0, height), LINE);
                let (top, bottom) = match (place.top, place.bottom) {
                    (Some(top), None) => (top, top + place.max_height),
                    (None, Some(bottom)) => (height - bottom - place.max_height, height - bottom),
                    _ => panic!("{y} in {height}: {place:?}"),
                };
                assert!(
                    bottom <= height - theme::GAP_TIGHT + 1e-3,
                    "{y} in {height}: reaches {bottom}"
                );
                if height >= theme::VIEWER_CARD_SHORTEST + 2.0 * theme::GAP_TIGHT {
                    assert!(
                        top >= theme::GAP_TIGHT - 1e-3,
                        "{y} in {height}: rises to {top}"
                    );
                }
            }
        }
    }

    /// The least card keeps its head and its foot whole and still shows a
    /// line of theirs, at every text size, in the worst case for what does
    /// not shrink: a login under the name, the account line wrapped onto a
    /// second line, a row of badges, and the foot wrapped onto two rows
    /// (`Watch` beside the question, or a narrow chat). Only the lines well
    /// shrinks, so what a short card loses is lines, never its controls.
    #[test]
    fn the_least_card_keeps_its_head_and_foot() {
        const BORDER: f32 = 1.0;
        let pill = theme::LINE_TIGHT + 2.0 * theme::CONTROL_PAD_Y;
        let words = theme::LINE_BODY + theme::LINE_TIGHT + 2.0 * theme::LINE_TIGHT;
        let head =
            2.0 * theme::PANEL_PAD + words.max(theme::VIEWER_CARD_AVATAR).max(theme::ICON_BUTTON);
        let well = 2.0 * theme::GAP_WORD + 2.0 * BORDER;
        let foot = 2.0 * theme::PANEL_PAD + 2.0 * pill + theme::GAP_TIGHT;
        for size in settings::ChatTextSize::ALL {
            let metrics = Metrics::of(size);
            let badges =
                metrics.badge.max(theme::LINE_TIGHT) + 2.0 * theme::TAG_PAD_Y + theme::PANEL_PAD;
            let line = metrics.line + 2.0 * metrics.row_pad_y;
            let least = 2.0 * BORDER + head + badges + well + foot + line;
            assert!(
                least <= theme::VIEWER_CARD_SHORTEST,
                "{size:?}: the least card needs {least}"
            );
        }
    }

    #[test]
    fn a_card_stays_inside_its_box() {
        for x in [0.0, 5.0, 150.0, 295.0, 300.0] {
            let place = place((x, 100.0), (300.0, 600.0), LINE);
            assert!(place.left >= theme::GAP_TIGHT, "{x}: {place:?}");
            assert!(
                place.left + place.width <= 300.0 - theme::GAP_TIGHT,
                "{x}: {place:?}"
            );
        }
        let wide = place((200.0, 100.0), (640.0, 600.0), LINE);
        assert_eq!(wide.width, theme::VIEWER_CARD_WIDTH);
        assert_eq!(
            wide.left,
            200.0 - theme::GAP_SECTION,
            "a little before the press"
        );
        let right = place((600.0, 100.0), (640.0, 600.0), LINE);
        assert_eq!(
            right.left,
            640.0 - theme::GAP_TIGHT - theme::VIEWER_CARD_WIDTH
        );

        let narrowest = place((10.0, 100.0), (theme::CHAT_WIDTH_MIN, 600.0), LINE);
        assert_eq!(
            narrowest.width,
            theme::CHAT_WIDTH_MIN - 2.0 * theme::GAP_TIGHT
        );
        assert_eq!(narrowest.left, theme::GAP_TIGHT);
    }

    /// A box too short for the least card: it stands on the box's foot and
    /// rises past its top, over the pane's own picture, rather than falling
    /// into the pane below.
    #[test]
    fn a_short_box_rises_and_never_falls() {
        for y in [10.0, 60.0, 110.0] {
            let place = place((100.0, y), (300.0, 120.0), LINE);
            assert_eq!(place.top, None);
            assert_eq!(place.bottom, Some(theme::GAP_TIGHT));
            assert_eq!(place.max_height, theme::VIEWER_CARD_SHORTEST);
        }
    }

    /// A long word is drawn in pieces sized to fit the narrowest chat's row
    /// (`chat_text::PIECE_CHARS`: sixteen of the widest glyphs at body size,
    /// about 190 pixels), and a card's line has to take one too: the card at
    /// its narrowest, less its border and a row's padding either side.
    #[test]
    fn a_card_line_fits_a_piece_at_the_narrowest_chat() {
        const PIECE: f32 = 190.0;
        const BORDER: f32 = 1.0;
        let card = place((0.0, 0.0), (theme::CHAT_WIDTH_MIN, 600.0), LINE);
        let text = card.width - 2.0 * BORDER - 2.0 * theme::ROW_PAD_X;
        assert!(text >= PIECE, "a card line is {text} px wide");
    }

    #[test]
    fn the_login_shows_only_when_it_is_another_word() {
        assert!(!shows_login("Fisher", "fisher"));
        assert!(!shows_login("FISHER", "fisher"));
        assert!(shows_login("피셔", "fisher"));
        assert!(shows_login("Fisher_", "fisher"));
    }

    /// The copy menu's test, for the card: a control that waited for a
    /// click could miss it, since the press may close the card or take the
    /// page away before the release. Held against the file's own source, as
    /// no test without a window can press one.
    #[test]
    fn card_controls_act_on_the_press() {
        let source = include_str!("viewer_card.rs");
        let code = source
            .split("#[cfg(test)]")
            .next()
            .expect("the file has code above its tests");
        let code: String = code.split_whitespace().collect();
        assert!(
            !code.contains(".on_click("),
            "something on the viewer card waits for a click"
        );
        assert!(
            code.contains(".on_mouse_down(MouseButton::Left,"),
            "the viewer card's controls no longer act on the press"
        );
    }
}
