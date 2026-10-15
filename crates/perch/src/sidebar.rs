//! The follows rail.
//!
//! The channels you follow, in three groups, down the left-hand edge, beside
//! both pages, and a few you do not, in a fourth. It exists because the two
//! halves of this app were further apart than the work is: picking a channel
//! meant leaving the one you were watching, going to a grid of thumbnails,
//! and coming back. Most of the time the question is not "what is on" — it is
//! "who else is live", which is a list of names and numbers and fits in a
//! column.
//!
//! Four groups, top to bottom (see [`groups`]). The channels pinned there,
//! live or not — see [`settings::Settings::pinned`] — followed or not: a pin
//! nobody follows is asked about by itself (`root::pane_streams`), and is
//! live when that says so. Whoever of them is live comes first, by viewers,
//! then the rest by when they were last live; see [`pin_order`]. Who else is
//! live, by viewers. Then a few live channels you do not follow that are like
//! the ones you watch, each saying which channel led to it — see
//! `crate::recommended`, which says where they come from and how rarely they
//! are asked for. Then everyone else followed, as names, each
//! saying when it was last live, folded under a count, because that is the
//! longest list in the app and most evenings nobody wants it. Folded is
//! where it starts each session, and folded it builds no rows and asks for
//! no pictures. The rows all hold still under the pointer; see
//! `RootView::hold_live`. A pin is made and taken off from a followed
//! channel's row itself, under the pointer, from a pane's More and from a
//! channel's page, and saved by `RootView::set_pinned`.
//!
//! On the left, opposite chat. Chat belongs to the pane it is part of and sits
//! on the right of it; the rail belongs to the window. Putting both on the same
//! edge made two unrelated columns of names next to each other, and made
//! switching streams a reach across the whole window from the thing you were
//! reading. The window draws it once, under the title bar and beside whichever
//! page is up, so it does not fade with the page when you change between them.
//!
//! It folds away, and stays folded — see [`settings::Settings::sidebar_collapsed`].
//! A window left open for three hours on one stream should be able to be just
//! the stream. What folds it and brings it back is not here: it is the rail
//! button at the left of the title bar, which is there whether the rail is or
//! not, and `B`. Fullscreen hides the rail along with the bar, folded or not;
//! see `layout::rail_shown`.
//!
//! A live row the pointer rests on brings up its stream's card beside the
//! rail, over the page: see `crate::rail_preview`, which says when and what.
//! The rows measure the pointer for it with probes of their own, as every
//! hover here is measured, and report to the root, which keeps the state.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use emotes::ImageCache;
use gpui::{
    anchored, canvas, deferred, div, img, point, prelude::*, px, relative, Context, ElementId,
    ScrollHandle, SharedString, Window,
};
use gpui_component::scroll::{Scrollbar, ScrollbarShow};
use settings::channel_key;
use twitch_api::{Channel, LiveStream};

use crate::assets::Icon;
use crate::browse::{format_viewers, Action};
use crate::controls;
use crate::guide;
use crate::last_live::LastLive;
use crate::rail_preview;
use crate::recommended::Suggestion;
use crate::theme;

/// How wide the rail is when it is open.
///
/// Narrow enough that a 16:9 video beside it still has room to be a video, wide
/// enough for a name and a game underneath it. Fixed rather than derived: this
/// is a list of one-line rows, and there is nothing for extra width to do.
pub const WIDTH: f32 = 236.0;

/// The avatar. Small on purpose — it is here to be recognised at a glance, not
/// looked at, and a row taller than two lines of text stops the rail being a
/// list.
const AVATAR: f32 = 30.0;

/// Twitch serves profile pictures at a handful of sizes; this is the smallest
/// that still looks right on a HiDPI display at [`AVATAR`].
pub(crate) const AVATAR_REQUEST: u32 = 70;

/// The group a row's hover-revealed controls watch for the pointer.
const ROW_GROUP: &str = "rail-row";

/// Between the live dot and the number it belongs to. Tighter than
/// [`theme::GAP_TIGHT`], because they are one thing rather than two.
const GAP_COUNT: f32 = 4.0;

/// From a row's right-hand edge to a preview card's left: the list's own
/// padding out to the rail's edge, then a gap, so the card stands clear of
/// the rail's border rather than touching it.
const PREVIEW_OFFSET: f32 = theme::GAP_TIGHT * 2.0;

/// What a pin control away from the rail says a press does, with the
/// channel `pinned` or not: a pane's More row (`video_view::menu::pin_row`)
/// and the channel page's pin, which name the rail since they are not
/// beside it. The rail's own pin says `Pin` and `Unpin`.
pub fn pin_words(pinned: bool) -> &'static str {
    if pinned {
        "Unpin from the rail"
    } else {
        "Pin to the rail"
    }
}

/// Whether a window `width` by `height`, under a title bar `title_bar`
/// tall, has room for a preview card where the rail hangs it: the whole
/// card to the right of the rail, and its tallest under the bar, a gap
/// inside the window all round. In a smaller window the snap that keeps
/// the card inside it would slide it left over the row's pin and `+`, or
/// lift it over the title bar, so no card shows there at all.
pub fn previews_fit(width: f32, height: f32, title_bar: f32) -> bool {
    width >= WIDTH + PREVIEW_OFFSET + rail_preview::PREVIEW_WIDTH + theme::GAP
        && height >= title_bar + rail_preview::PREVIEW_TALLEST + 2.0 * theme::GAP
}

