//! What a stopped live pane asks about its channel: its newest past
//! broadcasts, for the pane to offer next — the last one, when the channel is
//! off, and the one that just ended, from its start. And whether the pane can
//! offer to start by itself when the channel comes back on.
//!
//! Asked through the worker's own request for it (`Request::Broadcasts`),
//! never a channel page's: a pane's answer must not land on a page's shelf,
//! nor its failure on a page. The answer lands in the pane's
//! `Slot::archives`, by the channel it was asked for — or, for a live pane
//! that asked to rewind, in its `Slot::rewind` (`root::rewind`), which rides
//! the same request.

use gpui::{Context, Window};
use twitch_api::Video;

use super::RootView;
use crate::browse::SignIn;
use crate::twitch::Request;
use crate::watch::{Lookup, StreamState};

impl RootView {
    /// Ask about the past broadcasts of the channel on the pane at `index`,
    /// when the pane is live and the ask can be answered.
    ///
    /// Only signed in, since Helix wants a token and the worker would hold
    /// the request behind its sign-in until then; `SignedIn` asks for every
    /// pane that missed out (`ask_missing`). Not again while an ask is out.
    /// Otherwise at every stop, even with an answer in hand: the channel may
    /// have broadcast since, while the pane sat stopped. That answer is what
    /// the pane goes on offering until the new one comes
    /// ([`Lookup::Refreshing`]), so a `Try again` that finds the channel
    /// still off never blanks the card it is showing.
    ///
    /// The channel's id, which Helix lists videos by, comes from whatever
    /// list knows it; failing all of them, the worker looks it up by name
    /// ([`request_broadcasts`](Self::request_broadcasts)).
    pub(super) fn ask_broadcasts(&mut self, index: usize) {
        let Some(slot) = self.slots.get(index) else {
            return;
        };
        if !slot.is_live()
            || !matches!(self.sign_in, SignIn::SignedIn(_))
            || slot.archives.waiting()
        {
            return;
        }
        let login = slot.channel.clone();
        if self.request_broadcasts(login) {
            self.slots[index].archives.asked();
        }
    }

    /// Send the worker the ask about `login`'s newest past broadcasts,
    /// reporting whether anyone is there to answer it: a stopped pane's
    /// ([`ask_broadcasts`](Self::ask_broadcasts)) and a live pane's rewind
    /// (`rewind`) alike, whose answers both come to
    /// [`on_broadcasts`](Self::on_broadcasts).
    pub(super) fn request_broadcasts(&mut self, login: String) -> bool {
        let user_id = self.known_user_id(&login);
        self.twitch.request(Request::Broadcasts { login, user_id })
    }

    /// `login`'s Helix id, from whatever list knows it: the live ones, the
    /// offline follows, or the channel page open on it. `None` when none
    /// does, and the worker looks it up by name. For the asks that Helix
    /// answers by id: a pane's past broadcasts, and a channel's schedule
    /// (`root::schedule`).
    pub(super) fn known_user_id(&self, login: &str) -> Option<String> {
        self.stream_info(login)
            .map(|stream| stream.user_id.clone())
            .or_else(|| {
                self.offline
                    .iter()
                    .find(|channel| channel.login == login)
                    .map(|channel| channel.user_id.clone())
            })
            .or_else(|| {
                self.discovery
                    .channel
                    .as_ref()
                    .filter(|page| page.login == login)
                    .and_then(|page| page.user_id.clone())
            })
            .filter(|id| !id.is_empty())
    }

