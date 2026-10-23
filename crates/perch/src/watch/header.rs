//! A pane's header: who is on, how many are watching, how long for, what
//! they are doing, whether it is muted, paused or in an ad break, and the
//! pane's own controls — how every chat is drawn, on a header that sits on
//! chat (`chat_menu`), the pane's menu behind a ⋯, and its ×. Where it goes
//! is [`Placement`]. It is also what a pane is dragged by, onto another
//! pane, to swap the two.
//!
//! Two shapes, from one set of facts ([`Facts`]). On the chat panel — at the
//! top of chat beside the picture, between the picture and chat when stacked
//! — it is the pane's details at their size: the channel's face, its name at
//! a title's size on a line of its own, then the badge, the numbers and the
//! tags, then what is on and its game, each on a line of its own
//! ([`details`]). The panel is a panel already, so the header costs the
//! picture nothing there. With chat hidden, or no chat to show, there is no
//! panel, and it rides the top of the picture instead, compact, on the wash
//! the control bar uses ([`compact`]) — up while the pointer is on the pane,
//! for a moment after a pane key, and at rest over a pane with no picture
//! to cover. That band, its fade and its hold on the pointer are
//! `watch::pane`'s; this file is only the header.
//!
//! For a week of October 2026 a beside pane's details went under its
//! picture, where its column had black to give, with the picture pinned to
//! the top and chat headed by a bar of its own. The user did not like it,
//! and they came back to the chat panel — laid out on lines of their own,
//! so the name is never cut to its first six letters by the badge, the
//! numbers and three icons, as it was there before. What the details under
//! the picture had as controls of their own — the pin, the channel's past
//! broadcasts, out into a window of its own and back — are rows of the
//! pane's menu, behind the ⋯ (`crate::channel_menu`).

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    canvas, div, img, prelude::*, px, AnyElement, Context, Div, ElementId, MouseButton,
    MouseDownEvent, Pixels, Point, SharedString, Stateful, Window,
};

use super::{pane_id, PaneAction, PaneDrag, PaneInfo, Slot, StreamState};
use crate::assets::Icon;
use crate::controls::{self, Variant};
use crate::keys::Hint;
use crate::{browse, channel_page, theme};

/// Where a pane's header goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// At the top of the chat panel: above chat beside the picture, between
    /// the picture and chat when stacked, as the pane's details at their
    /// size ([`details`]).
    Panel,
    /// Over the top of the picture, compact ([`compact`]), because there is
    /// no panel for it.
    OverPicture,
}

impl Placement {
    /// Where a pane whose chat is `chat_hidden`, and which has one at all,
    /// puts its header. One rule for both arrangements: with no chat on
    /// screen there is no panel to sit on, beside the picture or under it,
    /// and it goes over the picture rather than taking a strip of its own
    /// that would move the picture when `C` is pressed.
    pub fn of(chat_hidden: bool, has_chat: bool) -> Self {
        if chat_hidden || !has_chat {
            Placement::OverPicture
        } else {
            Placement::Panel
        }
    }
}

/// Whether a pane says `buffering`, its live picture `buffering` (waiting on
/// the network, `VideoView::is_buffering`), with or without an ad break up
/// (`in_ad_break`): only without, since an ad break holds the picture still
/// too, and its own tag says why. The one rule for the header, and for a
/// pop-out's word over its picture, since the header in the main window
/// reads only the player that window draws.
pub fn says_buffering(buffering: bool, in_ad_break: bool) -> bool {
    buffering && !in_ad_break
}

