//! What a pane shows when it has no picture: one reading of the pane,
//! [`Showing`], and the screen drawn from it.
//!
//! One reading because two places say it, at two lengths: the pane, in a
//! sentence across the middle of where the picture would be, and the mini
//! player's tile, in a word. They used to read the slot separately — a
//! `status_message` here and a `state_word` in the mini player — so a new
//! state had two matches to land in, and nothing to say when it missed one.
//!
//! The screen is what a pane is waiting for, or what to do next. Starting,
//! it is the picture being waited for — the channel's live preview, the one
//! its browse card shows, or a recording's own thumbnail — dimmed under the
//! channel's name, and drawn under the player, so the first frame fades in
//! over it rather than out of black. Stopped, it says why, and offers what
//! there is: an offline channel's last broadcast and `Start when they go
//! live`, an ended broadcast's recording from its start, and asking again.

use chrono::Utc;
use emotes::ImageCache;
use gpui::{div, img, prelude::*, px, AnyElement, Context, Div, SharedString, Stateful, Window};
use gpui_component::switch::Switch;
use twitch_api::Video;

use super::{pane_id, placement, NextUp, PaneAction, PaneInfo, Placement, Slot, StreamState};
use crate::browse::{self, Action};
use crate::channel_page::{self, Card};
use crate::controls::{self, Variant};
use crate::{motion, seek_bar, theme};

/// What a pane is showing, read once from its slot for everything that says
/// it.
///
/// The same events read for what was asked for. streamlink says "no playable
/// streams" both for a channel that is off and for a recording that has
/// expired, and mpv's end of file is a broadcast finishing or a recording
/// reaching its end; only the pane knows which it asked for, so the reading
/// is made here, once, rather than by everything that words it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Showing<'a> {
    /// The player has the pane, and nothing is said over it.
    Picture,
    /// Waiting on streamlink to find the stream, or on the player's first
    /// frame to fade in. `at` is where a recording opens when that is not
    /// the top: picked up from the history, a link's moment, or a quality
    /// change part-way through.
    Starting { recording: bool, at: Option<f64> },
    /// The channel was not broadcasting when the pane asked.
    Offline,
    /// The recording would not play: expired, or taken down.
    Unavailable,
    /// The broadcast stopped while the pane was showing it.
    Ended,
    /// The recording played to its end.
    Finished,
    /// Something went wrong, and this is what.
    Failed(&'a SharedString),
    /// The picture is in a window of its own (`crate::stage`): the pane
    /// keeps its place, its header and its chat here, and says where its
    /// picture went, with the way to bring it back.
    Elsewhere,
}

/// How `slot` reads; see [`Showing`]. `covered` is whether its player's
/// picture has faded in over the whole pane (`VideoView::covers`), which
/// until it has leaves a pane with a player still starting; `popped` is
/// whether that picture is in a window of its own, which outranks
/// everything else the pane could say. A popped pane that stops playing
/// comes home (`RootView::restage`), so `Elsewhere` only ever covers a pane
/// that is starting or playing.
pub fn showing(slot: &Slot, covered: bool, popped: bool) -> Showing<'_> {
    if popped {
        return Showing::Elsewhere;
    }
    let recording = !slot.is_live();
    let starting = Showing::Starting {
        recording,
        at: (recording && slot.resume_at >= 1.0).then_some(slot.resume_at),
    };
    match &slot.state {
        StreamState::Playing(_) => player(covered, starting),
        StreamState::Starting => starting,
        StreamState::Offline if recording => Showing::Unavailable,
        StreamState::Offline => Showing::Offline,
        StreamState::Ended if recording => Showing::Finished,
        StreamState::Ended => Showing::Ended,
        StreamState::Failed(reason) => Showing::Failed(reason),
    }
}

