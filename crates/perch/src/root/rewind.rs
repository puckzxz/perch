//! The root's side of rewinding a live pane: telling each live player when
//! its broadcast started, so its bar can draw the timeline; a press on that
//! timeline, carried out at once with the archive in hand or once the ask
//! for it is answered; and the archive opened in the pane's place. Then the
//! way back: telling each recording's player whether its broadcast is still
//! on, so its bar can offer `LIVE`, and the channel opened again in the
//! pane's place when it is pressed. The rules are `crate::rewind`'s.
//!
//! The ask is a stopped pane's ask (`Request::Broadcasts`, through
//! `request_broadcasts`), sent only when a press finds no archive in hand,
//! so nothing polls for it; and only signed in, since Helix wants a token.
//! Whether a recording's broadcast is still on asks nothing at all: it is
//! read off the live lists the app keeps anyway, as each answer brings them.

use chrono::{DateTime, Utc};
use gpui::{Context, Window};
use twitch_api::Video;

use super::RootView;
use crate::browse::SignIn;
use crate::rewind::{self, Listed, Moment, Origin, Press};
use crate::watch::{Slot, StreamState};

/// Said when Twitch lists no recording of the broadcast: the channel keeps
/// no past broadcasts, or has not listed this one yet.
const NO_ARCHIVE: &str = "No past broadcast to rewind into";

/// Said for a press while signed out, which has no token to ask with.
const SIGNED_OUT: &str = "Sign in to rewind";

/// Said when the ask itself failed; the reason is in the log.
const ASK_FAILED: &str = "Could not look for the past broadcast";

impl RootView {
    /// When the broadcast the live pane `slot` is showing began, from
    /// whichever list knows it (`live_info`): what its player's timeline
    /// runs from. `None` for a recording, and for a channel in no list that
    /// says, which then has no timeline.
    pub(super) fn live_since(&self, slot: &Slot) -> Option<DateTime<Utc>> {
        if !slot.is_live() {
            return None;
        }
        self.live_info(&slot.channel)
            .and_then(|info| info.started_at)
            .and_then(rewind::parse_start)
    }

    /// Tell every live player when its broadcast began, as the lists now
    /// say: after every answer from the worker, since any of them may have
    /// brought a list that knows a channel nothing knew when its picture
    /// arrived — a follows poll, a search. A player hears it only when it
    /// changes (`VideoView::set_live_since`), and a list that has since lost
    /// the channel takes nothing away: the start a player has is still its
    /// broadcast's.
    pub(super) fn sync_live_since(&mut self, cx: &mut Context<Self>) {
        let known: Vec<_> = self
            .slots
            .iter()
            .filter_map(|slot| Some((slot.video()?.clone(), self.live_since(slot)?)))
            .collect();
        for (view, since) in known {
            view.update(cx, |view, cx| view.set_live_since(Some(since), cx));
        }
    }

    /// Whether the recording pane `slot` is playing the archive of a
    /// broadcast its channel is still on (`rewind::back_to_live`), from what
    /// the lists say of the channel ([`listed`](Self::listed)) and what the
    /// pane knows by itself (`Slot::origin`). What offers the pane's bar
    /// `LIVE`, while the player also finds the archive still growing. False
    /// for a live pane.
    pub(super) fn still_live(&self, slot: &Slot) -> bool {
        let Some(video) = slot.recording() else {
            return false;
        };
        rewind::back_to_live(video, self.listed(&slot.channel), &slot.origin, Utc::now())
    }

