//! The follows rail.
//!
//! The channels you follow, in three groups, down the left-hand edge, beside
//! both pages. It exists because the two halves of this app were further
//! apart than the work is: picking a channel meant leaving the one you were
//! watching, going to a grid of thumbnails, and coming back. Most of the time
//! the question is not "what is on" — it is "who else is live", which is a
//! list of names and numbers and fits in a column.
//!
//! Three groups, top to bottom (see [`groups`]). The channels pinned there,
//! in the order they were pinned, live or not — see
//! [`settings::Settings::pinned`]. Who else is live, by viewers. Then everyone
//! else followed, as names, folded under a count, because that is the longest
//! list in the app and most evenings nobody wants it. Folded is where it
//! starts each session, and folded it builds no rows and asks for no
//! pictures. The rows all hold still under the pointer; see
//! `RootView::hold_live`. A pin is made and taken off from the row itself,
//! under the pointer, and saved by `RootView::set_pinned`.
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

use std::collections::HashMap;
use std::sync::Arc;

use emotes::ImageCache;
use gpui::{div, img, prelude::*, px, Context, ElementId, ScrollHandle, SharedString, Window};
use gpui_component::scroll::{Scrollbar, ScrollbarShow};
use settings::channel_key;
use twitch_api::{Channel, LiveStream};

use crate::assets::Icon;
use crate::browse::{format_viewers, Action};
use crate::controls;
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
const AVATAR_REQUEST: &str = "70x70";

/// The group a row's hover-revealed controls watch for the pointer.
const ROW_GROUP: &str = "rail-row";

/// Between the live dot and the number it belongs to. Tighter than
/// [`theme::GAP_TIGHT`], because they are one thing rather than two.
const GAP_COUNT: f32 = 4.0;

/// Everything the rail shows, as the root holds it.
pub struct Rail<'a> {
    /// Who is live: by viewers, or held still under the pointer.
    pub follows: &'a [LiveStream],
    /// Everyone else followed: by name, or held still the same way.
    pub offline: &'a [Channel],
    /// What is pinned, in pin order; see [`settings::Settings::pinned`].
    pub pinned: &'a [String],
    /// Login to profile picture, for everyone the worker has looked up this
    /// session. Only live follows are looked up; an offline row has a face
    /// only if its channel was live earlier and nobody has restarted since.
    pub avatars: &'a HashMap<String, String>,
    /// The channels open in a live pane.
    pub watching: &'a [String],
    /// Whether "open beside what is playing" is on offer; see
    /// `RootView::can_add`.
    pub can_add: bool,
    /// Whether a follows list has come back yet. Until one has, a pin cannot
    /// be told live from offline.
    pub follows_loaded: bool,
    /// Whether the offline group is unfolded.
    pub offline_open: bool,
}

/// One channel, as the follows lists know it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row<'a> {
    Live(&'a LiveStream),
    Offline(&'a Channel),
    /// A pin neither list has: they have not come back yet, or the channel
    /// is no longer followed. Its login is all there is to show.
    Unknown(&'a str),
}

impl Row<'_> {
    fn login(&self) -> &str {
        match self {
            Row::Live(stream) => &stream.user_login,
            Row::Offline(channel) => &channel.login,
            Row::Unknown(login) => login,
        }
    }

    fn name(&self) -> &str {
        match self {
            Row::Live(stream) => &stream.display_name,
            Row::Offline(channel) => &channel.display_name,
            Row::Unknown(login) => login,
        }
    }

    /// What a click on the row does: watch who is live, and open the page of
    /// anyone else, which is what a click on an offline name does on the
    /// Following tab.
    fn open(&self) -> Action {
        match self {
            Row::Live(stream) => Action::Watch(stream.user_login.clone()),
            Row::Offline(channel) => Action::open_channel(channel),
            Row::Unknown(login) => Action::OpenChannel {
                login: login.to_string(),
                display_name: login.to_string(),
                user_id: None,
            },
        }
    }
}

/// The rail's three groups, each in the order it is drawn.
#[derive(Debug)]
pub struct Groups<'a> {
    pub pinned: Vec<Row<'a>>,
    pub live: Vec<&'a LiveStream>,
    pub offline: Vec<&'a Channel>,
}