/// What a pane with a player shows: its picture once that covers the pane,
/// and until then what the pane showed while it was `starting`.
///
/// A player draws nothing before its first frame, and that frame takes
/// `theme::MOTION_VIDEO` to fade in. Read as the picture at once, the pane
/// cut to black the moment streamlink was ready — the poster gone, a second
/// or two of nothing, then the picture fading in out of the black. Read as
/// still starting until the fade is done, what was being waited for stays
/// under the picture the whole way, and a tile goes on saying `Starting…`
/// rather than a player's own word for it.
fn player(covered: bool, starting: Showing<'_>) -> Showing<'_> {
    if covered {
        Showing::Picture
    } else {
        starting
    }
}

impl Showing<'_> {
    /// What a mini-player tile with no picture says instead: short, because
    /// a tile is a few words wide. The pane says the whole of it, one click
    /// away.
    pub fn word(&self) -> Option<&'static str> {
        match self {
            Showing::Picture => None,
            Showing::Starting { .. } => Some("Starting…"),
            Showing::Offline => Some("Offline"),
            Showing::Unavailable => Some("Unavailable"),
            Showing::Ended => Some("Ended"),
            Showing::Finished => Some("Finished"),
            Showing::Failed(_) => Some("Failed"),
            // No tile reads it: the mini player draws only the panes the
            // main window has (`RootView::mini_slots`). A word all the same,
            // so a tile that ever did would not go blank.
            Showing::Elsewhere => Some("Elsewhere"),
        }
    }

    /// What the pane says across the middle of where its picture would be,
    /// about the channel it calls `name`.
    ///
    /// Sentence case, like the tile's words and the pill under it. These were
    /// lowercase while every control around them was too; once the controls
    /// took capitals, a lowercase line over a `Try again` read as a fragment
    /// that had lost its start.
    ///
    /// Except a failure, which is said in the words of whatever failed —
    /// streamlink, mpv, the streamlink crate or the player — passed through
    /// as is. Those words are mostly lowercase, so a failed pane still reads
    /// as the fragment above; HANDOFF lists it among the lines not swept yet.
    fn sentence(&self, name: &str) -> Option<SharedString> {
        Some(match self {
            Showing::Picture => return None,
            Showing::Starting {
                recording: true,
                at: Some(at),
            } => format!("Opening at {}…", seek_bar::timecode(*at)).into(),
            Showing::Starting {
                recording: true,
                at: None,
            } => "Opening the recording…".into(),
            Showing::Starting { .. } => "Starting…".into(),
            Showing::Offline => format!("{name} is offline").into(),
            Showing::Unavailable => "This recording is no longer available".into(),
            Showing::Ended => format!("{name} ended the stream").into(),
            Showing::Finished => "Finished".into(),
            Showing::Failed(reason) => (*reason).clone(),
            Showing::Elsewhere => "Playing in its own window".into(),
        })
    }

    /// Whether asking again is on offer, and what the control says.
    ///
    /// Not the same as failing: a stream that ended is not a fault, and asking
    /// for it again is still a useful move — a channel that dropped out comes
    /// back, and one that is really finished says so through the offline
    /// state. A recording that has finished offers to start over.
    fn next_step(&self) -> Option<&'static str> {
        match self {
            // Bringing it back is its own control; see `screen`.
            Showing::Picture | Showing::Starting { .. } | Showing::Elsewhere => None,
            Showing::Finished => Some("Watch again"),
            Showing::Offline | Showing::Unavailable | Showing::Ended | Showing::Failed(_) => {
                Some("Try again")
            }
        }
    }
}

/// How an offline pane offers its channel's last broadcast, by the room its
/// screen has.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum NextUpRoom {
    /// As the card a channel's page shows it as, this wide: its picture,
    /// how long it is, and where it was left.
    Card(f32),
    /// As one pill naming it, with its title and when it was a hover away.
    Line,
    /// Not at all: the pane is too short for more than its words and its
    /// controls.
    Nothing,
}

/// A pill's height and the gap above it: what [`NextUpRoom::Line`] needs
/// beside the rest of the screen.
const PILL_ROW: f32 = theme::LINE_TIGHT + 2.0 * theme::CONTROL_PAD_Y + theme::GAP;

/// What a stopped pane whose video box is `width` by `height` has room for,
/// beside its sentence and controls ([`theme::STATUS_ROOM`]).
///
/// A card as wide as the box allows, up to the browse grid's widest, and
/// narrowed to the box's height when that is what runs out first; never
/// narrower than the grid's narrowest, below which its title stops being
/// worth reading. Short of that, a pill, if the box has a row to spare.
pub(super) fn next_up_room(width: f32, height: f32) -> NextUpRoom {
    let across = (width - 2.0 * theme::PANEL_PAD).min(browse::CARD_MAX);
    let picture = height - theme::VIDEO_CARD_TEXT - theme::STATUS_ROOM;
    let card = across.min(picture * 16.0 / 9.0);
    if card >= browse::CARD_MIN {
        NextUpRoom::Card(card)
    } else if height >= theme::STATUS_ROOM + PILL_ROW {
        NextUpRoom::Line
    } else {
        NextUpRoom::Nothing
    }
}