/// Everything the rail shows, as the root holds it.
pub struct Rail<'a> {
    /// Who is live: by viewers, or held still under the pointer.
    pub follows: &'a [LiveStream],
    /// Everyone else followed: by name, or held still the same way.
    pub offline: &'a [Channel],
    /// What Helix said of the channels the follows poll does not ask about,
    /// the pins nobody follows among them, when asked by login
    /// (`root::pane_streams`): a pin found here is live.
    pub asked: &'a [LiveStream],
    /// What is pinned, in the order it is drawn: [`pin_order`]'s, or held
    /// still the same way. See [`settings::Settings::pinned`].
    pub pinned: &'a [String],
    /// Live channels like the ones watched, ranked, or held still the same
    /// way; empty whenever the group is hidden. See `crate::recommended`.
    pub recommended: &'a [Suggestion],
    /// Login to profile picture, for everyone the worker has looked up this
    /// session: live follows at every poll, offline ones once a session (see
    /// `twitch::unpictured`). A recommendation brings its own picture.
    pub avatars: &'a HashMap<String, String>,
    /// When each offline follow was last live, as far as Twitch has said:
    /// the line under an offline row's name. See `crate::last_live`.
    pub last_live: &'a LastLive,
    /// The channels open in a live pane.
    pub watching: &'a [String],
    /// Whether "open beside what is playing" is on offer; see
    /// `RootView::can_add`.
    pub can_add: bool,
    /// Whether a follows list has come back yet. Until one has, a pin cannot
    /// be told live from offline, nor followed from not.
    pub follows_loaded: bool,
    /// Whether the offline group is unfolded.
    pub offline_open: bool,
    /// Whether the live rows measure the pointer for their preview cards:
    /// only while the pointer is in the window and nothing is drawn over
    /// the rail. See `crate::rail_preview`.
    pub previews: bool,
    /// The row whose preview card is up, by its key ([`row_key`]), and the
    /// row the pointer is on, card or not: `rail_preview::Preview`'s, read
    /// once for the frame.
    pub preview_shown: Option<&'a str>,
    pub preview_pointed: Option<&'a str>,
}

/// One channel, as the follows lists know it, as the ask about channels
/// nobody follows found it, or as Twitch suggests it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row<'a> {
    /// Live: a follow, or a pin nobody follows that the ask about it found
    /// on (`root::pane_streams`).
    Live(&'a LiveStream),
    Offline(&'a Channel),
    /// A pin neither follows list has, and no ask has found live: the lists
    /// have not come back yet, or the channel is not followed and is off,
    /// or not asked about yet. Its login is all there is to show, dim, as an
    /// offline name is.
    Unknown(&'a str),
    /// A live channel nobody here follows, offered because it is like the
    /// ones watched; see `crate::recommended`.
    Recommended(&'a Suggestion),
}

impl Row<'_> {
    fn login(&self) -> &str {
        match self {
            Row::Live(stream) => &stream.user_login,
            Row::Offline(channel) => &channel.login,
            Row::Unknown(login) => login,
            Row::Recommended(suggestion) => &suggestion.channel.login,
        }
    }

    fn name(&self) -> &str {
        match self {
            Row::Live(stream) => &stream.display_name,
            Row::Offline(channel) => &channel.display_name,
            Row::Unknown(login) => login,
            Row::Recommended(suggestion) => &suggestion.channel.display_name,
        }
    }

    /// What a click on the row does: watch whoever is live, followed or
    /// suggested, and open the page of anyone else, which is what a click on
    /// an offline name does on Home.
    fn open(&self) -> Action {
        match self {
            Row::Live(stream) => Action::Watch(stream.user_login.clone()),
            Row::Recommended(suggestion) => Action::Watch(suggestion.channel.login.clone()),
            Row::Offline(channel) => Action::open_channel(channel),
            Row::Unknown(login) => Action::OpenChannel {
                login: login.to_string(),
                display_name: login.to_string(),
                user_id: None,
            },
        }
    }

    /// What a live row says under its name, in the colour it says it in, and
    /// how many are watching; `None` for a channel that is not live. A follow
    /// says what it is playing. A recommendation says why it is offered, in
    /// the accent, which is what tells it from a follow at a glance; what it
    /// is playing is on its preview card (`crate::rail_preview`).
    fn live(&self) -> Option<(&str, gpui::Hsla, u64)> {
        match self {
            Row::Live(stream) => Some((&stream.game_name, theme::text_dim(), stream.viewer_count)),
            Row::Recommended(suggestion) => Some((
                &suggestion.reason,
                theme::accent(),
                suggestion.channel.viewer_count,
            )),
            Row::Offline(_) | Row::Unknown(_) => None,
        }
    }

    /// Where the row's face comes from: the picture the worker looked up for
    /// a follow, at the rail's size, or the one a recommendation came with,
    /// which is that size already.
    fn avatar(&self, avatars: &HashMap<String, String>) -> Option<String> {
        match self {
            Row::Recommended(suggestion) => {
                Some(suggestion.channel.profile_image_url.clone()).filter(|url| !url.is_empty())
            }
            // A miss simply leaves the original, which is a larger picture
            // of the right person; see `twitch_api::profile_image_at`.
            _ => avatars
                .get(self.login())
                .map(|url| twitch_api::profile_image_at(url, AVATAR_REQUEST)),
        }
    }

    /// Whether the row offers the pin. A followed channel's does, and a
    /// pinned row's, which unpins. A recommendation's does not: pinned from
    /// there, a live row would land in Pinned as a bare login until the ask
    /// about it was answered (`root::pane_streams`). A channel nobody follows
    /// is pinned from its pane's More or its page instead, where it is
    /// already open (see `Action::SetPinned`).
    fn offers_pin(&self) -> bool {
        !matches!(self, Row::Recommended(_))
    }
}

/// The rail's four groups, each in the order it is drawn.
#[derive(Debug)]
pub struct Groups<'a> {
    pub pinned: Vec<Row<'a>>,
    pub live: Vec<&'a LiveStream>,
    pub recommended: Vec<&'a Suggestion>,
    pub offline: Vec<&'a Channel>,
    /// Whether a pin the follows list carries is live. Not every live pin:
    /// one nobody follows is live by the ask about it, and is nobody you
    /// follow.
    pub followed_pin_live: bool,
}

impl Groups<'_> {
    /// What the Live group says with nobody in it: not "nobody" when a pin
    /// just above is a live follow.
    pub fn nobody_live(&self) -> &'static str {
        if self.followed_pin_live {
            "Nobody else you follow is live."
        } else {
            "Nobody you follow is live."
        }
    }

    /// Whether the live row `key` names ([`row_key`]) is among these: a
    /// live pin, a live follow or a recommendation. What the root asks of
    /// the row its preview card follows, since only that row's probe can
    /// say the pointer left it, and a row that went (the stream ended at a
    /// poll, or the channel moved group) says nothing at all.
    pub fn draws_live_row(&self, key: &str) -> bool {
        self.pinned
            .iter()
            .any(|row| row.live().is_some() && row_key(Group::Pinned, row.login()) == key)
            || self
                .live
                .iter()
                .any(|stream| row_key(Group::Live, &stream.user_login) == key)
            || self
                .recommended
                .iter()
                .any(|suggestion| row_key(Group::Recommended, &suggestion.channel.login) == key)
    }
}