/// Sort the follows into the rail's groups.
///
/// A pinned channel is in Pinned and nowhere else, so it is one row and is
/// counted once; Live and Offline leave it out. Pins keep pin order whoever
/// is live — a group that reshuffled by viewers would be the live list again.
/// Before any follows list has come back every pin is `Unknown`, rather than
/// read as offline from lists that are only empty because nothing has
/// arrived. Logins are compared through `settings::channel_key`, the rule the
/// pins were stored by.
pub fn groups<'a>(
    follows: &'a [LiveStream],
    offline: &'a [Channel],
    pinned: &'a [String],
    follows_loaded: bool,
) -> Groups<'a> {
    let keys: Vec<String> = pinned.iter().map(|pin| channel_key(pin)).collect();
    let is_pinned = |login: &str| !keys.is_empty() && keys.contains(&channel_key(login));

    let rows = pinned
        .iter()
        .zip(&keys)
        .map(|(pin, key)| {
            if !follows_loaded {
                return Row::Unknown(pin);
            }
            if let Some(stream) = follows
                .iter()
                .find(|stream| channel_key(&stream.user_login) == *key)
            {
                Row::Live(stream)
            } else if let Some(channel) = offline
                .iter()
                .find(|channel| channel_key(&channel.login) == *key)
            {
                Row::Offline(channel)
            } else {
                Row::Unknown(pin)
            }
        })
        .collect();

    Groups {
        pinned: rows,
        live: follows
            .iter()
            .filter(|stream| !is_pinned(&stream.user_login))
            .collect(),
        offline: offline
            .iter()
            .filter(|channel| !is_pinned(&channel.login))
            .collect(),
    }
}

/// Which group a row is drawn in. Part of its id, so the same channel can
/// move between groups without a stale hover following it — an id from the
/// row's position would hand one row's state to whichever row lands there.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Group {
    Pinned,
    Live,
    Offline,
}

impl Group {
    fn row_id(self, login: &str) -> ElementId {
        let group = match self {
            Group::Pinned => "pinned",
            Group::Live => "live",
            Group::Offline => "offline",
        };
        ElementId::Name(format!("rail-{group}-{login}").into())
    }
}