/// What an ended pane offers for watching the broadcast again from its
/// start.
#[derive(Debug, PartialEq)]
enum FromStart<'a> {
    /// The recording of the broadcast that ended, found among the channel's
    /// archives.
    Offered(&'a Video),
    /// Still asking whether there is one: the control, not yet on offer.
    Looking,
    /// No archive matched, or nobody could ask. Nothing is offered rather
    /// than an older broadcast, which would not be this one.
    Nothing,
}

/// What `next` and `looking`, from [`PaneInfo`], come to for an ended pane.
fn from_start<'a>(next: Option<NextUp<'a>>, looking: bool) -> FromStart<'a> {
    match (next, looking) {
        (Some(next), _) => FromStart::Offered(next.video),
        (None, true) => FromStart::Looking,
        (None, false) => FromStart::Nothing,
    }
}

/// The pane with no picture: what it is waiting for, or what it is showing
/// in a sentence and what there is to do about it. Nothing at all for a pane
/// that has its picture.
///
/// Absolute over the whole pane, so it splits nothing with the player, which
/// is drawn over it; see `watch::pane`. `room` is what the box has room for
/// beside the sentence ([`next_up_room`]), `window_hovered` gates the one
/// tooltip, as the header's are gated, and `cache` is where the pictures
/// come from.
#[allow(clippy::too_many_arguments)]
pub(super) fn screen<V: 'static>(
    slot: &Slot,
    pane: &PaneInfo,
    showing: Showing<'_>,
    room: NextUpRoom,
    window_hovered: bool,
    cache: &ImageCache,
    on_pane: impl Fn(&mut V, &str, PaneAction, &mut Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> AnyElement {
    let Some(text) = showing.sentence(&pane.name) else {
        return div().into_any_element();
    };
    // With no chat on screen the pane's header rests over the top of this
    // screen, which has no picture for it to keep clear of; the words are
    // centred in what it leaves rather than under it.
    let words = div()
        .absolute()
        .inset_0()
        .when(placement(slot) == Placement::OverPicture, |words| {
            words.pt(px(theme::BAND_ROOM))
        })
        .flex()
        .flex_col()
        .gap(px(theme::GAP))
        .items_center()
        .justify_center();
    let layer = div().absolute().inset_0();

    if let Showing::Starting { .. } = showing {
        return starting(slot, pane, text, words, layer).into_any_element();
    }

    // Something gone wrong sits still and is read, in the colour of a fault.
    let failed = matches!(showing, Showing::Failed(_));
    // Title-sized: this is the only thing in a pane that can be a thousand
    // pixels wide, and body text in the middle of it read as a caption on a
    // picture that had not arrived rather than as the pane's own state.
    let label = div()
        .text_size(px(theme::TEXT_TITLE))
        .text_color(if failed {
            theme::danger()
        } else {
            theme::text_dim()
        })
        .child(text);

    // Every state that is not going anywhere on its own gets this, and
    // until it existed the only way to ask again was to close the pane and
    // open the channel a second time. Beside `Watch from the start` it is
    // one choice among three, and drawn as one; anywhere else it is the
    // thing to do.
    let again = showing.next_step().map(|words| {
        let variant = if showing == Showing::Ended {
            Variant::Pill
        } else {
            Variant::Primary
        };
        act(
            slot,
            "retry",
            words,
            variant,
            PaneAction::Retry,
            &on_pane,
            cx,
        )
    });

    // Where the follows poll can start the pane by itself, it says so, and
    // can be told not to. A setting rather than something being waited for,
    // so it never breathes.
    let switch = pane.start_offered.then(|| {
        let key = slot.key.clone();
        let on_pane = on_pane.clone();
        Switch::new(pane_id(&slot.key, "start-live"))
            .checked(slot.start_when_live)
            .label("Start when they go live")
            .text_color(theme::text_muted())
            .on_click(cx.listener(move |view, checked: &bool, window, cx| {
                on_pane(view, &key, PaneAction::StartWhenLive(*checked), window, cx)
            }))
    });

    let body = match showing {
        Showing::Offline => {
            let last = pane
                .next
                .map(|next| last_broadcast(slot, next, room, window_hovered, cache, &on_pane, cx));
            words
                .child(label)
                .children(last.flatten())
                .children(switch)
                .children(again)
        }
        Showing::Ended => {
            let start = match from_start(pane.next, pane.looking) {
                FromStart::Offered(video) => Some(
                    act(
                        slot,
                        "from-start",
                        "Watch from the start",
                        Variant::Primary,
                        PaneAction::WatchFromStart(Box::new(video.clone())),
                        &on_pane,
                        cx,
                    )
                    .into_any_element(),
                ),
                FromStart::Looking => {
                    Some(controls::waiting("Watch from the start").into_any_element())
                }
                FromStart::Nothing => None,
            };
            // Close as well as the header's ×: an ended broadcast is the one
            // stop where going is as likely as staying.
            let close = act(
                slot,
                "end-close",
                "Close",
                Variant::Pill,
                PaneAction::Close,
                &on_pane,
                cx,
            );
            words
                .child(label)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .justify_center()
                        .gap(px(theme::GAP_TIGHT))
                        .children(start)
                        .children(again)
                        .child(close),
                )
                .children(switch)
        }
        // The way back, as the main button: the one thing to do about a
        // picture that is somewhere else.
        Showing::Elsewhere => words.child(label).child(act(
            slot,
            "pop-in",
            "Bring back",
            Variant::Primary,
            PaneAction::PopIn,
            &on_pane,
            cx,
        )),
        _ => words.child(label).children(again),
    };
    layer.child(body).into_any_element()
}