/// Sort the follows, and the recommendations, into the rail's groups.
///
/// A pinned channel is in Pinned and nowhere else, so it is one row and is
/// counted once; Live, Recommended and Offline leave it out — a pin on a
/// channel nobody follows can be recommended too. Pins are drawn in the
/// order `pinned` gives them, which is [`pin_order`]'s or the order held
/// under the pointer; each is what [`pin_row`] makes of it. Logins are
/// compared through `settings::channel_key`, the rule the pins were stored
/// by. The recommendations arrive ranked and already without anyone followed
/// or open (`RootView::rank_recommended`), and keep their order — and are
/// not shown at all until a follows list has come back, whatever the root
/// still holds: without one, nobody can say they are not followed.
pub fn groups<'a>(
    follows: &'a [LiveStream],
    offline: &'a [Channel],
    asked: &'a [LiveStream],
    recommended: &'a [Suggestion],
    pinned: &'a [String],
    follows_loaded: bool,
) -> Groups<'a> {
    let keys: Vec<String> = pinned.iter().map(|pin| channel_key(pin)).collect();
    let is_pinned = |login: &str| !keys.is_empty() && keys.contains(&channel_key(login));

    let rows = pinned
        .iter()
        .zip(&keys)
        .map(|(pin, key)| pin_row(pin, key, follows, offline, asked, follows_loaded))
        .collect();
    let followed_pin_live = follows_loaded
        && follows
            .iter()
            .any(|stream| keys.contains(&channel_key(&stream.user_login)));

    Groups {
        pinned: rows,
        followed_pin_live,
        live: follows
            .iter()
            .filter(|stream| !is_pinned(&stream.user_login))
            .collect(),
        recommended: recommended
            .iter()
            .filter(|suggestion| follows_loaded && !is_pinned(&suggestion.channel.login))
            .collect(),
        offline: offline
            .iter()
            .filter(|channel| !is_pinned(&channel.login))
            .collect(),
    }
}

/// One pin's row, `key` being its `settings::channel_key`: live or offline as
/// the follows lists have it; failing those, live when `asked`, what the ask
/// about channels nobody follows found on (`root::pane_streams`), has it;
/// and `Unknown` otherwise. Before any follows list has come back every pin
/// is `Unknown`, rather than read as offline from lists that are only empty
/// because nothing has arrived.
fn pin_row<'a>(
    pin: &'a str,
    key: &str,
    follows: &'a [LiveStream],
    offline: &'a [Channel],
    asked: &'a [LiveStream],
    follows_loaded: bool,
) -> Row<'a> {
    if !follows_loaded {
        return Row::Unknown(pin);
    }
    if let Some(stream) = follows
        .iter()
        .find(|stream| channel_key(&stream.user_login) == key)
    {
        Row::Live(stream)
    } else if let Some(channel) = offline
        .iter()
        .find(|channel| channel_key(&channel.login) == key)
    {
        Row::Offline(channel)
    } else if let Some(stream) = asked
        .iter()
        .find(|stream| channel_key(&stream.user_login) == key)
    {
        Row::Live(stream)
    } else {
        Row::Unknown(pin)
    }
}

/// Where a pin goes in Pinned, first to last: the order the variants are
/// in, then within each the order of what they hold.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum PinPlace {
    /// Live, the most watched first.
    Live(Reverse<u64>),
    /// Offline, the most recently live first.
    Off(Reverse<DateTime<Utc>>),
    /// Nothing to place it by: no follows list yet, a pin nobody follows
    /// that is not live, or an offline one Twitch has said nothing of yet,
    /// or that has never broadcast. These keep the order they were pinned
    /// in, the sort being stable.
    Unplaced,
}

/// The pins in the order Pinned draws them: whoever is live first, most
/// watched at the top, as the Live group goes; then whoever is off, most
/// recently live first (`LastLive::at`, the time the line under the name
/// counts from); then the rest in the order they were pinned. So the group
/// says the same as the two below it — who is on and how big, who was on
/// lately — for the channels that matter most.
///
/// Sorted afresh at every draw of the rail and held still while the pointer
/// is on a list of follows, as everything else on the rail is: see
/// `RootView::order_pins`. Each pin is the one `pinned` holds, so a held
/// order and a fresh one name the same strings.
pub fn pin_order(
    pinned: &[String],
    follows: &[LiveStream],
    offline: &[Channel],
    asked: &[LiveStream],
    last_live: &LastLive,
    follows_loaded: bool,
) -> Vec<String> {
    let mut placed: Vec<(PinPlace, &String)> = pinned
        .iter()
        .map(|pin| {
            let row = pin_row(
                pin,
                &channel_key(pin),
                follows,
                offline,
                asked,
                follows_loaded,
            );
            let place = match row {
                Row::Live(stream) => PinPlace::Live(Reverse(stream.viewer_count)),
                Row::Offline(channel) => last_live
                    .at(&channel.login)
                    .map_or(PinPlace::Unplaced, |at| PinPlace::Off(Reverse(at))),
                Row::Unknown(_) | Row::Recommended(_) => PinPlace::Unplaced,
            };
            (place, pin)
        })
        .collect();
    placed.sort_by(|a, b| a.0.cmp(&b.0));
    placed.into_iter().map(|(_, pin)| pin.clone()).collect()
}