/// One row: avatar and name, and for a live channel what they are playing
/// and how many are watching. Under the pointer it offers the pin — to pin
/// it, or on a pinned row to unpin it — and, for a live channel while there
/// is room beside what is playing, `+`.
///
/// An offline channel, or a pin nobody can place, is the name alone in
/// `text_dim`, with no dot and no count: there is nobody watching to count,
/// and a dimmer name is what tells it from the live rows around it at a
/// glance. Its face is whatever this session already has for it; the rail
/// asks Twitch for no pictures of channels that are not live.
fn row<V: 'static>(
    group: Group,
    channel: Row<'_>,
    rail: &Rail<'_>,
    cache: &ImageCache,
    on_action: impl Fn(&mut V, Action, &mut Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let live = match channel {
        Row::Live(stream) => Some(stream),
        Row::Offline(_) | Row::Unknown(_) => None,
    };
    let login = channel.login().to_string();
    let watching = live.is_some() && rail.watching.contains(&login);
    let can_add = live.is_some() && rail.can_add;
    let pinned = group == Group::Pinned;
    let open = channel.open();
    let on_click = on_action.clone();
    let on_pin = on_action.clone();
    let on_add = on_action;
    let pin_login = login.clone();
    let add_login = login.clone();

    // Twitch's URLs carry the size in the filename rather than as a parameter,
    // so this is a substitution rather than a query. A miss simply leaves the
    // original, which is a larger picture of the right person.
    let picture = rail
        .avatars
        .get(&login)
        .map(|url| url.replace("300x300", AVATAR_REQUEST))
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
                .when_some(live, |text, stream| {
                    text.child(
                        div()
                            .w_full()
                            .text_ellipsis()
                            .line_clamp(1)
                            .text_size(px(theme::TEXT_META))
                            .line_height(px(theme::LINE_TIGHT))
                            .text_color(theme::text_dim())
                            .child(SharedString::from(stream.game_name.clone())),
                    )
                }),
        )
        // The count, and the dot that says the count is of people watching now.
        // Swapped for the row's own controls under the pointer: the row is
        // already a click, so the other things you might mean have to be
        // somewhere the first is not.
        .when_some(live, |row, stream| {
            row.child(
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(GAP_COUNT))
                    .group_hover(ROW_GROUP, |style| style.invisible())
                    .child(controls::live_dot())
                    .child(
                        div()
                            .text_size(px(theme::TEXT_META))
                            .line_height(px(theme::LINE_TIGHT))
                            .text_color(theme::text_muted())
                            .child(SharedString::from(format_viewers(stream.viewer_count))),
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
                .child(
                    controls::icon_button(
                        // Keyed on the state its words follow, so the Pin a
                        // press has just made true does not stay up; see
                        // `controls::tip`.
                        ElementId::Name(
                            format!("rail-{}-{login}", if pinned { "unpin" } else { "pin" }).into(),
                        ),
                        Icon::Pin,
                        // Lit on a pinned row: it is the answer there, the way
                        // a chosen tab is, and pressing it takes the answer back.
                        if pinned {
                            controls::Variant::Selected
                        } else {
                            controls::Variant::Pill
                        },
                    )
                    .tooltip(controls::tip(if pinned { "Unpin" } else { "Pin" }))
                    .on_click(cx.listener(move |view, _event, window, cx| {
                        cx.stop_propagation();
                        let pin = Action::SetPinned {
                            login: pin_login.clone(),
                            pinned: !pinned,
                        };
                        on_pin(view, pin, window, cx)
                    })),
                )
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
}

/// A group's name over its rows. `first` is the top of the list, which needs
/// no gap above it to set it apart from the group before.
fn heading(label: &'static str, first: bool) -> impl IntoElement {
    controls::group_heading()
        .when(!first, |heading| heading.mt(px(theme::GAP)))
        .child(label)
}

/// The rail: Pinned, when anything is; Live; then Offline, folded under its
/// count until `on_fold` opens it. The root decides whether it is drawn at
/// all; see `RootView::follows_rail`.
pub fn rail<V: 'static>(
    rail: Rail<'_>,
    cache: &Arc<ImageCache>,
    scroll: &ScrollHandle,
    on_action: impl Fn(&mut V, Action, &mut Window, &mut Context<V>) + Clone + 'static,
    on_fold: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let groups = groups(rail.follows, rail.offline, rail.pinned, rail.follows_loaded);

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
                cache,
                on_action.clone(),
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
            cache,
            on_action.clone(),
            cx,
        ));
    }
    if groups.live.is_empty() {
        // Not "nobody" when a pin just above is live.
        let pinned_live = groups
            .pinned
            .iter()
            .any(|channel| matches!(channel, Row::Live(_)));
        list = list.child(
            div()
                .px(px(theme::PANEL_PAD))
                .py(px(theme::GAP))
                .text_size(px(theme::TEXT_META))
                .line_height(px(theme::LINE_BODY))
                .text_color(theme::text_dim())
                .child(if pinned_live {
                    "Nobody else you follow is live."
                } else {
                    "Nobody you follow is live."
                }),
        );
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
                    cache,
                    on_action.clone(),
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
            })
            .collect()
    }

    /// A pinned channel is one row, in Pinned, and not again in the group it
    /// would otherwise be in — live or offline.
    #[test]
    fn a_pinned_channel_appears_once() {
        let follows = [stream("alice", 300), stream("bob", 200)];
        let offline = [channel("carol"), channel("dave")];
        let pinned = pins(&["bob", "dave"]);

        let groups = groups(&follows, &offline, &pinned, true);
        assert_eq!(
            groups.pinned,
            [Row::Live(&follows[1]), Row::Offline(&offline[1])]
        );
        assert_eq!(groups.live, [&follows[0]]);
        assert_eq!(groups.offline, [&offline[0]]);
    }

    /// Pin order, not viewers: the busiest pin does not climb over the one
    /// pinned first, and a pin that is offline keeps its place among them.
    #[test]
    fn pins_keep_pin_order_whoever_is_live() {
        let follows = [stream("big", 9000), stream("small", 5)];
        let offline = [channel("asleep")];
        let pinned = pins(&["small", "asleep", "big"]);

        let groups = groups(&follows, &offline, &pinned, true);
        assert_eq!(logins(&groups.pinned), ["small", "asleep", "big"]);
        assert!(matches!(groups.pinned[1], Row::Offline(_)));
        assert!(groups.live.is_empty());
    }

    /// Before any list has come back, empty lists say nothing about who is
    /// offline. Every pin waits as `Unknown`; so does a pin that neither list
    /// has once they have come back.
    #[test]
    fn pins_are_unknown_until_the_follows_arrive() {
        let pinned = pins(&["alice", "gone"]);
        let before = groups(&[], &[], &pinned, false);
        assert_eq!(before.pinned, [Row::Unknown("alice"), Row::Unknown("gone")]);

        let follows = [stream("alice", 10)];
        let after = groups(&follows, &[], &pinned, true);
        assert_eq!(after.pinned, [Row::Live(&follows[0]), Row::Unknown("gone")]);
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
        let pinned = groups(&follows, &offline, &settings.pinned, true);
        assert_eq!(pinned.pinned, [Row::Live(&follows[1])]);
        assert_eq!(pinned.live, [&follows[0]]);
        let rows = pinned.pinned.len() + pinned.live.len() + pinned.offline.len();
        assert_eq!(rows, 3, "a pinned channel was counted twice");

        assert!(settings.set_pinned("bob", false));
        let unpinned = groups(&follows, &offline, &settings.pinned, true);
        assert!(unpinned.pinned.is_empty());
        assert_eq!(unpinned.live, [&follows[0], &follows[1]]);
    }

    /// The count on the fold is of the rows under it, which leaves out the
    /// pins — whatever case they were written in.
    #[test]
    fn the_offline_count_leaves_out_pins() {
        let offline = [channel("alice"), channel("bob"), channel("carol")];
        let pinned = pins(&["Bob"]);

        let groups = groups(&[], &offline, &pinned, true);
        assert_eq!(groups.offline.len(), 2);
        assert_eq!(groups.offline, [&offline[0], &offline[2]]);
        assert_eq!(groups.pinned, [Row::Offline(&offline[1])]);
    }
}