/// A starting pane: the picture it is waiting for, when there is one to
/// hand, dimmed under the channel's name and a line that breathes while it
/// waits — a stream takes a few seconds to arrive, and a motionless word is
/// indistinguishable from a hang.
fn starting(slot: &Slot, pane: &PaneInfo, line: SharedString, words: Div, layer: Div) -> Div {
    // Over a poster, full-strength text on the badges' dim, the one tier
    // that reads on it over the brightest picture there is. On black, the
    // quiet tier a pane with nothing in it has always used.
    let color = if pane.poster.is_some() {
        theme::text()
    } else {
        theme::text_dim()
    };
    layer
        .when_some(pane.poster.clone(), |layer, poster| {
            layer
                // A definite box, so the image's own aspect ratio sizes
                // nothing (the `img` trap); contained, the way the picture
                // will sit when it comes.
                .child(img(poster).absolute().size_full())
                .child(div().absolute().inset_0().bg(theme::overlay()))
        })
        .child(
            words
                .text_color(color)
                .child(
                    div()
                        .text_size(px(theme::TEXT_TITLE))
                        .font_weight(theme::weight_title())
                        .child(pane.name.clone()),
                )
                .child(motion::waiting(
                    pane_id(&slot.key, "status"),
                    div().text_size(px(theme::TEXT_BODY)).child(line),
                )),
        )
}

/// An offline channel's last broadcast, as the room allows: a card, or a
/// pill. Either plays it in this pane, from where it was left.
fn last_broadcast<V: 'static>(
    slot: &Slot,
    next: NextUp<'_>,
    room: NextUpRoom,
    window_hovered: bool,
    cache: &ImageCache,
    on_pane: &(impl Fn(&mut V, &str, PaneAction, &mut Window, &mut Context<V>) + Clone + 'static),
    cx: &mut Context<V>,
) -> Option<AnyElement> {
    let now = Utc::now();
    let byline = channel_page::describe(next.video, now);
    match room {
        NextUpRoom::Card(width) => {
            // The card speaks the browse page's actions; the one it can send
            // here is a click on it, and the pane hears that as itself.
            let key = slot.key.clone();
            let on_pane = on_pane.clone();
            let on_card =
                move |view: &mut V, action: Action, window: &mut Window, cx: &mut Context<V>| {
                    if let Action::WatchVideo(video) = action {
                        on_pane(view, &key, PaneAction::WatchHere(video), window, cx);
                    }
                };
            let card = Card {
                index: 0,
                video: next.video,
                watched: next.watched,
                byline,
                forgettable: false,
            };
            Some(channel_page::card(card, width, cache, false, now, on_card, cx).into_any_element())
        }
        NextUpRoom::Line => {
            let about = [SharedString::from(next.video.title.clone()), byline.into()];
            Some(
                act(
                    slot,
                    "last-broadcast",
                    "Watch the last broadcast",
                    Variant::Pill,
                    PaneAction::WatchHere(Box::new(next.video.clone())),
                    on_pane,
                    cx,
                )
                .when(window_hovered, |pill| {
                    pill.tooltip(controls::full_text(about))
                })
                .into_any_element(),
            )
        }
        NextUpRoom::Nothing => None,
    }
}