/// Which group a row is drawn in. Part of its id, so the same channel can
/// move between groups without a stale hover following it — an id from the
/// row's position would hand one row's state to whichever row lands there.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Group {
    Pinned,
    Live,
    Recommended,
    Offline,
}

impl Group {
    fn row_id(self, login: &str) -> ElementId {
        ElementId::Name(row_key(self, login).into())
    }
}

/// A row's key: its element id's words, and what its preview card is kept
/// by (`rail_preview::Preview`), so a channel that moves to another group
/// starts its wait again rather than bringing a card along.
fn row_key(group: Group, login: &str) -> String {
    let group = match group {
        Group::Pinned => "pinned",
        Group::Live => "live",
        Group::Recommended => "recommended",
        Group::Offline => "offline",
    };
    format!("rail-{group}-{login}")
}

/// One row: avatar and name, and for a live channel what they are playing —
/// or, for a recommendation, why it is offered — and how many are watching.
/// Under the pointer a followed channel's row offers the pin — to pin it, or
/// on a pinned row, followed or not, to unpin it — and any live row, while
/// there is room beside what is playing, `+`.
///
/// An offline channel, or a pin nobody can place, is the name in `text_dim`,
/// with no dot and no count: there is nobody watching to count, and a dimmer
/// name is what tells it from the live rows around it at a glance. Under an
/// offline follow's name, where a live row says what is on, is when it was
/// last live ("Live 3 hours ago"), once Twitch has said; the row is the
/// avatar's height either way, which two lines of text exactly fill, so the
/// line costs the rail no room. Its face is the one the worker looked up
/// for it once this session (see `twitch::poll_follows`).
///
/// A live row measures the pointer for its preview card and tells
/// `on_point` when that changes, and draws the card while the rail says it
/// is up; see [`preview`].
#[allow(clippy::too_many_arguments)]
fn row<V: 'static>(
    group: Group,
    channel: Row<'_>,
    rail: &Rail<'_>,
    now: DateTime<Utc>,
    cache: &ImageCache,
    on_action: impl Fn(&mut V, Action, &mut Window, &mut Context<V>) + Clone + 'static,
    on_point: impl Fn(&mut V, String, bool, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let live = channel.live();
    let login = channel.login().to_string();
    let key = row_key(group, &login);
    let previews = rail.previews && live.is_some();
    let probe = previews.then(|| {
        let pointed = rail.preview_pointed == Some(key.as_str());
        preview_probe(key.clone(), pointed, on_point, cx)
    });
    let card = (previews && rail.preview_shown == Some(key.as_str()))
        .then(|| preview(channel, cache))
        .flatten();
    // The line under the name: a live row's game or reason, or an offline
    // follow's last time live, in the colour each says it in.
    let about: Option<(SharedString, gpui::Hsla)> = match (live, channel) {
        (Some((about, color, _)), _) => Some((SharedString::from(about.to_string()), color)),
        (None, Row::Offline(offline)) => rail
            .last_live
            .words(&offline.login, now)
            .map(|words| (SharedString::from(words), theme::text_dim())),
        (None, _) => None,
    };
    let watching = live.is_some() && rail.watching.contains(&login);
    let can_add = live.is_some() && rail.can_add;
    let offers_pin = channel.offers_pin();
    let pinned = group == Group::Pinned;
    let open = channel.open();
    let on_click = on_action.clone();
    let on_pin = on_action.clone();
    let on_add = on_action;
    let pin_login = login.clone();
    let add_login = login.clone();

    // Through the same cache as every other picture in the app, so a face
    // seen once is on disk for the next launch.
    let picture = channel
        .avatar(rail.avatars)
        .and_then(|url| cache.get_or_request(&url));

    let face = match picture {
        Some(path) => img(path)
            .w(px(AVATAR))
            .h(px(AVATAR))
            .rounded_full()
            .into_any_element(),
        // Sized placeholder, so a rail whose pictures have not arrived is the
        // same shape as one whose pictures have.
        None => div()
            .w(px(AVATAR))
            .h(px(AVATAR))
            .rounded_full()
            .bg(theme::surface_raised())
            .into_any_element(),
    };

    let name = div()
        .w_full()
        .text_ellipsis()
        .line_clamp(1)
        .text_size(px(theme::TEXT_LABEL))
        .font_weight(theme::weight_title())
        .line_height(px(theme::LINE_TIGHT))
        .text_color(if live.is_some() {
            theme::text()
        } else {
            theme::text_dim()
        })
        .child(SharedString::from(channel.name().to_string()));

    div()
        .id(group.row_id(&login))
        .group(ROW_GROUP)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::GAP_TIGHT))
        .px(px(theme::PANEL_PAD))
        .py(px(theme::GAP_TIGHT))
        .rounded(px(theme::RADIUS))
        .cursor_pointer()
        // A channel already open reads as chosen rather than as offered, which
        // is the same thing the browse page's tab pills say about themselves —
        // under the pointer too, as theirs does.
        .when(watching, |row| row.bg(theme::accent_dim()))
        .hover(move |style| {
            style.bg(if watching {
                theme::accent_dim()
            } else {
                theme::hover()
            })
        })
        .active(|style| style.bg(theme::pressed()))
        .on_click(
            cx.listener(move |view, _event, window, cx| on_click(view, open.clone(), window, cx)),
        )
        .child(div().flex_none().child(face))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(name)
                .when_some(about, |text, (about, color)| {
                    text.child(
                        div()
                            .w_full()
                            .text_ellipsis()
                            .line_clamp(1)
                            .text_size(px(theme::TEXT_META))
                            .line_height(px(theme::LINE_TIGHT))
                            .text_color(color)
                            .child(about),
                    )
                }),
        )
        // The count, and the dot that says the count is of people watching now.
        // Swapped for the row's own controls under the pointer: the row is
        // already a click, so the other things you might mean have to be
        // somewhere the first is not. Left standing on a row with no controls
        // to show — a recommendation with no room beside what is playing —
        // where hiding it would show nothing in its place.
        .when_some(live, |row, (_, _, viewers)| {
            row.child(
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(GAP_COUNT))
                    .when(offers_pin || can_add, |count| {
                        count.group_hover(ROW_GROUP, |style| style.invisible())
                    })
                    .child(controls::live_dot())
                    .child(
                        div()
                            .text_size(px(theme::TEXT_META))
                            .line_height(px(theme::LINE_TIGHT))
                            .text_color(theme::text_muted())
                            .child(SharedString::from(format_viewers(viewers))),
                    ),
            )
        })
        // Hidden rather than transparent until the row is pointed at: at zero
        // opacity a control still takes the click. Each stops the press, or
        // the row underneath fires too — replacing every open pane with this
        // one, for `+`.
        .child(
            div()
                .absolute()
                .right(px(theme::PANEL_PAD))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::GAP_WORD))
                .invisible()
                .group_hover(ROW_GROUP, |style| style.visible())
                .when(offers_pin, |buttons| {
                    buttons.child(
                        controls::icon_button(
                            // Keyed on the state its words follow, so the Pin a
                            // press has just made true does not stay up; see
                            // `controls::tip`.
                            ElementId::Name(
                                format!("rail-{}-{login}", if pinned { "unpin" } else { "pin" })
                                    .into(),
                            ),
                            Icon::Pin,
                            // Lit on a pinned row: it is the answer there, the
                            // way a chosen tab is, and pressing it takes the
                            // answer back.
                            if pinned {
                                controls::Variant::Selected
                            } else {
                                controls::Variant::Pill
                            },
                        )
                        .tooltip(controls::tip(if pinned { "Unpin" } else { "Pin" }))
                        .on_click(cx.listener(
                            move |view, _event, window, cx| {
                                cx.stop_propagation();
                                let pin = Action::SetPinned {
                                    login: pin_login.clone(),
                                    pinned: !pinned,
                                };
                                on_pin(view, pin, window, cx)
                            },
                        )),
                    )
                })
                .when(can_add, |buttons| {
                    buttons.child(
                        controls::pill(
                            ElementId::Name(format!("rail-add-{login}").into()),
                            "+",
                            controls::Variant::Pill,
                        )
                        .tooltip(controls::tip("Open beside what is playing"))
                        .on_click(cx.listener(
                            move |view, _event, window, cx| {
                                cx.stop_propagation();
                                on_add(view, Action::Add(add_login.clone()), window, cx)
                            },
                        )),
                    )
                }),
        )
        .children(probe)
        // Hung off the row's right-hand edge, level with its top, and
        // drawn deferred so the rail's scroller does not clip it; snapped
        // inside the window, so a row near the foot of the rail lifts its
        // card rather than losing the end of it. The snap only ever lifts
        // it: the root shows cards only in a window with room for one
        // beside the rail and under the title bar ([`previews_fit`]), and
        // a row the rail's top cuts off shows none (`preview_probe`), so
        // the card never slides back over the row or up over the bar.
        .when_some(card, |row, card| {
            row.child(
                div().absolute().top_0().left(relative(1.)).child(deferred(
                    anchored()
                        .offset(point(px(PREVIEW_OFFSET), px(0.)))
                        .snap_to_window_with_margin(px(theme::GAP))
                        .child(card),
                )),
            )
        })
}