/// Everything a header says about its pane, worked out once and drawn in
/// whichever shape the pane has: its details on the chat panel
/// ([`details`]), or compact over the picture ([`compact`]).
pub(super) struct Facts {
    /// What the pane calls its channel.
    pub(super) name: SharedString,
    /// What the name opens, and what it says it opens.
    url: String,
    tooltip: SharedString,
    /// How many are watching, as the browse cards write it (`358`, `8.8k`),
    /// and for how long; both absent for a channel opened by name that no
    /// list carries, the uptime for one only the rail's recommendations
    /// know, and both once the stream has ended, since they come from a
    /// list that will not know for another minute.
    pub(super) viewers: Option<String>,
    pub(super) uptime: Option<String>,
    /// What is actually on: a live stream's title and its game, or a
    /// recording's title and when it was, empty parts left out.
    pub(super) about: Vec<SharedString>,
    /// The tag a recording wears where a live pane wears `LIVE`.
    kind: Option<&'static str>,
    /// Whether the pane shows a live picture, which earns the red badge.
    playing: bool,
    /// What the player is doing, and an ad break streamlink is holding the
    /// picture through, counting down where it said how long.
    muted: bool,
    paused: bool,
    buffering: bool,
    ad_break: Option<SharedString>,
}

impl Facts {
    /// What `slot`'s header says, from what is known about it (`pane`).
    pub(super) fn of<V: 'static>(slot: &Slot, pane: &PaneInfo, cx: &Context<V>) -> Facts {
        let recording = slot.recording();
        let info = pane.live;

        // A stream that has finished takes its live numbers with it. They
        // come from a list that will not know for up to a minute, and an
        // uptime that goes on counting beside "ended the stream" is the same
        // lie the frozen last frame used to tell.
        let ended = matches!(slot.state, StreamState::Ended);
        let (viewers, uptime) = match info.filter(|_| !ended) {
            Some(live) => (
                Some(browse::format_viewers(live.viewers)),
                live.started_at.and_then(browse::uptime),
            ),
            None => (None, None),
        };

        // What is actually on, which the header once never said: it knew who
        // you were watching and how many others were, and not a word about
        // what they were doing. For a recording, its title and when it was,
        // in place of a live stream's title and game: the game is not
        // something Helix says about a video, and the date is what tells two
        // recordings apart.
        let about = match recording {
            Some(video) => vec![
                video.title.clone(),
                channel_page::describe(video, chrono::Utc::now()),
            ],
            None => info
                .into_iter()
                .flat_map(|live| [live.title.to_string(), live.game.to_string()])
                .collect(),
        }
        .into_iter()
        .map(SharedString::from)
        .filter(|text| !text.trim().is_empty())
        .collect();

        // What the player is doing, read off the view rather than copied
        // onto the slot, so there is one source. These used to be visible
        // only while the pointer was over the video: a channel saved muted
        // opened silent with nothing on screen to say so, and a paused pane
        // looked like a stalled stream. The header is the one place a pane
        // has for facts, so they go here. The quality does not: it is on the
        // control bar.
        let (muted, paused, buffering) = pane
            .player
            .as_ref()
            .map(|view| {
                let player = view.read(cx);
                (player.is_muted(), player.is_paused(), player.is_buffering())
            })
            .unwrap_or((false, false, false));
        // An ad streamlink is filtering out, which holds the picture still
        // with nothing on it to say why: said here, off the picture, beside
        // `muted` and `paused`, counting down when streamlink gave a length
        // (`crate::ad_break`). The root repaints once a second while it is up.
        let ad_break = slot
            .ad_break
            .map(|ad| SharedString::from(ad.tag(std::time::Instant::now())));