/// One of the screen's pills: `words` on it, `role` in its id, and `action`
/// asked of the root for this pane when it is pressed.
fn act<V: 'static>(
    slot: &Slot,
    role: &str,
    words: &'static str,
    variant: Variant,
    action: PaneAction,
    on_pane: &(impl Fn(&mut V, &str, PaneAction, &mut Window, &mut Context<V>) + Clone + 'static),
    cx: &mut Context<V>,
) -> Stateful<Div> {
    let key = slot.key.clone();
    let on_pane = on_pane.clone();
    controls::pill(pane_id(&slot.key, role), words, variant).on_click(
        cx.listener(move |view, _event, window, cx| {
            on_pane(view, &key, action.clone(), window, cx)
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::watch::tests::{live, recording};

    /// streamlink and mpv say the same things about a channel and a recording;
    /// what they mean depends on which the pane asked for.
    #[test]
    fn showing_reads_each_state_for_what_was_asked() {
        let mut channel = live();
        let mut video = recording(0.0);

        channel.set_state(StreamState::Offline);
        video.set_state(StreamState::Offline);
        assert_eq!(showing(&channel, false, false), Showing::Offline);
        assert_eq!(showing(&video, false, false), Showing::Unavailable);

        channel.set_state(StreamState::Ended);
        video.set_state(StreamState::Ended);
        assert_eq!(showing(&channel, false, false), Showing::Ended);
        assert_eq!(showing(&video, false, false), Showing::Finished);

        let reason = SharedString::from("streamlink is not installed");
        channel.set_state(StreamState::Failed(reason.clone()));
        assert_eq!(showing(&channel, false, false), Showing::Failed(&reason));
        assert_eq!(
            showing(&channel, false, false).sentence("Forsen"),
            Some(reason),
            "a failure is said in its own words"
        );
    }

    /// A recording picked up part-way says where, so a pane opening three
    /// hours in does not look like one starting from the top; under a second
    /// in is the top.
    #[test]
    fn a_recording_says_where_it_opens() {
        assert_eq!(
            showing(&recording(3723.0), false, false),
            Showing::Starting {
                recording: true,
                at: Some(3723.0),
            }
        );
        assert_eq!(
            showing(&recording(3723.0), false, false).sentence("Forsen"),
            Some("Opening at 1:02:03…".into())
        );
        assert_eq!(
            showing(&recording(0.5), false, false),
            Showing::Starting {
                recording: true,
                at: None,
            }
        );
        assert_eq!(
            showing(&live(), false, false),
            Showing::Starting {
                recording: false,
                at: None,
            },
            "a live stream has no moment to open at"
        );
    }

    /// A player whose first frame has not faded in is still the pane
    /// starting, saying what it said before the player came: where a
    /// recording opens, and the tile's `Starting…`.
    #[test]
    fn a_player_without_its_first_frame_still_reads_as_starting() {
        for pane in [live(), recording(3723.0)] {
            let starting = showing(&pane, false, false);
            assert_eq!(player(false, starting), starting);
            assert_eq!(player(false, starting).word(), Some("Starting…"));
        }
    }

    /// Once the picture covers the pane, the pane is the picture and says
    /// nothing over it.
    #[test]
    fn a_covered_player_is_the_picture() {
        for pane in [live(), recording(3723.0)] {
            let covered = player(true, showing(&pane, false, false));
            assert_eq!(covered, Showing::Picture);
            assert_eq!(covered.word(), None);
            assert_eq!(covered.sentence("Forsen"), None);
        }
    }

    /// A tile has a word for everything but a picture, and a pane a sentence;
    /// a pane with no picture always has something to say.
    #[test]
    fn every_state_but_the_picture_has_a_tile_word() {
        let reason = SharedString::from("no");
        let stopped = [
            Showing::Starting {
                recording: false,
                at: None,
            },
            Showing::Starting {
                recording: true,
                at: Some(60.0),
            },
            Showing::Offline,
            Showing::Unavailable,
            Showing::Ended,
            Showing::Finished,
            Showing::Failed(&reason),
            Showing::Elsewhere,
        ];
        for state in stopped {
            assert!(state.word().is_some(), "{state:?} has no tile word");
            assert!(state.sentence("Forsen").is_some(), "{state:?} says nothing");
        }
        assert_eq!(Showing::Picture.word(), None);
        assert_eq!(Showing::Picture.sentence("Forsen"), None);
    }

    /// A pane whose picture is in a window of its own says so, whatever its
    /// player is doing — starting, or playing and covering the pane — and
    /// offers the way back rather than asking again.
    #[test]
    fn a_popped_pane_reads_as_elsewhere() {
        for pane in [live(), recording(3723.0)] {
            for covered in [false, true] {
                let popped = showing(&pane, covered, true);
                assert_eq!(popped, Showing::Elsewhere, "covered {covered}");
                assert_eq!(
                    popped.sentence("Forsen"),
                    Some("Playing in its own window".into())
                );
                assert_eq!(popped.next_step(), None, "nothing to ask for again");
            }
        }
    }

    /// A pane the size of a window has room for the whole card, as wide as
    /// the browse grid ever draws one.
    #[test]
    fn a_large_box_gets_the_card() {
        assert_eq!(
            next_up_room(1280.0, 720.0),
            NextUpRoom::Card(browse::CARD_MAX)
        );
        match next_up_room(1280.0, 340.0) {
            NextUpRoom::Card(width) => assert!(
                (browse::CARD_MIN..browse::CARD_MAX).contains(&width),
                "a shorter pane narrows the card to fit, got {width}"
            ),
            other => panic!("a 340px pane fits a card, got {other:?}"),
        }
    }

    /// Too short for a card's picture, a pane still has a row for a pill.
    #[test]
    fn a_short_box_gets_the_line() {
        assert_eq!(next_up_room(800.0, 200.0), NextUpRoom::Line);
        assert_eq!(
            next_up_room(200.0, 720.0),
            NextUpRoom::Line,
            "too narrow for a card"
        );
    }

    /// A sliver of a pane keeps its words and its controls, and offers
    /// nothing else.
    #[test]
    fn a_tiny_box_gets_nothing() {
        assert_eq!(next_up_room(300.0, 120.0), NextUpRoom::Nothing);
    }

    /// With the ended broadcast's recording found, the pane offers it from
    /// the start, beside asking again.
    #[test]
    fn an_ended_pane_offers_its_broadcast_from_the_start() {
        let slot = recording(0.0);
        let video = slot.recording().unwrap();
        let next = NextUp {
            video,
            watched: None,
        };
        assert_eq!(from_start(Some(next), false), FromStart::Offered(video));
        assert_eq!(
            from_start(Some(next), true),
            FromStart::Offered(video),
            "an answer in hand outranks a second ask still out"
        );
        assert_eq!(
            Showing::Ended.sentence("Forsen"),
            Some("Forsen ended the stream".into())
        );
        assert_eq!(Showing::Ended.next_step(), Some("Try again"));
    }

    /// While the archives are being asked for, the control is there and not
    /// yet on offer.
    #[test]
    fn an_ended_pane_says_it_is_still_looking() {
        assert_eq!(from_start(None, true), FromStart::Looking);
    }

    /// Nothing found offers nothing from the start — never an older
    /// broadcast — and asking again is still there.
    #[test]
    fn an_ended_pane_with_no_archive_offers_only_asking_again() {
        assert_eq!(from_start(None, false), FromStart::Nothing);
        assert_eq!(Showing::Ended.next_step(), Some("Try again"));
    }
}
