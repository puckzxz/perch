//! A pane's header: who is on, how many are watching, how long for, what
//! they are doing, whether it is muted, paused or in an ad break, and the
//! pane's own icons — how every chat is drawn, on a header that sits on chat
//! (`chat_menu`), out into a window of its own and back, and its ×. Where it
//! goes is [`Placement`]. It is also what a pane is dragged by, onto another
//! pane, to swap the two.
//!
//! It is the same header in both places. Above (or below) chat it sits on
//! the chat panel, where it costs nothing: chat is already a panel. With
//! chat hidden, or with no chat to show, there is no panel, and it rides the
//! top of the picture instead, on the same wash the control bar uses — up
//! while the pointer is on the pane, for a moment after a pane key, and at
//! rest over a pane with no picture to cover. That band, its fade and its
//! hold on the pointer are `watch::pane`'s; this file is only the header.

use gpui::{div, prelude::*, px, Context, MouseButton, MouseDownEvent, SharedString, Window};

use super::{pane_id, PaneAction, PaneDrag, PaneInfo, Showing, Slot, StreamState};
use crate::assets::Icon;
use crate::controls::{self, Variant};
use crate::keys::Hint;
use crate::{browse, channel_page, theme};

/// Where a pane's header goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// In the chat panel: above chat beside the picture, under the picture
    /// when stacked.
    Panel,
    /// Over the top of the picture, because there is no panel for it.
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

/// What a pane's header offers for a window of the pane's own: the
/// proposal's "header gains pop out and close as icons", beside the ×.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PopOutButton {
    /// Nothing: the pop-out is not offered here, or the pane has no player
    /// to move — starting, or stopped.
    Hidden,
    /// Move the picture into a window of its own.
    PopOut,
    /// Bring it back from one.
    PopIn,
}

/// Whether the header of a pane `showing` this offers the guide: when the
/// pane has stopped for good — off, ended, finished, unplayable, failed — or
/// its picture is in a window of its own. Those panes have no bar in this
/// window, where every other pane's Guide is, and a pane whose stream has
/// just ended is when "what else is on" matters most. Not while one is
/// starting: its bar is a moment away, and the icon would come and go.
fn offers_guide(showing: &Showing) -> bool {
    match showing {
        Showing::Picture | Showing::Starting { .. } => false,
        Showing::Offline
        | Showing::Unavailable
        | Showing::Ended
        | Showing::Finished
        | Showing::Failed(_)
        | Showing::Elsewhere => true,
    }
}

impl PopOutButton {
    /// Where the pop-out is `offered`: Bring back for a pane whose picture
    /// is `popped`, whatever it is doing there, and Pop out for one at home
    /// that is `playing`, which is the pane `RootView::pop_out` takes.
    fn of(offered: bool, popped: bool, playing: bool) -> Self {
        match (offered, popped, playing) {
            (false, _, _) => PopOutButton::Hidden,
            (true, true, _) => PopOutButton::PopIn,
            (true, false, true) => PopOutButton::PopOut,
            (true, false, false) => PopOutButton::Hidden,
        }
    }
}