    /// The worker's answer about `login`'s past broadcasts, for the live
    /// pane on that channel — ignored if the pane has gone since, or is no
    /// longer waiting on it. A pane that began to play again while the ask
    /// was out has forgotten it, and its answer is let go by when it comes:
    /// taken, it would stand in for the fresh ask its next stop makes, or a
    /// rewind's — an ended broadcast matched against a list from before it
    /// existed, or a live one told it has no archive. A failed ask leaves the
    /// pane's last answer standing, if it had one ([`Lookup::settle`]).
    ///
    /// The recordings are also the newer word on any the history holds, as a
    /// channel page's listing is.
    ///
    /// Answers are told apart by order, which the worker keeps. An answer to
    /// an ask the pane forgot when it began to play again is let go by
    /// ([`Slot::stray_answers`](crate::watch::Slot::stray_answers)), so it
    /// can stand in for no ask made since. After those, a live pane waiting
    /// on its rewind takes the answer first (`rewind`): a pane asks for its
    /// rewind only while it plays and for itself only once it has stopped,
    /// so a pane waiting on both — one that stopped with a rewind's ask out —
    /// asked for the rewind first, and the first answer is that one.
    pub(super) fn on_broadcasts(
        &mut self,
        login: String,
        result: Result<Vec<Video>, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let answer = match result {
            Ok(videos) => {
                self.refresh_history(&videos, cx);
                self.hear_muted(&videos);
                Some(videos)
            }
            Err(reason) => {
                eprintln!("broadcasts: {login}: {reason}");
                None
            }
        };
        if let Some(slot) = self
            .slots
            .iter_mut()
            .find(|slot| slot.is_live() && slot.channel == login && slot.stray_answers > 0)
        {
            slot.stray_answers -= 1;
        } else if let Some(index) = self
            .slots
            .iter()
            .position(|slot| slot.is_live() && slot.channel == login && slot.rewind.asking())
        {
            self.on_rewind_answer(index, answer.as_deref(), window, cx);
        } else if let Some(slot) = self
            .slots
            .iter_mut()
            .find(|slot| slot.is_live() && slot.channel == login && slot.archives.waiting())
        {
            slot.archives.settle(answer);
        }
        cx.notify();
    }

    /// Ask for every stopped live pane that has not been asked for: what
    /// sign-in landing calls, for panes that stopped while nobody was signed
    /// in, or while a new client id's worker was starting.
    pub(super) fn ask_missing(&mut self) {
        let missing: Vec<usize> = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| {
                slot.is_live()
                    && matches!(slot.state, StreamState::Offline | StreamState::Ended)
                    && slot.archives == Lookup::NotAsked
            })
            .map(|(index, _)| index)
            .collect();
        for index in missing {
            self.ask_broadcasts(index);
        }
    }

    /// Forget every ask still out, for a worker that will never answer it:
    /// one replaced by a new client id, whose answers die with it, or one that
    /// has stopped because sign-in failed. A pane left waiting would never
    /// ask again, and an ended one would go on saying it was looking. A first
    /// ask is as if never made, and the next sign-in asks it afresh
    /// (`ask_missing`); a repeat goes back to the answer it would have
    /// replaced ([`Lookup::forget`]).
    ///
    /// A live pane's ask for its rewind too, and the press waiting on it,
    /// which nothing will now carry out: the next press asks again. And the
    /// answers a pane was letting go by, which will not come either.
    ///
    /// The rail's ask for recommendations too, which is not a pane's but dies
    /// with the worker the same way, and would otherwise hold off every ask
    /// after it for good; see `Recommended::forget`. And the ask for when the
    /// offline follows were last live, for the same reason; see
    /// `LastLive::forget`. And the asks for Shared Chat partners' names,
    /// which would otherwise never be asked again; see
    /// `SourceRooms::forget`. And the asks for chat badges, likewise; see
    /// `chat_badges::Library::forget`. And the asks for channels'
    /// schedules, likewise; see `schedule::Schedules::forget`.
    pub(super) fn forget_asks(&mut self) {
        for slot in &mut self.slots {
            slot.archives.forget();
            slot.rewind.forget();
            slot.stray_answers = 0;
        }
        self.recommended.forget();
        self.last_live.forget();
        self.source_rooms.forget();
        self.badges.forget();
        self.schedules.forget();
    }

    /// Whether a pane on `channel` offers `Start when they go live`: only
    /// where the offer can be kept. What starts a pane by itself is the
    /// follows poll (`on_streams`), which runs only signed in and lists only
    /// channels you follow; a switch on any other pane would promise what
    /// nothing will do.
    pub(super) fn start_offered(&self, channel: &str) -> bool {
        matches!(self.sign_in, SignIn::SignedIn(_))
            && self.follows_loaded
            && (self
                .follows
                .iter()
                .any(|stream| stream.user_login == channel)
                || self
                    .offline
                    .iter()
                    .any(|followed| followed.login == channel))
    }
}
