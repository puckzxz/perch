//! What a pane shows when it has no picture: one reading of the pane,
//! [`Showing`], and the screen drawn from it.
//!
//! One reading because two places say it, at two lengths: the pane, in a
//! sentence across the middle of where the picture would be, and the mini
//! player's tile, in a word. They used to read the slot separately — a
//! `status_message` here and a `state_word` in the mini player — so a new
//! state had two matches to land in, and nothing to say when it missed one.

use gpui::{div, prelude::*, px, AnyElement, Context, SharedString, Window};

use super::{pane_id, placement, PaneAction, Placement, Slot, StreamState};
use crate::{controls, motion, seek_bar, theme};

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
    /// Waiting on streamlink to find the stream. `at` is where a recording
    /// opens when that is not the top: picked up from the history, a link's
    /// moment, or a quality change part-way through.
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
}

/// How `slot` reads; see [`Showing`].
pub fn showing(slot: &Slot) -> Showing<'_> {
    let recording = !slot.is_live();
    match &slot.state {
        StreamState::Playing(_) => Showing::Picture,
        StreamState::Starting => Showing::Starting {
            recording,
            at: (recording && slot.resume_at >= 1.0).then_some(slot.resume_at),
        },
        StreamState::Offline if recording => Showing::Unavailable,
        StreamState::Offline => Showing::Offline,
        StreamState::Ended if recording => Showing::Finished,
        StreamState::Ended => Showing::Ended,
        StreamState::Failed(reason) => Showing::Failed(reason),
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
        })
    }

    /// Whether there is anything to do about it, and what the control says.
    ///
    /// Not the same as failing: a stream that ended is not a fault, and asking
    /// for it again is still the one useful move — a channel that dropped out
    /// comes back, and one that is really finished says so through the
    /// offline state. A recording that has finished offers to start over.
    fn next_step(&self) -> Option<&'static str> {
        match self {
            Showing::Picture | Showing::Starting { .. } => None,
            Showing::Finished => Some("Watch again"),
            Showing::Offline | Showing::Unavailable | Showing::Ended | Showing::Failed(_) => {
                Some("Try again")
            }
        }
    }
}

/// The pane with no picture: what it is showing, in a sentence, and the one
/// thing to do about it. Nothing at all for a pane that has its picture.
pub(super) fn screen<V: 'static>(
    slot: &Slot,
    name: &str,
    on_pane: impl Fn(&mut V, &str, PaneAction, &mut Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> AnyElement {
    let showing = showing(slot);
    let Some(text) = showing.sentence(name) else {
        return div().into_any_element();
    };
    // Something still happening and something gone wrong need opposite
    // treatment: one should look alive, the other should sit still and be
    // read.
    let working = matches!(showing, Showing::Starting { .. });
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

    let key = slot.key.clone();
    div()
        .size_full()
        // With no chat on screen the pane's header rests over the top of
        // this screen, which has no picture for it to keep clear of; the
        // words are centred in what it leaves rather than under it.
        .when(placement(slot) == Placement::OverPicture, |screen| {
            screen.pt(px(theme::BAND_ROOM))
        })
        .flex()
        .flex_col()
        .gap(px(theme::GAP))
        .items_center()
        .justify_center()
        // Only the states that are going somewhere breathe. A pulsing error
        // would be both irritating and a repaint that never stops.
        .child(if working {
            motion::waiting(pane_id(&slot.key, "status"), label).into_any_element()
        } else {
            label.into_any_element()
        })
        // Every state that is not going anywhere on its own gets this, and
        // until it existed the only way to ask again was to close the pane and
        // open the channel a second time.
        .when_some(showing.next_step(), |pane, again| {
            pane.child(
                controls::pill(
                    pane_id(&slot.key, "retry"),
                    again,
                    controls::Variant::Primary,
                )
                .on_click(cx.listener(move |view, _event, window, cx| {
                    on_pane(view, &key, PaneAction::Retry, window, cx)
                })),
            )
        })
        .into_any_element()
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
        assert_eq!(showing(&channel), Showing::Offline);
        assert_eq!(showing(&video), Showing::Unavailable);

        channel.set_state(StreamState::Ended);
        video.set_state(StreamState::Ended);
        assert_eq!(showing(&channel), Showing::Ended);
        assert_eq!(showing(&video), Showing::Finished);

        let reason = SharedString::from("streamlink is not installed");
        channel.set_state(StreamState::Failed(reason.clone()));
        assert_eq!(showing(&channel), Showing::Failed(&reason));
        assert_eq!(
            showing(&channel).sentence("Forsen"),
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
            showing(&recording(3723.0)),
            Showing::Starting {
                recording: true,
                at: Some(3723.0),
            }
        );
        assert_eq!(
            showing(&recording(3723.0)).sentence("Forsen"),
            Some("Opening at 1:02:03…".into())
        );
        assert_eq!(
            showing(&recording(0.5)),
            Showing::Starting {
                recording: true,
                at: None,
            }
        );
        assert_eq!(
            showing(&live()),
            Showing::Starting {
                recording: false,
                at: None,
            },
            "a live stream has no moment to open at"
        );
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
        ];
        for state in stopped {
            assert!(state.word().is_some(), "{state:?} has no tile word");
            assert!(state.sentence("Forsen").is_some(), "{state:?} says nothing");
        }
        assert_eq!(Showing::Picture.word(), None);
        assert_eq!(Showing::Picture.sentence("Forsen"), None);
    }
}