/// What a live row's card shows: a follow's stream as the follows poll has
/// it, or a recommendation drawn as the guide draws one, with the reason it
/// is offered after its game. Nothing for a row that is not live.
fn preview(channel: Row<'_>, cache: &ImageCache) -> Option<gpui::AnyElement> {
    match channel {
        Row::Live(stream) => Some(rail_preview::card(stream, None, cache)),
        Row::Recommended(suggestion) => Some(rail_preview::card(
            &guide::as_stream(&suggestion.channel),
            Some(&suggestion.reason),
            cache,
        )),
        Row::Offline(_) | Row::Unknown(_) => None,
    }
}

/// The probe a live row measures the pointer with for its preview card: a
/// `canvas`, which inserts no hitbox, so it takes nothing from the row or
/// its controls, reading the pointer against the part of the row the
/// rail's scroller shows — a row half scrolled out from under the rail's
/// top does not count the pointer above the rail as its own — while the
/// pointer is in the window. It tells `on_point` only when the answer
/// differs from what the root held when the rail was drawn (`pointed`), so
/// a rail of rows nobody is pointing at says nothing at all.
///
/// A row the rail's top cuts off counts as not pointed at: its card hangs
/// from the row's top, which is then up under the title bar, and the card
/// would be drawn over the bar's arrows and search box.
///
/// A report asks for the next frame itself (`request_animation_frame`):
/// it comes from prepaint, where a `cx.notify()` marks the view dirty but
/// asks for no frame (HANDOFF.md, "A notify from a probe asks for no
/// frame"), and this frame's card was settled in `render` from what the
/// root held before. Without it a card stayed up over the page after the
/// pointer left its row, until something else happened to draw.
fn preview_probe<V: 'static>(
    key: String,
    pointed: bool,
    on_point: impl Fn(&mut V, String, bool, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let owner = cx.entity().downgrade();
    canvas(
        move |bounds, window, cx| {
            let clip = window.content_mask().bounds;
            let shown = bounds.intersect(&clip);
            let now = window.is_window_hovered()
                && bounds.top() >= clip.top()
                && shown.contains(&window.mouse_position());
            if now != pointed {
                window.request_animation_frame();
                owner
                    .update(cx, |view, cx| on_point(view, key, now, cx))
                    .ok();
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full()
}

/// A group's name over its rows. `first` is the top of the list, which needs
/// no gap above it to set it apart from the group before.
fn heading(label: &'static str, first: bool) -> impl IntoElement {
    controls::group_heading()
        .when(!first, |heading| heading.mt(px(theme::GAP)))
        .child(label)
}

/// The rail: Pinned, when anything is; Live; Recommended, when anything is;
/// then Offline, folded under its count until `on_fold` opens it. The root
/// decides whether it is drawn at all; see `RootView::follows_rail`.
pub fn rail<V: 'static>(
    rail: Rail<'_>,
    cache: &Arc<ImageCache>,
    scroll: &ScrollHandle,
    on_action: impl Fn(&mut V, Action, &mut Window, &mut Context<V>) + Clone + 'static,
    on_point: impl Fn(&mut V, String, bool, &mut Context<V>) + Clone + 'static,
    on_fold: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let groups = groups(
        rail.follows,
        rail.offline,
        rail.asked,
        rail.recommended,
        rail.pinned,
        rail.follows_loaded,
    );
    let now = Utc::now();

    // Its own scroller. The rail is as tall as the window and a hundred live
    // follows is longer than that, and it must not scroll the page behind it.
    let mut list = div()
        .id("sidebar-list")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(scroll)
        .flex()
        .flex_col()
        .px(px(theme::GAP_TIGHT))
        .pb(px(theme::GAP_TIGHT));

    let pinned = !groups.pinned.is_empty();
    if pinned {
        list = list.child(heading("Pinned", true));
        for channel in &groups.pinned {
            list = list.child(row(
                Group::Pinned,
                *channel,
                &rail,
                now,
                cache,
                on_action.clone(),
                on_point.clone(),
                cx,
            ));
        }
    }

    list = list.child(heading("Live", !pinned));
    for stream in &groups.live {
        list = list.child(row(
            Group::Live,
            Row::Live(stream),
            &rail,
            now,
            cache,
            on_action.clone(),
            on_point.clone(),
            cx,
        ));
    }
    if groups.live.is_empty() {
        list = list.child(
            div()
                .px(px(theme::PANEL_PAD))
                .py(px(theme::GAP))
                .text_size(px(theme::TEXT_META))
                .line_height(px(theme::LINE_BODY))
                .text_color(theme::text_dim())
                .child(groups.nobody_live()),
        );
    }

    // Under who is live and over the fold, so a channel you do not follow
    // never sits above one you do. A plain heading like the two above it.
    // No heading at all with nothing under it: before the follows arrive,
    // signed out, an empty answer, or Twitch turning the query down.
    if !groups.recommended.is_empty() {
        list = list.child(heading("Recommended", false));
        for suggestion in &groups.recommended {
            list = list.child(row(
                Group::Recommended,
                Row::Recommended(suggestion),
                &rail,
                now,
                cache,
                on_action.clone(),
                on_point.clone(),
                cx,
            ));
        }
    }

    // No heading at all with nobody to fold: signed out, or still loading.
    if !groups.offline.is_empty() {
        list = list.child(
            controls::fold(
                "rail-offline-fold",
                format!("Offline ({})", groups.offline.len()),
                rail.offline_open,
            )
            .mt(px(theme::GAP))
            .on_click(cx.listener(move |view, _event, window, cx| on_fold(view, window, cx))),
        );
        if rail.offline_open {
            for channel in &groups.offline {
                list = list.child(row(
                    Group::Offline,
                    Row::Offline(channel),
                    &rail,
                    now,
                    cache,
                    on_action.clone(),
                    on_point.clone(),
                    cx,
                ));
            }
        }
    }

    div()
        .flex_none()
        .w(px(WIDTH))
        .h_full()
        .flex()
        .flex_col()
        .bg(theme::surface())
        .border_r_1()
        .border_color(theme::border())
        .child(
            div()
                .flex_1()
                .min_h_0()
                .relative()
                .flex()
                .flex_col()
                .child(list)
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .child(Scrollbar::vertical(scroll).scrollbar_show(ScrollbarShow::Hover)),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(login: &str, viewers: u64) -> LiveStream {
        LiveStream {
            id: String::new(),
            user_login: login.into(),
            user_id: String::new(),
            display_name: login.into(),
            title: String::new(),
            game_name: String::new(),
            viewer_count: viewers,
            thumbnail_url: String::new(),
            started_at: String::new(),
        }
    }

    fn channel(login: &str) -> Channel {
        Channel {
            login: login.into(),
            user_id: String::new(),
            display_name: login.into(),
        }
    }

    fn pins(logins: &[&str]) -> Vec<String> {
        logins.iter().map(|login| login.to_string()).collect()
    }

    fn logins<'a>(rows: &[Row<'a>]) -> Vec<&'a str> {
        rows.iter()
            .map(|row| match row {
                Row::Live(stream) => stream.user_login.as_str(),
                Row::Offline(channel) => channel.login.as_str(),
                Row::Unknown(login) => *login,
                Row::Recommended(suggestion) => suggestion.channel.login.as_str(),
            })
            .collect()
    }

    fn suggestion(login: &str, viewers: u64) -> Suggestion {
        Suggestion {
            channel: twitch_api::recommend::SimilarChannel {
                login: login.into(),
                user_id: String::new(),
                display_name: login.to_uppercase(),
                title: String::new(),
                game_name: "Dota 2".into(),
                game_id: String::new(),
                viewer_count: viewers,
                profile_image_url: format!("https://example.invalid/{login}-70x70.png"),
                stream_id: String::new(),
            },
            reason: "Like forsen".into(),
        }
    }

    /// With recommendations, they are a group of their own, in the order
    /// they came ranked, and nobody moves out of Live or Offline for them;
    /// without any, the group is empty, and the rail draws no heading for it.
    #[test]
    fn recommendations_are_a_group_of_their_own_when_there_are_any() {
        let follows = [stream("alice", 300)];
        let offline = [channel("carol")];
        let recommended = [suggestion("small", 5), suggestion("big", 9000)];

        let with = groups(&follows, &offline, &[], &recommended, &[], true);
        assert_eq!(with.recommended, [&recommended[0], &recommended[1]]);
        assert_eq!(with.live, [&follows[0]]);
        assert_eq!(with.offline, [&offline[0]]);
        assert!(with.pinned.is_empty());

        let without = groups(&follows, &offline, &[], &[], &[], true);
        assert!(without.recommended.is_empty());
        assert_eq!(without.live, [&follows[0]]);

        // Before the follows have come back — a new client id's worker still
        // signing in — nothing is recommended, whatever was held from before.
        let unloaded = groups(&[], &[], &[], &recommended, &[], false);
        assert!(unloaded.recommended.is_empty());
    }

    /// A pin set by hand on a channel nobody follows is in Pinned, and is not
    /// recommended beside it as well — whatever case it was written in.
    #[test]
    fn a_pinned_channel_is_not_recommended_again() {
        let recommended = [suggestion("handpinned", 50), suggestion("other", 40)];
        let pinned = pins(&["HandPinned"]);

        let groups = groups(&[], &[], &[], &recommended, &pinned, true);
        assert_eq!(groups.pinned, [Row::Unknown("HandPinned")]);
        assert_eq!(groups.recommended, [&recommended[1]]);
    }

    /// A recommendation's row watches the channel on a click, as a live
    /// follow's does; says why it is offered, in the accent, with the
    /// viewers; brings its own face; and offers no pin.
    #[test]
    fn a_recommended_row_is_a_live_row_without_a_pin() {
        let suggested = suggestion("maxylobes", 1234);
        let row = Row::Recommended(&suggested);
        assert!(matches!(row.open(), Action::Watch(login) if login == "maxylobes"));
        assert_eq!(row.name(), "MAXYLOBES");
        assert_eq!(row.live(), Some(("Like forsen", theme::accent(), 1234)));
        assert!(!row.offers_pin());
        assert_eq!(
            row.avatar(&HashMap::new()).as_deref(),
            Some("https://example.invalid/maxylobes-70x70.png")
        );

        let follow = stream("alice", 300);
        let live = Row::Live(&follow);
        assert!(live.offers_pin());
        assert!(matches!(live.open(), Action::Watch(login) if login == "alice"));
        let avatars = HashMap::from([(
            "alice".to_string(),
            "https://example.invalid/alice-300x300.png".to_string(),
        )]);
        assert_eq!(
            live.avatar(&avatars).as_deref(),
            Some("https://example.invalid/alice-70x70.png")
        );
    }

    /// The reason is the accent on the rail's own surface, and has to read
    /// there at rest and under the pointer's wash. Under the press, for the
    /// instant a click lasts, nothing on a rail row reaches AA — a live
    /// follow's game line, in `text_dim`, measures about 4.0:1 — so there the
    /// reason is held to reading no worse than the line it stands in for.
    #[test]
    fn the_reason_reads_on_the_rail() {
        let color = |(_, color, _): (&str, gpui::Hsla, u64)| color;
        let suggested = suggestion("someone", 1);
        let follow = stream("alice", 1);
        let reason = Row::Recommended(&suggested).live().map(color).unwrap();
        let game = Row::Live(&follow).live().map(color).unwrap();

        for (state, under) in [
            ("at rest", theme::surface()),
            ("hovered", theme::surface().blend(theme::hover())),
        ] {
            let ratio = theme::contrast(reason, under);
            assert!(
                ratio >= theme::MIN_CONTRAST,
                "a recommendation's reason reads {ratio:.2}:1 {state}"
            );
        }
        let pressed = theme::surface().blend(theme::pressed());
        assert!(
            theme::contrast(reason, pressed) >= theme::contrast(game, pressed),
            "under the press a recommendation's reason reads worse than a follow's game"
        );
    }

    /// A pinned channel is one row, in Pinned, and not again in the group it
    /// would otherwise be in — live or offline.
    #[test]
    fn a_pinned_channel_appears_once() {
        let follows = [stream("alice", 300), stream("bob", 200)];
        let offline = [channel("carol"), channel("dave")];
        let pinned = pins(&["bob", "dave"]);

        let groups = groups(&follows, &offline, &[], &[], &pinned, true);
        assert_eq!(
            groups.pinned,
            [Row::Live(&follows[1]), Row::Offline(&offline[1])]
        );
        assert_eq!(groups.live, [&follows[0]]);
        assert_eq!(groups.offline, [&offline[0]]);
    }

    /// Whoever is live first, by viewers, a pin nobody follows among them;
    /// then whoever is off, the most recently live first; then the pins
    /// with nothing to go by, in the order they were pinned.
    #[test]
    fn pins_go_live_by_viewers_then_off_by_when_last_live() {
        let follows = [stream("big", 9000), stream("small", 5)];
        let asked = [stream("faraway", 300)];
        let offline = [channel("lastweek"), channel("never"), channel("yesterday")];
        let at = |when: &str| {
            DateTime::parse_from_rfc3339(when)
                .unwrap()
                .with_timezone(&Utc)
        };
        let mut last_live = LastLive::default();
        last_live.went_live(["lastweek"], at("2026-09-01T12:00:00Z"));
        last_live.went_live(["yesterday"], at("2026-09-07T12:00:00Z"));
        let pinned = pins(&[
            "gone",
            "lastweek",
            "small",
            "never",
            "yesterday",
            "big",
            "faraway",
        ]);

        let order = pin_order(&pinned, &follows, &offline, &asked, &last_live, true);
        assert_eq!(
            order,
            [
                "big",
                "faraway",
                "small",
                "yesterday",
                "lastweek",
                "gone",
                "never"
            ]
        );
        // And that is the order Pinned draws them in.
        let groups = groups(&follows, &offline, &asked, &[], &order, true);
        assert_eq!(logins(&groups.pinned), order);
        assert!(groups.live.is_empty());
    }

    /// Before any follows list nobody can be placed, so the pins are in the
    /// order they were pinned, whatever the ask about channels nobody
    /// follows has found.
    #[test]
    fn pins_keep_pin_order_until_the_follows_arrive() {
        let asked = [stream("faraway", 300)];
        let pinned = pins(&["alice", "faraway"]);
        let order = pin_order(&pinned, &[], &[], &asked, &LastLive::default(), false);
        assert_eq!(order, pinned);
    }

    /// Before any list has come back, empty lists say nothing about who is
    /// offline. Every pin waits as `Unknown`; so does a pin that neither list
    /// has once they have come back, and that no ask has found live.
    #[test]
    fn pins_are_unknown_until_the_follows_arrive() {
        let pinned = pins(&["alice", "gone"]);
        let before = groups(&[], &[], &[], &[], &pinned, false);
        assert_eq!(before.pinned, [Row::Unknown("alice"), Row::Unknown("gone")]);

        let follows = [stream("alice", 10)];
        let after = groups(&follows, &[], &[], &[], &pinned, true);
        assert_eq!(after.pinned, [Row::Live(&follows[0]), Row::Unknown("gone")]);
    }

    /// A pin nobody follows is live, with what Helix said, once the ask
    /// about it finds it on — whatever case it was pinned in — and a live
    /// row a card can follow; the follows' own word comes first for a
    /// channel they have. An empty Live group does not count it as somebody
    /// else you follow. Before a follows list has come back it waits with
    /// the rest.
    #[test]
    fn a_pin_nobody_follows_is_live_when_the_ask_says_so() {
        let follows = [stream("alice", 300)];
        let asked = [stream("faraway", 42), stream("alice", 1)];
        let pinned = pins(&["Faraway", "alice", "asleep"]);

        let groups_now = groups(&follows, &[], &asked, &[], &pinned, true);
        assert_eq!(
            groups_now.pinned,
            [
                Row::Live(&asked[0]),
                Row::Live(&follows[0]),
                Row::Unknown("asleep")
            ]
        );
        assert!(groups_now.draws_live_row(&row_key(Group::Pinned, "faraway")));
        assert!(groups_now.live.is_empty(), "alice is pinned");
        assert_eq!(groups_now.nobody_live(), "Nobody else you follow is live.");

        // Live by the ask alone, faraway is nobody you follow.
        let faraway = pins(&["faraway"]);
        let unfollowed = groups(&[], &[], &asked, &[], &faraway, true);
        assert_eq!(unfollowed.pinned, [Row::Live(&asked[0])]);
        assert_eq!(unfollowed.nobody_live(), "Nobody you follow is live.");

        let before = groups(&[], &[], &asked, &[], &pinned, false);
        assert!(before
            .pinned
            .iter()
            .all(|row| matches!(row, Row::Unknown(_))));
    }

    /// What a press of a row's pin does to the rail: pinned through the
    /// settings, a live follow leaves Live for Pinned and is one row; unpinned,
    /// it goes back where it was.
    #[test]
    fn pinning_from_a_row_moves_it_from_live_to_pinned() {
        let follows = [stream("alice", 300), stream("bob", 200)];
        let offline = [channel("carol")];
        let mut settings = settings::Settings::default();

        assert!(settings.set_pinned("Bob", true));
        let pinned = groups(&follows, &offline, &[], &[], &settings.pinned, true);
        assert_eq!(pinned.pinned, [Row::Live(&follows[1])]);
        assert_eq!(pinned.live, [&follows[0]]);
        let rows = pinned.pinned.len() + pinned.live.len() + pinned.offline.len();
        assert_eq!(rows, 3, "a pinned channel was counted twice");

        assert!(settings.set_pinned("bob", false));
        let unpinned = groups(&follows, &offline, &[], &[], &settings.pinned, true);
        assert!(unpinned.pinned.is_empty());
        assert_eq!(unpinned.live, [&follows[0], &follows[1]]);
    }

    /// The count on the fold is of the rows under it, which leaves out the
    /// pins — whatever case they were written in.
    #[test]
    fn the_offline_count_leaves_out_pins() {
        let offline = [channel("alice"), channel("bob"), channel("carol")];
        let pinned = pins(&["Bob"]);

        let groups = groups(&[], &offline, &[], &[], &pinned, true);
        assert_eq!(groups.offline.len(), 2);
        assert_eq!(groups.offline, [&offline[0], &offline[2]]);
        assert_eq!(groups.pinned, [Row::Offline(&offline[1])]);
    }

    /// The live rows a preview card can follow, by the key it keeps them
    /// by: a live pin, a live follow, a recommendation. An offline pin is
    /// not one, a row keeps its group in its key, and a stream that ended
    /// at a poll is no longer drawn, so the root forgets the row it was.
    #[test]
    fn a_card_follows_only_live_rows_still_drawn() {
        let follows = [stream("alice", 300), stream("bob", 20)];
        let offline = [channel("carol")];
        let recommended = [suggestion("dave", 5)];
        let pinned = pins(&["bob", "carol"]);

        let drawn = groups(&follows, &offline, &[], &recommended, &pinned, true);
        assert!(drawn.draws_live_row(&row_key(Group::Live, "alice")));
        assert!(drawn.draws_live_row(&row_key(Group::Pinned, "bob")));
        assert!(drawn.draws_live_row(&row_key(Group::Recommended, "dave")));
        assert!(!drawn.draws_live_row(&row_key(Group::Live, "bob")));
        assert!(!drawn.draws_live_row(&row_key(Group::Pinned, "carol")));
        assert!(!drawn.draws_live_row(&row_key(Group::Offline, "carol")));

        // Alice's stream ends at the next poll, with the pointer on her row.
        let ended = groups(&follows[1..], &offline, &[], &recommended, &pinned, true);
        assert!(!ended.draws_live_row(&row_key(Group::Live, "alice")));
    }

    /// A card shows only where the snap that keeps it in the window would
    /// not move it back over the rail or up over the title bar.
    #[test]
    fn previews_need_room_beside_the_rail_and_under_the_bar() {
        let wide = WIDTH + PREVIEW_OFFSET + rail_preview::PREVIEW_WIDTH + theme::GAP;
        let tall = theme::TITLE_BAR_HEIGHT + rail_preview::PREVIEW_TALLEST + 2.0 * theme::GAP;
        assert!(previews_fit(1600.0, 900.0, theme::TITLE_BAR_HEIGHT));
        assert!(previews_fit(wide, tall, theme::TITLE_BAR_HEIGHT));
        assert!(!previews_fit(wide - 1.0, 900.0, theme::TITLE_BAR_HEIGHT));
        assert!(!previews_fit(1600.0, tall - 1.0, theme::TITLE_BAR_HEIGHT));
        // Fullscreen has no bar to keep clear of.
        assert!(previews_fit(1600.0, tall - 1.0, 0.0));
    }
}