        Facts {
            name: pane.name.clone(),
            // The name opens what the pane is playing: the channel, or this
            // one recording, from its start — the name says which, never
            // when. The moment is More's to offer, on the bar.
            url: slot.link(false),
            tooltip: match recording {
                Some(_) => SharedString::from("Open this broadcast on twitch.tv"),
                None => SharedString::from(format!("Open twitch.tv/{}", slot.channel)),
            },
            viewers,
            uptime,
            about,
            kind: recording.map(|video| channel_page::kind_tag(video.kind)),
            // Whether this pane is showing a picture, which is not the same
            // as whether it exists: an offline or failed pane used to draw
            // the app's only saturated red beside its name while the video
            // underneath said the channel was not streaming. A recording
            // never gets the `LIVE` badge: nothing about it is happening now.
            playing: matches!(slot.state, StreamState::Playing(_)) && slot.is_live(),
            muted,
            paused,
            buffering,
            ad_break,
        }
    }

    /// The live numbers on one line, the shape the browse card's overlay
    /// has: `358 · 8h 20m`, or `8.8k watching · 3h 12m` where there is room
    /// to say what the first one counts (`watching`). Empty when there are
    /// none.
    pub(super) fn numbers(&self, watching: bool) -> String {
        let viewers = self.viewers.as_ref().map(|viewers| {
            if watching {
                format!("{viewers} watching")
            } else {
                viewers.clone()
            }
        });
        viewers
            .into_iter()
            .chain(self.uptime.clone())
            .collect::<Vec<_>>()
            .join(" · ")
    }

    /// The title and the game, or a recording's title and when it was, on
    /// one line, for the compact header that has one line for them.
    fn about_line(&self) -> SharedString {
        self.about
            .iter()
            .map(SharedString::as_ref)
            .collect::<Vec<_>>()
            .join(" · ")
            .into()
    }
}

/// The channel's name, which opens it on twitch.tv — the site's chat and
/// everything else the app does not do — at `size`.
///
/// It gives way, with the numbers and the ad-break tag, when the row runs
/// out: a live pane that is muted and paused has the red badge and two tags
/// beside its name in the compact header, and at the chat's narrower widths
/// a long name held flex_none pushed the icons past the edge. The tooltip
/// still names the channel in full. The numbers' pattern, which cuts the
/// line rather than wrapping it. A click that ends a drag of the header is
/// no click on the name; see [`pane_header`].
pub(super) fn name_link<V: 'static>(
    slot: &Slot,
    facts: &Facts,
    size: f32,
    window_hovered: bool,
    cx: &mut Context<V>,
) -> Stateful<Div> {
    let url = facts.url.clone();
    let tooltip = facts.tooltip.clone();
    div()
        .id(pane_id(&slot.key, "open"))
        .flex_shrink()
        .min_w_0()
        .text_ellipsis()
        .line_clamp(1)
        .text_size(px(size))
        .font_weight(theme::weight_title())
        .text_color(theme::text())
        .cursor_pointer()
        .hover(|style| style.text_color(theme::accent()))
        .when(window_hovered, |name| name.tooltip(controls::tip(tooltip)))
        .on_click(cx.listener(move |_, _event, _window, cx| {
            if !cx.has_active_drag() {
                cx.open_url(&url)
            }
        }))
        .child(facts.name.clone())
}

/// What follows the name, onto `row`: a recording's kind or a live pane's
/// red badge, the `numbers`, then `muted`, `paused`, `buffering` and an ad
/// break's tag — on the name's row in the compact header, and on a row of
/// their own under the name in the details.
pub(super) fn with_status(row: Div, facts: &Facts, numbers: String) -> Div {
    row
        // Which kind of recording, since the three play alike: `replay` for
        // a broadcast, and a highlight or an upload by its own name.
        .when_some(facts.kind, |row, kind| row.child(controls::tag(kind)))
        // And a live pane says so in the same place, in the red badge its
        // timeline ends in: the header is up at rest, so this is where a
        // glance down a page of panes tells live from `replay`. It still
        // says the numbers after it are live rather than a playback position.
        .when(facts.playing, |row| row.child(controls::live_badge()))
        .when(!numbers.is_empty(), |row| {
            row.child(
                // `text_ellipsis` plus `line_clamp`, not `truncate`: see the
                // handoff on why the latter clips mid-glyph in a flex row with
                // no definite width, which is what this row is.
                div()
                    .min_w_0()
                    .text_ellipsis()
                    .line_clamp(1)
                    .text_size(px(theme::TEXT_META))
                    .text_color(theme::text_muted())
                    .child(SharedString::from(numbers)),
            )
        })
        .when(facts.muted, |row| row.child(controls::tag("muted")))
        .when(facts.paused, |row| row.child(controls::tag("paused")))
        // A live picture standing still on the network, said so it does not
        // read as a player that has hung; see `says_buffering`.
        .when(
            says_buffering(facts.buffering, facts.ad_break.is_some()),
            |row| row.child(controls::tag("buffering")),
        )
        // The one tag that gives way, with the numbers, when the row runs
        // out: a narrow header that is muted or paused as well has no room
        // for it, and flex_none it pushed the icons past the edge.
        .children(facts.ad_break.clone().map(|words| {
            controls::tag(words)
                .flex_shrink()
                .min_w_0()
                .text_ellipsis()
                .line_clamp(1)
        }))
}