/// Everything true about a stream that is not playback: who it is, how many
/// people are there, how long it has been going, what they are doing — and
/// the pane's icons, which name their keys: out into a window of its own or
/// back (`P`), where that is offered, and its × (`Ctrl+W`).
///
/// In a panel it is static information on a panel, where it costs nothing.
/// Over the picture it is the same information on the picture, which is
/// exactly what you end up staring past for three hours, so it comes and
/// goes there with the pointer; see [`Placement`]. `marked` underlines it as
/// the pane the keys talk to. `picture` says whether the pane has a picture
/// up, which with the header over the top of the pane decides who offers
/// chat back: the bar does when there is a picture, and this header does
/// when there is not, since without a picture there is no bar.
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
) -> impl IntoElement {
    let recording = slot.recording();
    let info = pane.live;
    let name = pane.name.clone();

    // Both are absent for a channel opened by name that no list carries, and
    // the uptime for one only the rail's recommendations know, which do not
    // say when it began. The same shape as the browse card's overlay,
    // "358 · 8h 20m", and for the same reason: the `LIVE` badge before it
    // already says what the first number counts, as the card's dot does, and
    // a header 340px wide has no room to say it again in words once `muted`
    // has to fit too.
    let meta = info
        .into_iter()
        .flat_map(|live| {
            [
                Some(browse::format_viewers(live.viewers)),
                live.started_at.and_then(browse::uptime),
            ]
        })
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");

    // What is actually on, which this header has never said: it knew who you
    // were watching and how many others were, and not a word about what they
    // were doing. The title and the game share one line, joined like `meta`
    // above, rather than taking one each — a pane header is 340px wide, and a
    // row of chrome here costs a row of chat in every pane on the page.
    //
    // A title is routinely longer than that line, so the whole of it — and the
    // game under it — is a hover away, through the same builder the browse
    // cards use.
    //
    // For a recording the second line is its title and when it was, in place
    // of a live stream's title and game: the game is not something Helix
    // says about a video, and the date is what tells two recordings apart.
    let about: Vec<SharedString> = match recording {
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
    let about_line = about
        .iter()
        .map(SharedString::as_ref)
        .collect::<Vec<_>>()
        .join(" · ");

    // Whether this pane is showing a picture, which is not the same as whether
    // it exists: an offline or failed pane used to draw the app's only
    // saturated red beside its name while the video underneath said the channel
    // was not streaming. A recording never gets the `LIVE` badge: nothing about
    // it is happening now.
    let playing = matches!(slot.state, StreamState::Playing(_)) && slot.is_live();
    // A stream that has finished takes its live numbers with it. They come
    // from a list that will not know for up to a minute, and an uptime that
    // goes on counting beside "ended the stream" is the same lie the frozen
    // last frame used to tell.
    let ended = matches!(slot.state, StreamState::Ended);
    // What the player is doing, read off the view rather than copied onto the
    // slot, so there is one source. These used to be visible only while the
    // pointer was over the video: a channel saved muted opened silent with
    // nothing on screen to say so, and a paused pane looked like a stalled
    // stream. The header is the one place a pane has for facts, so they go
    // here — in a panel always; over the picture, with the rest of the
    // header, while it is up. The quality does not: it is on the control bar,
    // and the header has no room for a fourth thing.
    let (muted, paused) = pane
        .player
        .as_ref()
        .map(|view| {
            let player = view.read(cx);
            (player.is_muted(), player.is_paused())
        })
        .unwrap_or((false, false));
    // An ad streamlink is filtering out, which holds the picture still with
    // nothing on it to say why: said here, off the picture, beside `muted`
    // and `paused`, counting down when streamlink gave a length
    // (`crate::ad_break`). The root repaints once a second while it is up.
    let ad_break = slot
        .ad_break
        .map(|ad| SharedString::from(ad.tag(std::time::Instant::now())));
    // The name opens what the pane is playing: the channel, or this one
    // recording, from its start — the name says which, never when. The
    // moment is More's to offer, on the bar.
    let url = slot.link(false);
    let tooltip = match recording {
        Some(_) => SharedString::from("Open this broadcast on twitch.tv"),
        None => SharedString::from(format!("Open twitch.tv/{}", slot.channel)),
    };

    // Over a pane with no picture there is no bar, so nothing else on screen
    // brings hidden chat back to the pointer: `C` would, and a pointer has to
    // be able to as well.
    let offer_chat = placement == Placement::OverPicture && !picture && slot.chat.is_some();
    let key = slot.key.clone();
    let show_chat = offer_chat.then(|| {
        let on_pane = on_pane.clone();
        let key = key.clone();
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

    // The guide, from a pane with no bar to offer it (`offers_guide`). On
    // the press, in the capture phase, stopped there, as the bar's Guide is
    // and for the same reason (`video_view::bar`'s `guide_button`): the
    // guide's own press outside, heard in the capture phase after this
    // header, would close it and the click after open it again. Stopped
    // there, the press begins no drag of the header either. Its id and
    // tooltip follow whether the guide is up from this pane, as the bar's
    // do.
    let guide = offers_guide(&pane.showing).then(|| {
        let on_pane = on_pane.clone();
        let key = key.clone();
        let (id, tip) = if pane.guide_from_here {
            ("guide-open", "Close the guide")
        } else {
            ("guide", "Show what else is on")
        };
        controls::icon_button(pane_id(&slot.key, id), Icon::Guide, Variant::OnVideo)
            .when(window_hovered, |button| button.tooltip(controls::tip(tip)))
            .capture_any_mouse_down(
                cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                    if event.button != MouseButton::Left || event.click_count > 1 {
                        return;
                    }
                    cx.stop_propagation();
                    on_pane(view, &key, PaneAction::Guide, window, cx)
                }),
            )
    });

    // Out into a window of its own, or back from it: what `P` does for the
    // pane the keys talk to, offered on every pane that can go or come back.
    // Its id follows what a press does, so a tooltip that came up over Pop
    // out does not go on saying so once `P` has popped the pane; see
    // `controls::tip`.
    let pop = PopOutButton::of(
        pane.pop_out_offered,
        matches!(pane.showing, Showing::Elsewhere),
        matches!(slot.state, StreamState::Playing(_)),
    );
    let pop = match pop {
        PopOutButton::Hidden => None,
        PopOutButton::PopOut => Some(("pop-out", Icon::PopOut, "Pop out", PaneAction::PopOut)),
        PopOutButton::PopIn => Some(("pop-in", Icon::PopIn, "Bring back", PaneAction::PopIn)),
    }
    .map(|(id, icon, words, action)| {
        let on_pane = on_pane.clone();
        let key = key.clone();
        controls::icon_button(pane_id(&slot.key, id), icon, Variant::OnVideo)
            .when(window_hovered, |button| {
                button.tooltip(controls::tip(Hint::PopOut.tooltip(words)))
            })
            .on_click(cx.listener(move |view, _event, window, cx| {
                if !cx.has_active_drag() {
                    on_pane(view, &key, action.clone(), window, cx)
                }
            }))
    });

    // How every chat is drawn, its text size and the time on every message:
    // the chat options menu (`chat_menu`), on a header that sits on a chat
    // panel and nowhere else, since with no chat on screen there is nothing
    // to see a change in. No key, like the menu's rows: the pointer is how
    // anyone reaches it.
    //
    // The icon is the menu's anchor, as the bar's right-hand cluster is the
    // anchor of the bar's menus: a press anywhere outside it closes the menu,
    // and a press on it is not "elsewhere", so it toggles rather than closing
    // the menu and opening it straight again. Its id follows whether the
    // menu is open, as Pop out's follows what a press does, so a tooltip up
    // before the press does not linger over the menu; and while the menu is
    // open there is none, since it would come up over the rows.
    let chat_options = (placement == Placement::Panel).then(|| {
        let open = pane.chat_menu.is_some();
        let on_toggle = on_pane.clone();
        let on_close = on_pane.clone();
        let toggle_key = key.clone();
        let close_key = key.clone();
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
                on_toggle(view, &toggle_key, PaneAction::ToggleChatMenu, window, cx)
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
    });

    // On every pane, a lone one included. It used to go when only one pane
    // was left, while `Ctrl+W` went on closing that one — a control the
    // keyboard had and the pointer did not. An icon now, like every other
    // control on a pane, and like them it names its key under the pointer.
    let on_drag = on_pane.clone();
    let close = controls::icon_button(
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
    }));

    // The handle a pane is dragged by, onto another; see above. Keyed by the
    // pane, like every id in it, so it is the same element wherever the pane
    // is in the grid.
    let owner = cx.entity().downgrade();
    div()
        .id(pane_id(&slot.key, "header"))
        .when(movable, |header| {
            header.on_drag(
                PaneDrag {
                    key: slot.key.clone(),
                },
                move |drag: &PaneDrag, _offset, window, cx| {
                    owner
                        .update(cx, |view, cx| {
                            on_drag(view, &drag.key, PaneAction::BeginMove, window, cx)
                        })
                        .ok();
                    cx.new(|_| gpui::Empty)
                },
            )
        })
        .flex_none()
        .w_full()
        .flex()
        // A column now, because what is on is a line of its own under who is
        // on. It does not fit beside them: the first row is already the name,
        // the numbers, `muted`, `paused` and the ×.
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
        .border_color(match (marked, placement) {
            (true, _) => theme::accent(),
            (false, Placement::Panel) => theme::border(),
            (false, Placement::OverPicture) => theme::border_clear(),
        })
        .child(
            div()
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::GAP_TIGHT))
                .child(
                    // The channel on twitch.tv, one click from its name: the
                    // site's chat and everything else the app does not do.
                    // A message can be sent from the chat's own composer.
                    // It gives way, with `meta` and the ad-break tag, when the
                    // row runs out: a live pane that is muted and paused has
                    // the red badge and two tags beside its name, and at the
                    // chat's narrower widths a long name held flex_none pushed
                    // the pop-out icon and the × past the edge. The tooltip
                    // still names the channel in full. The meta's pattern,
                    // which cuts the line rather than wrapping it.
                    div()
                        .id(pane_id(&slot.key, "open"))
                        .flex_shrink()
                        .min_w_0()
                        .text_ellipsis()
                        .line_clamp(1)
                        .text_size(px(theme::TEXT_BODY))
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
                        .child(name),
                )
                // Said where `muted` and `paused` are said, and for the same
                // reason: it is a fact about the pane that the picture alone
                // does not carry.
                // Which kind of recording, since the three play alike:
                // `replay` for a broadcast, and a highlight or an upload by
                // its own name.
                .when_some(recording, |header, video| {
                    header.child(controls::tag(channel_page::kind_tag(video.kind)))
                })
                // And a live pane says so in the same place, in the red
                // badge its timeline ends in, where it used to have the
                // browse cards' dot in front of its name: the header is up at
                // rest on chat, so this is where a glance down a page of panes
                // tells live from `replay`. It still says the numbers after
                // it are live rather than a playback position.
                .when(playing, |header| header.child(controls::live_badge()))
                .when(!ended && !meta.is_empty(), |header| {
                    header.child(
                        // `text_ellipsis` plus `line_clamp`, not `truncate`:
                        // see the handoff on why the latter clips mid-glyph in
                        // a flex row with no definite width, which is what this
                        // row is.
                        div()
                            .min_w_0()
                            .text_ellipsis()
                            .line_clamp(1)
                            .text_size(px(theme::TEXT_META))
                            .text_color(theme::text_muted())
                            .child(SharedString::from(meta)),
                    )
                })
                .when(muted, |header| header.child(controls::tag("muted")))
                .when(paused, |header| header.child(controls::tag("paused")))
                // The one tag that gives way, with `meta`, when the row runs
                // out: a narrow header on chat that is muted or paused as
                // well has no room for it beside the icons, and flex_none it
                // pushed the pop-out icon and the × past the edge. The
                // meta's pattern, which cuts the line rather than wrapping it.
                .children(ad_break.map(|words| {
                    controls::tag(words)
                        .flex_shrink()
                        .min_w_0()
                        .text_ellipsis()
                        .line_clamp(1)
                }))
                .child(div().flex_1())
                // The right-hand cluster: the guide and chat back, when only
                // this header can offer them, or the chat options, when it
                // sits on chat, then out or back, then the ×.
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(theme::GAP_TIGHT))
                        .children(guide)
                        .children(show_chat)
                        .children(chat_options)
                        .children(pop)
                        .child(close),
                ),
        )
        .when(!about.is_empty(), |header| {
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
                    .child(SharedString::from(about_line)),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The header offers the guide on a pane with no bar in this window —
    /// stopped, or its picture elsewhere — and not on one playing or
    /// starting, whose bar has it or soon will.
    #[test]
    fn the_header_offers_the_guide_only_where_no_bar_does() {
        let failure = SharedString::from("no");
        for showing in [
            Showing::Offline,
            Showing::Unavailable,
            Showing::Ended,
            Showing::Finished,
            Showing::Failed(&failure),
            Showing::Elsewhere,
        ] {
            assert!(offers_guide(&showing), "{showing:?}");
        }
        assert!(!offers_guide(&Showing::Picture));
        assert!(!offers_guide(&Showing::Starting {
            recording: false,
            at: None
        }));
    }

    /// A pane out offers to come back, whatever it is doing out there; one
    /// at home offers to go only with a player to move; and where the
    /// pop-out is not offered, no header says a word about it.
    #[test]
    fn the_header_offers_a_window_only_to_a_pane_that_can_go_or_come_back() {
        assert_eq!(PopOutButton::of(true, false, true), PopOutButton::PopOut);
        assert_eq!(PopOutButton::of(true, true, true), PopOutButton::PopIn);
        assert_eq!(
            PopOutButton::of(true, true, false),
            PopOutButton::PopIn,
            "restarting in its window"
        );
        assert_eq!(
            PopOutButton::of(true, false, false),
            PopOutButton::Hidden,
            "starting or stopped at home"
        );
        for (popped, playing) in [(false, false), (false, true), (true, false), (true, true)] {
            assert_eq!(
                PopOutButton::of(false, popped, playing),
                PopOutButton::Hidden
            );
        }
    }
}