    /// What the live lists say about `channel`, for
    /// [`still_live`](Self::still_live). Off when you follow it and the
    /// follows poll has it among the offline follows, whatever a list
    /// fetched once and kept — Popular, a category, a search — still says
    /// of the broadcast that has since ended. Otherwise whatever list
    /// carries it ([`stream_info`](Self::stream_info)), with the follows
    /// first.
    fn listed(&self, channel: &str) -> Listed<'_> {
        let followed_off = !self.known_live.contains(channel)
            && self
                .offline
                .iter()
                .any(|followed| followed.login == channel);
        if followed_off {
            return Listed::Off;
        }
        self.stream_info(channel)
            .and_then(|stream| {
                Some(Listed::On {
                    since: rewind::parse_start(&stream.started_at)?,
                    broadcast: Some(stream.id.as_str()).filter(|id| !id.is_empty()),
                })
            })
            .unwrap_or(Listed::Unknown)
    }

    /// Tell every recording's player whether its broadcast is still on
    /// ([`still_live`](Self::still_live)): after every answer from the
    /// worker, alongside [`sync_live_since`](Self::sync_live_since), since
    /// any of them may be the follows poll that has lost the channel, or a
    /// list that has found it. Unlike the start, a list that has lost the
    /// channel does take this away: `LIVE` on a broadcast that has ended
    /// would open a pane that says so. A player hears it only when it
    /// changes (`VideoView::set_back_to_live`).
    pub(super) fn sync_back_to_live(&mut self, cx: &mut Context<Self>) {
        let offers: Vec<_> = self
            .slots
            .iter()
            .filter(|slot| !slot.is_live())
            .filter_map(|slot| Some((slot.video()?.clone(), self.still_live(slot))))
            .collect();
        for (view, offered) in offers {
            view.update(cx, |view, cx| view.set_back_to_live(offered, cx));
        }
    }

    /// `LIVE` pressed on the bar of the recording pane at `index`: the
    /// channel opened again in its place, cold, as opening a channel is, so
    /// the pane is at the live edge with the live chat
    /// (`replace_with_channel`), and the recording's place goes into the
    /// history as it would on a close. If the channel is already open live
    /// in another pane, that one is chosen instead and the recording stays.
    ///
    /// Not checked against the lists again: the bar offered it on what they
    /// said at the last answer, and a channel that went off since opens to
    /// a pane that says it is offline, which is the news.
    pub(super) fn back_to_live(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let slot = &self.slots[index];
        if slot.is_live() {
            return;
        }
        let key = slot.key.clone();
        let channel = slot.channel.clone();
        self.replace_with_channel(&key, channel, window, cx);
    }

    /// A press let go on the timeline of the pane at `index`, asking for a
    /// `moment` of its broadcast: `PaneAction::Rewind`.
    ///
    /// With the archive in hand — found for the broadcast the press was on,
    /// which a restart since has not replaced ([`rewind::Rewind::press`]) —
    /// it opens at once. Without, it is asked for,
    /// and the press is carried out when the answer comes
    /// (`on_rewind_answer`); a press while the ask is out takes the place of
    /// the one before. Signed out there is nothing to ask with, which a
    /// toast says, and the pane stays live. Only for a live pane that is
    /// playing: the press came from its picture's bar, and a pane that has
    /// stopped since offers its broadcast its own way.
    pub(super) fn rewind(
        &mut self,
        index: usize,
        moment: Moment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let slot = &self.slots[index];
        if !slot.is_live() || !matches!(slot.state, StreamState::Playing(_)) {
            return;
        }
        let signed_in = matches!(self.sign_in, SignIn::SignedIn(_));
        let login = slot.channel.clone();
        let key = slot.key.clone();
        match self.slots[index].rewind.press(moment) {
            Press::Open(archive) => self.open_rewind(&key, &archive, moment, window, cx),
            Press::Wait => {}
            Press::Ask if !signed_in => {
                self.slots[index].rewind.unasked();
                self.toast(SIGNED_OUT, cx);
            }
            Press::Ask => {
                if self.request_broadcasts(login) {
                    self.slots[index].rewind.asked();
                } else {
                    // The worker has gone, which it does when sign-in
                    // fails: as good as signed out.
                    self.slots[index].rewind.unasked();
                    self.toast(SIGNED_OUT, cx);
                }
            }
        }
    }

    /// The worker's answer to the rewind ask of the pane at `index`: the
    /// channel's newest past broadcasts, or `None` for an ask that failed.
    ///
    /// The archive of the broadcast the waiting press was on is looked for
    /// among them (`rewind::archive_for`, by that broadcast's start, so the
    /// one before a restart is never it). It is kept for the next press, and
    /// the press is carried out with it — only while the
    /// pane is still playing live, since a pane that stopped meanwhile
    /// offers its broadcast its own way. No archive is a toast, and the pane
    /// stays live.
    pub(super) fn on_rewind_answer(
        &mut self,
        index: usize,
        answer: Option<&[Video]>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let slot = &mut self.slots[index];
        let found = answer
            .zip(slot.rewind.waiting())
            .and_then(|(videos, moment)| {
                rewind::archive_for(videos, slot.broadcast.as_deref(), moment.since, Utc::now())
            });
        let Some(moment) = slot.rewind.settle(found) else {
            return;
        };
        if !matches!(slot.state, StreamState::Playing(_)) {
            return;
        }
        match (slot.rewind.archive().cloned(), answer) {
            (Some(archive), _) => {
                let key = slot.key.clone();
                self.open_rewind(&key, &archive, moment, window, cx);
            }
            (None, Some(_)) => self.toast(NO_ARCHIVE, cx),
            (None, None) => self.toast(ASK_FAILED, cx),
        }
    }

    /// Play `archive` in the pane `key` names from the moment the press
    /// asked for, in its place, as `Watch from the start` does
    /// (`replace_with_video`): the pane becomes a recording, with its own
    /// seek bar and its chat replayed, and the history notes where it
    /// opened. The archive is opened as long as it is now (`rewind::as_of`),
    /// since one kept from an earlier press was listed shorter. The
    /// recording keeps the broadcast it was rewound from
    /// (`rewind::Origin::Rewound`), so its way back to live outlasts the
    /// list the channel was found in.
    fn open_rewind(
        &mut self,
        key: &str,
        archive: &Video,
        moment: Moment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let now = Utc::now();
        let start_at = rewind::position_in(archive, moment.at, now);
        let origin = Origin::Rewound {
            since: moment.since,
        };
        self.replace_with_video(
            key,
            rewind::as_of(archive, now),
            start_at,
            origin,
            window,
            cx,
        );
    }
}