/// How every chat is drawn, its text size and the time on every message:
/// the chat options menu (`chat_menu`), offered over a pane's chat and
/// nowhere else, since with no chat on screen there is nothing to see a
/// change in — on the header when it sits on chat. No key, like the menu's
/// rows: the pointer is how anyone reaches it.
///
/// The icon is the menu's anchor, as the bar's right-hand cluster is the
/// anchor of the bar's menus: a press anywhere outside it closes the menu,
/// and a press on it is not "elsewhere", so it toggles rather than closing
/// the menu and opening it straight again. Its id follows whether the menu
/// is open, as a pill's follows its state, so a tooltip up before the press
/// does not linger over the menu; and while the menu is open there is none,
/// since it would come up over the rows.
pub(super) fn chat_options<V: 'static>(
    slot: &Slot,
    pane: &PaneInfo,
    window_hovered: bool,
    on_pane: impl Fn(&mut V, &str, PaneAction, &mut Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> Div {
    let open = pane.chat_menu.is_some();
    let on_close = on_pane.clone();
    let toggle_key = slot.key.clone();
    let close_key = slot.key.clone();
    let button = controls::icon_button(
        pane_id(
            &slot.key,
            if open {
                "chat-options-open"
            } else {
                "chat-options"
            },
        ),
        Icon::ChatOptions,
        Variant::OnVideo,
    )
    .when(window_hovered && !open, |button| {
        button.tooltip(controls::tip("Chat options"))
    })
    .on_click(cx.listener(move |view, _event, window, cx| {
        if !cx.has_active_drag() {
            on_pane(view, &toggle_key, PaneAction::ToggleChatMenu, window, cx)
        }
    }));
    div()
        .flex_none()
        .when(open, |anchor| {
            anchor.on_mouse_down_out(cx.listener(move |view, _event, window, cx| {
                on_close(view, &close_key, PaneAction::CloseChatMenu, window, cx)
            }))
        })
        .child(button)
}

/// The pane's menu (`crate::channel_menu`), from the ⋯ at the end of its
/// header: what the right button opens on its picture or its header, for a
/// pointer that does not reach for the right button — `Pin to the rail`,
/// `Past broadcasts`, `Pop out` or `Bring back`, and the rest. Hung under
/// the ⋯, its top right corner at the ⋯'s lower right, which a probe
/// measures as the header is painted, since a press says only where the
/// pointer was; a second press on the ⋯ puts the menu away
/// (`RootView::toggle_pane_menu`). On the press, as a right-click opens the
/// menu, and only the first press of a run, as the menu's rows act.
pub(super) fn menu_button<V: 'static>(
    slot: &Slot,
    name: &SharedString,
    window_hovered: bool,
    on_pane: impl Fn(&mut V, &str, PaneAction, &mut Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> Div {
    let key = slot.key.clone();
    // Where the ⋯'s lower right corner was painted, for the press to hang
    // the menu from. The probe sets it as the frame is laid out, before the
    // press's listener is registered with the same frame.
    let corner: Rc<Cell<Option<Point<Pixels>>>> = Rc::default();
    let measured = corner.clone();
    let probe = canvas(
        move |bounds, _window, _cx| measured.set(Some(bounds.bottom_right())),
        |_, _, _, _| {},
    )
    .absolute()
    .size_full();
    let tip = format!("More for {name}");
    let button = controls::icon_button(pane_id(&slot.key, "menu"), Icon::More, Variant::OnVideo)
        .when(window_hovered, |button| button.tooltip(controls::tip(tip)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                if event.click_count > 1 {
                    return;
                }
                let under = corner.get().unwrap_or(event.position);
                on_pane(view, &key, PaneAction::MenuButton(under), window, cx)
            }),
        );
    div().relative().flex_none().child(button).child(probe)
}

/// The pane's ×, on every pane, a lone one included. It used to go when only
/// one pane was left, while `Ctrl+W` went on closing that one — a control the
/// keyboard had and the pointer did not. An icon, like every other control
/// on a pane, and like them it names its key under the pointer.
pub(super) fn close_button<V: 'static>(
    slot: &Slot,
    window_hovered: bool,
    on_pane: impl Fn(&mut V, &str, PaneAction, &mut Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> Stateful<Div> {
    let key = slot.key.clone();
    controls::icon_button(
        pane_id(&slot.key, "close"),
        Icon::Close,
        Variant::Destructive,
    )
    .when(window_hovered, |button| {
        button.tooltip(controls::tip(Hint::Close.tooltip("Close")))
    })
    .on_click(cx.listener(move |view, _event, window, cx| {
        if !cx.has_active_drag() {
            on_pane(view, &key, PaneAction::Close, window, cx)
        }
    }))
}

/// The header as the handle a pane is dragged by, onto another, with two
/// panes or more on the page (`movable`); see [`pane_header`]. Keyed by the
/// pane, like every id in it, so it is the same element wherever the pane
/// is in the grid, and whichever shape the header is in.
pub(super) fn handle<V: 'static>(
    slot: &Slot,
    movable: bool,
    on_pane: impl Fn(&mut V, &str, PaneAction, &mut Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> Stateful<Div> {
    let owner = cx.entity().downgrade();
    let on_menu = on_pane.clone();
    let key = slot.key.clone();
    div()
        .id(pane_id(&slot.key, "header"))
        // The pane's menu, about the channel it plays (`crate::channel_menu`),
        // as on its picture.
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                on_menu(view, &key, PaneAction::Menu(event.position), window, cx)
            }),
        )
        .when(movable, |header| {
            header.on_drag(
                PaneDrag {
                    key: slot.key.clone(),
                },
                move |drag: &PaneDrag, _offset, window, cx| {
                    owner
                        .update(cx, |view, cx| {
                            on_pane(view, &drag.key, PaneAction::BeginMove, window, cx)
                        })
                        .ok();
                    cx.new(|_| gpui::Empty)
                },
            )
        })
}

