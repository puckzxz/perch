//! A pane's header: who is on, how many are watching, how long for, what
//! they are doing, and the pane's ×. Where it goes is [`Placement`].
//!
//! It is the same header in both places. Above (or below) chat it sits on
//! the chat panel, where it costs nothing: chat is already a panel. With
//! chat hidden, or with no chat to show, there is no panel, and it rides the
//! top of the picture instead, on the same wash the control bar uses — up
//! while the pointer is on the pane, for a moment after a pane key, and at
//! rest over a pane with no picture to cover. That band, its fade and its
//! hold on the pointer are `watch::pane`'s; this file is only the header.

use gpui::{div, prelude::*, px, Context, SharedString, Window};

use super::{pane_id, PaneAction, PaneInfo, Slot, StreamState};
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

/// Everything true about a stream that is not playback: who it is, how many
/// people are there, how long it has been going, what they are doing — and
/// the pane's ×, which names its key.
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
    picture: bool,
    window_hovered: bool,
    on_pane: impl Fn(&mut V, &str, PaneAction, &mut Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let recording = slot.recording();
    let info = pane.stream;
    let name = pane.name.clone();

    // Both are absent for a channel opened by name that you do not follow:
    // the follows poll is where these numbers come from, and it only knows
    // about channels you follow. The same shape as the browse card's overlay,
    // "358 · 8h 20m", and for the same reason: the live dot beside it already
    // says what the first number counts, and a header 340px wide has no room
    // to say it again in words once `muted` has to fit too.
    let meta = info
        .into_iter()
        .flat_map(|stream| {
            [
                Some(browse::format_viewers(stream.viewer_count)),
                browse::uptime(&stream.started_at),
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
            .flat_map(|stream| [stream.title.clone(), stream.game_name.clone()])
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
    // was not streaming. A recording never gets the dot: nothing about it is
    // happening now.
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
    let (muted, paused) = slot
        .video()
        .map(|view| {
            let player = view.read(cx);
            (player.is_muted(), player.is_paused())
        })
        .unwrap_or((false, false));
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
            on_pane(view, &key, PaneAction::ToggleChat, window, cx)
        }))
    });

    // On every pane, a lone one included. It used to go when only one pane
    // was left, while `Ctrl+W` went on closing that one — a control the
    // keyboard had and the pointer did not. An icon now, like every other
    // control on a pane, and like them it names its key under the pointer.
    let close = controls::icon_button(
        pane_id(&slot.key, "close"),
        Icon::Close,
        Variant::Destructive,
    )
    .when(window_hovered, |button| {
        button.tooltip(controls::tip(Hint::Close.tooltip("Close")))
    })
    .on_click(cx.listener(move |view, _event, window, cx| {
        on_pane(view, &key, PaneAction::Close, window, cx)
    }));

    div()
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
                .when(playing, |header| {
                    // The same dot the browse cards use, for the same reason:
                    // it says the numbers beside it are live rather than a
                    // playback position.
                    header.child(controls::live_dot())
                })
                .child(
                    // Chat here is read-only by design. This is the way out of that:
                    // the one thing the app deliberately cannot do, one click from the
                    // name of the channel you would be saying it in.
                    div()
                        .id(pane_id(&slot.key, "open"))
                        .flex_none()
                        .text_size(px(theme::TEXT_BODY))
                        .font_weight(theme::weight_title())
                        .text_color(theme::text())
                        .cursor_pointer()
                        .hover(|style| style.text_color(theme::accent()))
                        .when(window_hovered, |name| name.tooltip(controls::tip(tooltip)))
                        .on_click(cx.listener(move |_, _event, _window, cx| cx.open_url(&url)))
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
                .child(div().flex_1())
                // The right-hand cluster: chat back, when only this header
                // can offer it, then the ×.
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(theme::GAP_TIGHT))
                        .children(show_chat)
                        // phase 3: maximize in window, pop out
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