/// Everything true about a stream that is not playback: who it is, how many
/// people are there, how long it has been going, what they are doing — and
/// the pane's controls, which name their keys where they have one.
///
/// On the chat panel it is static information on a panel, where it costs
/// nothing, laid out at its size ([`details`]). Over the picture it is the
/// same information on the picture, which is exactly what you end up
/// staring past for three hours, so it is compact there and comes and goes
/// with the pointer ([`compact`]); see [`Placement`]. `marked` underlines it
/// as the pane the keys talk to. `picture` says whether the pane has a
/// picture up, which with the header over the top of the pane decides who
/// offers chat back: the bar does when there is a picture, and this header
/// does when there is not, since without a picture there is no bar.
///
/// With two panes or more on the page (`movable`) the whole header is a
/// handle: dragged onto another pane, it swaps the two
/// (`PaneAction::MoveOnto`, which that pane's drop layer sends). The drag
/// says it has begun (`PaneAction::BeginMove`) so the other panes offer
/// themselves, and is drawn as nothing: gpui's drag belongs to the app, and
/// every window paints its preview, a pop-out over its picture included
/// (window.rs:2055-2060). A press on one of the header's controls can begin
/// a drag too, and then is no press on the control: each control ignores a
/// click that ends a drag, which gpui would otherwise still deliver when
/// the drag began and ended between two frames (div.rs:1569-1577) — the
/// name would open twitch.tv.
///
/// Its tooltips are there only while `window_hovered`, from
/// `window.is_window_hovered()`, as the bar's are (`bar::bar_icon`): gpui
/// hears the pointer leave the window as a flag, never a move, so a tooltip
/// up at the time would stay where the pointer was last seen, and the
/// right-hand pane's × is a few pixels from the window's edge. Dropping the
/// builder clears it (div.rs:1660-1668) — over the picture even after the
/// band has faded out, since an invisible element is still prepainted.
#[allow(clippy::too_many_arguments)]
pub(super) fn pane_header<V: 'static>(
    slot: &Slot,
    pane: &PaneInfo,
    placement: Placement,
    marked: bool,
    movable: bool,
    picture: bool,
    window_hovered: bool,
    on_pane: impl Fn(&mut V, &str, PaneAction, &mut Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> AnyElement {
    let facts = Facts::of(slot, pane, cx);
    let handle = handle(slot, movable, on_pane.clone(), cx);
    match placement {
        Placement::Panel => details(
            handle,
            slot,
            pane,
            &facts,
            marked,
            window_hovered,
            on_pane,
            cx,
        )
        .into_any_element(),
        Placement::OverPicture => compact(
            handle,
            slot,
            &facts,
            marked,
            picture,
            window_hovered,
            on_pane,
            cx,
        )
        .into_any_element(),
    }
}

/// The header on the chat panel, the pane's details at their size: the
/// channel's face where the rail has one (`PaneInfo::avatar`), its name at
/// a title's size with the pane's controls at the end of its line — the
/// chat options, the ⋯ and the × — then on a line of its own the badge, the
/// numbers saying what they count, and the tags, then the title on a line
/// of its own at the size you read, and the game under it. On `handle`, the
/// header's own element.
#[allow(clippy::too_many_arguments)]
fn details<V: 'static>(
    handle: Stateful<Div>,
    slot: &Slot,
    pane: &PaneInfo,
    facts: &Facts,
    marked: bool,
    window_hovered: bool,
    on_pane: impl Fn(&mut V, &str, PaneAction, &mut Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> Stateful<Div> {
    let name = name_link(slot, facts, theme::TEXT_TITLE, window_hovered, cx);
    let options = chat_options(slot, pane, window_hovered, on_pane.clone(), cx);
    let menu = menu_button(slot, &facts.name, window_hovered, on_pane.clone(), cx);
    let close = close_button(slot, window_hovered, on_pane, cx);
    // A line of their own, so there is room to say what the first number
    // counts, which the compact header leaves to the badge in front of it.
    let status = with_status(
        div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::GAP_TIGHT)),
        facts,
        facts.numbers(true),
    );
    let about = about_lines(slot, facts, window_hovered);

    handle
        .flex_none()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(theme::GAP_TIGHT))
        .px(px(theme::ROW_PAD_X))
        .pt(px(theme::GAP))
        .pb(px(theme::PANEL_PAD))
        .border_b_1()
        // Which pane the keyboard is talking to, with more than one to tell
        // apart; see `compact`.
        .border_color(if marked {
            theme::accent()
        } else {
            theme::border()
        })
        .child(
            div()
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::GAP))
                .children(pane.avatar.clone().map(|path| {
                    img(path)
                        .flex_none()
                        .size(px(theme::HEADER_AVATAR))
                        .rounded_full()
                }))
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
                                .child(name)
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .flex_none()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap(px(theme::GAP_WORD))
                                        .child(options)
                                        .child(menu)
                                        .child(close),
                                ),
                        )
                        .child(status),
                ),
        )
        .children(about)
}

/// The header over the top of the picture, compact: the name, the badge,
/// the numbers in the browse card's shape and the tags on one row with the
/// controls at its end — chat back where only this header can offer it,
/// the ⋯ and the × — and what is on, joined on one line under it. The
/// band it rides is `theme::BAND_ROOM` tall, worked out from this shape.
#[allow(clippy::too_many_arguments)]
fn compact<V: 'static>(
    handle: Stateful<Div>,
    slot: &Slot,
    facts: &Facts,
    marked: bool,
    picture: bool,
    window_hovered: bool,
    on_pane: impl Fn(&mut V, &str, PaneAction, &mut Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> Stateful<Div> {
    let about = facts.about.clone();
    let about_line = facts.about_line();

    // Over a pane with no picture there is no bar, so nothing else on screen
    // brings hidden chat back to the pointer: `C` would, and a pointer has to
    // be able to as well.
    let show_chat = (!picture && slot.chat.is_some()).then(|| {
        let on_pane = on_pane.clone();
        let key = slot.key.clone();
        controls::icon_button(
            pane_id(&slot.key, "show-chat"),
            Icon::ChatOff,
            Variant::OnVideo,
        )
        .when(window_hovered, |button| {
            button.tooltip(controls::tip(Hint::Chat.tooltip("Show chat")))
        })
        .on_click(cx.listener(move |view, _event, window, cx| {
            if !cx.has_active_drag() {
                on_pane(view, &key, PaneAction::ToggleChat, window, cx)
            }
        }))
    });
    let menu = menu_button(slot, &facts.name, window_hovered, on_pane.clone(), cx);
    let close = close_button(slot, window_hovered, on_pane, cx);
    // The numbers in the browse card's shape, "358 · 8h 20m", for the card's
    // reason: the `LIVE` badge before them already says what the first one
    // counts, and a row over the picture has the name and the tags too.
    let numbers = facts.numbers(false);
    let name = name_link(slot, facts, theme::TEXT_BODY, window_hovered, cx);

    handle
        .flex_none()
        .w_full()
        .flex()
        // A column, because what is on is a line of its own under who is on.
        // It does not fit beside them: the first row is already the name,
        // the numbers, `muted`, `paused` and the controls.
        .flex_col()
        .gap(px(theme::GAP_WORD))
        .px(px(theme::ROW_PAD_X))
        .pb(px(theme::GAP_TIGHT))
        .border_b_1()
        // Which pane the keyboard is talking to. `Space`, `M`, the arrows and
        // `Ctrl+W` all act on the pane you last pointed at, and with four on
        // screen nothing said which that was — so every press was a guess. One
        // line under one header, and only when there is more than one pane to
        // tell apart. Over the picture the rule is there unmarked too, only
        // clear, so the header is the same height marked or not and a pane
        // key's reveal never moves a line of it.
        .border_color(if marked {
            theme::accent()
        } else {
            theme::border_clear()
        })
        .child(
            with_status(
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(theme::GAP_TIGHT))
                    .child(name),
                facts,
                numbers,
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(theme::GAP_TIGHT))
                    .children(show_chat)
                    .child(menu)
                    .child(close),
            ),
        )
        .when(!about.is_empty(), |header| {
            // The title and the game share one line, joined like the numbers
            // above, rather than taking one each: over the picture, a row of
            // chrome is a row of the picture. A title is routinely longer
            // than that line, so the whole of it — and the game under it — is
            // a hover away, through the same builder the browse cards use.
            header.child(
                // `w_full`, not `min_w_0`: this one is a child of a flex
                // column, where a definite width is what the measure pass
                // needs before it will ellipsise at all.
                div()
                    .id(pane_id(&slot.key, "about"))
                    .w_full()
                    .text_ellipsis()
                    .line_clamp(1)
                    .text_size(px(theme::TEXT_META))
                    .line_height(px(theme::LINE_TIGHT))
                    .text_color(theme::text_dim())
                    .when(window_hovered, |line| {
                        line.tooltip(controls::full_text(about))
                    })
                    .child(about_line),
            )
        })
}

/// What is on, under who is on, in the details: the title on a line of its
/// own at the size you read, and the game under it in the supporting tier —
/// or a recording's title and when it was. Each cut to its line, with the
/// whole of both a hover away on the title, through the builder the browse
/// cards use.
fn about_lines(slot: &Slot, facts: &Facts, window_hovered: bool) -> Vec<Stateful<Div>> {
    let mut lines = facts.about.iter().cloned();
    let title = lines.next();
    let rest = lines.next();
    let whole = facts.about.clone();
    title
        .map(|title| {
            line(pane_id(&slot.key, "about"), title)
                .text_size(px(theme::TEXT_BODY))
                .line_height(px(theme::LINE_BODY))
                .text_color(theme::text())
                .when(window_hovered, |line| {
                    line.tooltip(controls::full_text(whole))
                })
        })
        .into_iter()
        .chain(rest.map(|rest| {
            line(pane_id(&slot.key, "about-more"), rest)
                .text_size(px(theme::TEXT_META))
                .line_height(px(theme::LINE_TIGHT))
                .text_color(theme::text_muted())
        }))
        .collect()
}

/// One line of the details' text, cut with an ellipsis where it runs out:
/// `w_full`, since a child of a flex column needs a definite width before
/// gpui's measure pass will ellipsise it.
fn line(id: impl Into<ElementId>, text: SharedString) -> Stateful<Div> {
    div()
        .id(id)
        .w_full()
        .text_ellipsis()
        .line_clamp(1)
        .child(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture waiting on the network says so, unless an ad break holds
    /// it still and says why; a picture that is not waiting says nothing,
    /// ad break or none.
    #[test]
    fn buffering_is_said_only_outside_an_ad_break() {
        assert!(says_buffering(true, false));
        assert!(!says_buffering(true, true));
        assert!(!says_buffering(false, false));
        assert!(!says_buffering(false, true));
    }

    fn facts(viewers: Option<&str>, uptime: Option<&str>) -> Facts {
        Facts {
            name: "lanternfall".into(),
            url: String::new(),
            tooltip: SharedString::default(),
            viewers: viewers.map(str::to_string),
            uptime: uptime.map(str::to_string),
            about: Vec::new(),
            kind: None,
            playing: true,
            muted: false,
            paused: false,
            buffering: false,
            ad_break: None,
        }
    }

    /// The numbers say what the first one counts only where there is room
    /// to, on a line of their own in the details; the compact header keeps
    /// the card's shape. Either part can be missing, and with neither there
    /// is nothing.
    #[test]
    fn the_numbers_say_watching_only_where_there_is_room() {
        let both = facts(Some("8.8k"), Some("3h 12m"));
        assert_eq!(both.numbers(false), "8.8k · 3h 12m");
        assert_eq!(both.numbers(true), "8.8k watching · 3h 12m");
        assert_eq!(facts(Some("358"), None).numbers(true), "358 watching");
        assert_eq!(facts(None, Some("25m")).numbers(true), "25m");
        assert_eq!(facts(None, None).numbers(true), "");
    }

    /// The title and the game, or a recording's title and when it was, go
    /// on one line in the compact header, joined like the numbers, with the
    /// parts there are.
    #[test]
    fn the_compact_header_joins_what_is_on() {
        let mut both = facts(None, None);
        both.about = vec!["first playthrough".into(), "Silent Hill f".into()];
        assert_eq!(both.about_line(), "first playthrough · Silent Hill f");
        let mut title = facts(None, None);
        title.about = vec!["first playthrough".into()];
        assert_eq!(title.about_line(), "first playthrough");
        assert_eq!(facts(None, None).about_line(), "");
    }
}
