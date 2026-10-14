//! What a channel is doing when nothing else the app asks would say, asked of
//! Helix by login (`Request::PaneStreams`) in one ask for all of them:
//!
//! - **A live pane starting or playing a channel no fetched list carries** —
//!   opened by name, from a link, or from the rail's Recommended group. Those
//!   answers say no start, no title and no count, so such a pane had no
//!   timeline to rewind along (`rewind`) and a header with nothing after the
//!   name. Asked while it starts, too, not only once it plays, so the answer
//!   can be the picture it waits on (`RootView::poster`) — but not while it
//!   starts on a channel you follow, which the follows poll lists the minute
//!   it is on: followed and not listed, it is off, and each `Try again` on
//!   it would be one more ask for nothing. What was asked is held while the
//!   pane starts or plays, whatever list comes to carry it
//!   ([`PaneStreams::keep`]), so a cold restart, which passes through
//!   starting, or a search closed over the pane, does not let go of it only
//!   to ask it again.
//! - **A stopped live pane — off, ended or failed — on a channel nobody
//!   follows.** Whether some list carries it does not matter here: only a
//!   list asked again can notice a channel come back on, and the one that
//!   is, the follows poll, lists followed channels alone. The answer starts
//!   such a pane by the rule the poll's answer does (`RootView::start_due`),
//!   so it offers `Start when they go live` too (`RootView::start_offered`);
//!   and an ended one still has here the stream it was playing when it
//!   ended, which is how it learns its broadcast's id
//!   (`RootView::stream_stopped`), as a followed one learns it from the
//!   poll's list.
//! - **A channel pinned to the rail that nobody follows**, which the rail
//!   would otherwise show as a bare login, live or not (`sidebar::groups`).
//!
//! The two that hang on who is followed wait for the list of everyone
//! followed (`RootView::followed_loaded`): before it, a channel followed and
//! off cannot be told from one nobody follows.
//!
//! One request a round, however many panes and pins: asked again every
//! [`REASK`] while anything is still wanted, and at once when something is
//! wanted that was not, every time for all of it together
//! ([`PaneStreams::due`]), never one ask each. Up to Helix's hundred logins
//! an ask, panes first ([`covered`]).
//!
//! Only signed in, as every Helix ask is; signed out such a pane goes on
//! without, as it always did. The answers are kept here and nowhere else —
//! never in the follows, which three things read as who is live — and count
//! as one more list in `RootView::stream_info`, after every list the app
//! fetches for its own reasons.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::{Context, Window};
use twitch_api::LiveStream;

use super::RootView;
use crate::browse::SignIn;
use crate::twitch::Request;
use crate::watch::{Slot, StreamState};

/// How long before what was asked is asked again, while any of it is still
/// wanted: what keeps a pane's header count and title from going stale, and
/// how soon a stopped pane on a channel nobody follows, or such a pin, is
/// seen to come on. One request every two minutes, for everything the ask
/// covers, beside the follows poll's one a minute.
const REASK: Duration = Duration::from_secs(120);

/// A live pane, as the ask reads it ([`Pane::of`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pane<'a> {
    /// Starting this channel: asked about while no list carries it and
    /// nobody is known to follow it.
    Starting(&'a str),
    /// Playing this channel: asked about while no list carries it.
    Playing(&'a str),
    /// Stopped on this channel — off, ended or failed: asked about while
    /// nobody follows it.
    Stopped(&'a str),
}

impl<'a> Pane<'a> {
    /// `slot` as the ask reads it, or `None` for a recording, which is
    /// never asked about as a pane.
    fn of(slot: &'a Slot) -> Option<Self> {
        if !slot.is_live() {
            return None;
        }
        Some(match slot.state {
            StreamState::Starting => Pane::Starting(&slot.channel),
            StreamState::Playing(_) => Pane::Playing(&slot.channel),
            StreamState::Offline | StreamState::Ended | StreamState::Failed(_) => {
                Pane::Stopped(&slot.channel)
            }
        })
    }

    /// The channel of a pane that starts or plays it, whose answer and ask
    /// are held while it does ([`PaneStreams::keep`]).
    fn running(self) -> Option<&'a str> {
        match self {
            Pane::Starting(channel) | Pane::Playing(channel) => Some(channel),
            Pane::Stopped(_) => None,
        }
    }
}

/// Whether the ask covers a stopped pane or a pin on `login`: once who is
/// followed is known (`followed`, `None` until the list of everyone followed
/// has come back), while nobody follows it. The one rule for [`covered`] and
/// for `RootView::start_offered`, so the switch on a stopped pane is offered
/// only where this ask, or the follows poll, will keep it.
fn unfollowed(login: &str, followed: Option<&impl Fn(&str) -> bool>) -> bool {
    followed.is_some_and(|followed| !followed(login))
}

/// Which channels the ask covers, each once: a playing pane's while no list
/// carries it (`listed`), and a starting one's while nobody is known to
/// follow it either; a stopped pane's and a pin's while nobody follows it
/// ([`unfollowed`]) — and those two not until the list of everyone followed
/// has come back (`followed` is `None` until then).
///
/// At most [`twitch_api::MAX_LOGINS_PER_REQUEST`], the most Helix answers in
/// one ask, which would otherwise drop the rest and have them read as off.
/// The panes come first, in their order, then the pins in theirs: a pane is
/// never the one left out, which is what lets every stopped pane on a
/// channel nobody follows offer `Start when they go live`
/// (`RootView::start_offered`) without asking whether it made the cut.
fn covered(
    panes: &[Pane<'_>],
    pinned: &[String],
    listed: impl Fn(&str) -> bool,
    followed: Option<impl Fn(&str) -> bool>,
) -> Vec<String> {
    let followed = followed.as_ref();
    let known_followed = |login: &str| followed.is_some_and(|followed| followed(login));
    let panes = panes.iter().filter_map(|pane| match *pane {
        Pane::Starting(channel) => {
            (!listed(channel) && !known_followed(channel)).then_some(channel)
        }
        Pane::Playing(channel) => (!listed(channel)).then_some(channel),
        Pane::Stopped(channel) => unfollowed(channel, followed).then_some(channel),
    });
    let pins = pinned
        .iter()
        .map(String::as_str)
        .filter(|&pin| unfollowed(pin, followed));
    let mut covered: Vec<String> = Vec::new();
    for login in panes.chain(pins) {
        if covered.len() == twitch_api::MAX_LOGINS_PER_REQUEST {
            break;
        }
        if !covered.iter().any(|kept| kept == login) {
            covered.push(login.to_string());
        }
    }
    covered
}

/// The streams asked for, and when each channel was last asked about.
#[derive(Default)]
pub(super) struct PaneStreams {
    pub(super) streams: Vec<LiveStream>,
    asked: HashMap<String, Instant>,
}

impl PaneStreams {
    /// What to ask about at `now`, of `wanted` ([`covered`]): all of it when
    /// any of it is due — never asked about, or asked about more than
    /// [`REASK`] ago — and nothing when none is. All of it rather than the
    /// part that is due, so everything wanted is asked about together and
    /// comes due together: asked as each came due, a pin made a minute
    /// after a pane was opened would be a second ask every round.
    fn due(&self, wanted: &[String], now: Instant) -> Vec<String> {
        let any = wanted.iter().any(|login| {
            self.asked
                .get(login.as_str())
                .is_none_or(|at| now.duration_since(*at) >= REASK)
        });
        if any {
            wanted.to_vec()
        } else {
            Vec::new()
        }
    }

    /// Note that `logins` were asked about at `now`.
    fn asked(&mut self, logins: &[String], now: Instant) {
        for login in logins {
            self.asked.insert(login.clone(), now);
        }
    }

    /// Take Helix's answer about `logins`: each one's stream replaces what
    /// was kept for it, and one asked about that the answer leaves out is
    /// off, so what was kept for it goes.
    fn answered(&mut self, logins: &[String], streams: Vec<LiveStream>) {
        self.streams
            .retain(|stream| !logins.contains(&stream.user_login));
        self.streams.extend(
            streams
                .into_iter()
                .filter(|stream| logins.contains(&stream.user_login)),
        );
    }

    /// The streams kept for `logins`: once they have just been answered
    /// ([`answered`](Self::answered)), who among them that answer found on.
    fn about(&self, logins: &[String]) -> Vec<LiveStream> {
        self.streams
            .iter()
            .filter(|stream| logins.contains(&stream.user_login))
            .cloned()
            .collect()
    }

    /// Forget every channel neither `wanted` ([`covered`]) nor `running` on
    /// a pane ([`Pane::running`]): what was kept for it, and when it was
    /// asked about, so it is asked about afresh if it is wanted again — and a
    /// pin or a stopped pane's channel that has come to be followed is read
    /// from the follows rather than from an answer that nothing asks again.
    ///
    /// A running pane's channel is held while a list carries it, as a
    /// playing one's always was: the list is read first (`stream_info`), and
    /// let go, the answer would be asked for again at once each time the list
    /// stopped carrying it — a search closed, a category left — and the
    /// pane's next cold restart would go through the same.
    fn keep(&mut self, wanted: &[String], running: &[&str]) {
        let held =
            |login: &str| wanted.iter().any(|want| want == login) || running.contains(&login);
        self.streams.retain(|stream| held(&stream.user_login));
        self.asked.retain(|login, _| held(login));
    }
}

impl RootView {
    /// Who is followed, as the ask reads it: `None` until the list of
    /// everyone followed has come back (`followed_loaded`), since before it
    /// a channel followed and off cannot be told from one nobody follows.
    ///
    /// Read off the two lists of follows (`RootView::followed`), which for a
    /// moment at each poll have neither of a follow whose stream has just
    /// ended: the live list has let it go and the list of everyone followed
    /// is still on its way. A pin or a stopped pane on such a channel is
    /// wanted for that moment, which costs one ask, answered after that list
    /// (the worker takes asks in turn) and let go as it lands: once each
    /// time a stream ends, while that list comes back. While it fails to,
    /// the channel stays wanted and is asked about every [`REASK`] until it
    /// does.
    fn followed_known(&self) -> Option<impl Fn(&str) -> bool + '_> {
        self.followed_loaded
            .then_some(|login: &str| self.followed(login))
    }

    /// Whether a live pane stopped on `channel` is in the ask ([`covered`],
    /// whose cap never leaves a pane out), by the rule the ask reads such a
    /// pane by ([`unfollowed`]): what `start_offered` asks of a channel the
    /// follows poll does not cover.
    pub(super) fn asks_after_stopped(&self, channel: &str) -> bool {
        unfollowed(channel, self.followed_known().as_ref())
    }

    /// Ask about the channels nothing else asks about ([`covered`]) when any
    /// of them is due ([`PaneStreams::due`]), all in one ask; and forget the
    /// ones no longer wanted and on no running pane ([`PaneStreams::keep`]).
    ///
    /// Called from `restage`, the funnel every change of a pane ends in, so
    /// a pane is asked about as it starts and as it stops; from
    /// `set_pinned`, so a pin is asked about as it is made; and after every
    /// answer from the worker, the follows' included, which is what brings it
    /// round again every [`REASK`] and what notices a channel come to be
    /// followed or not. Cheap when nothing is due: a scan of at most four
    /// panes and the pins. An ask the worker could not take is not noted, so
    /// the next call tries again.
    pub(super) fn ask_pane_streams(&mut self) {
        let panes: Vec<Pane> = self.slots.iter().filter_map(Pane::of).collect();
        let wanted = covered(
            &panes,
            &self.settings.pinned,
            |login| self.listed_stream(login).is_some(),
            self.followed_known(),
        );
        let running: Vec<&str> = panes.iter().filter_map(|pane| pane.running()).collect();
        self.pane_streams.keep(&wanted, &running);
        if !matches!(self.sign_in, SignIn::SignedIn(_)) {
            return;
        }
        let now = Instant::now();
        let due = self.pane_streams.due(&wanted, now);
        if due.is_empty() {
            return;
        }
        if self.twitch.request(Request::PaneStreams {
            logins: due.clone(),
        }) {
            self.pane_streams.asked(&due, now);
        }
    }

    /// The worker's answer about `logins`. A failure is one line in the log
    /// and changes nothing, and the next [`REASK`] asks again.
    ///
    /// A stopped pane on a channel the answer finds on starts by the rule
    /// the follows poll's answer starts one by (`start_due`): the same
    /// broadcast tried once, by the same note on the pane. What a playing
    /// pane does with an answer — its timeline's start, its header — follows
    /// at the end of the event, with every other answer
    /// (`sync_live_since`).
    pub(super) fn on_pane_streams(
        &mut self,
        logins: Vec<String>,
        result: Result<Vec<LiveStream>, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(streams) => {
                self.pane_streams.answered(&logins, streams);
                let live = self.pane_streams.about(&logins);
                self.start_due(&live, window, cx);
            }
            Err(reason) => eprintln!("pane streams: {reason}"),
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(login: &str, title: &str) -> LiveStream {
        LiveStream {
            id: String::new(),
            user_login: login.to_string(),
            user_id: String::new(),
            display_name: login.to_string(),
            title: title.to_string(),
            game_name: String::new(),
            viewer_count: 0,
            thumbnail_url: String::new(),
            started_at: "2026-10-03T00:00:00Z".to_string(),
        }
    }

    fn logins(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    /// A channel never asked about is due at once; one asked about is not
    /// due again until [`REASK`] has passed.
    #[test]
    fn a_channel_is_asked_about_once_per_reask() {
        let mut kept = PaneStreams::default();
        let start = Instant::now();
        let open = logins(&["wren", "kestrel"]);
        assert_eq!(kept.due(&open, start), open);
        kept.asked(&open, start);
        assert!(kept.due(&open, start + REASK / 2).is_empty());
        assert_eq!(kept.due(&open, start + REASK), open);
    }

    /// Something newly wanted is asked about at once, together with
    /// everything already asked about, so from then on all of it comes due
    /// together: one ask a round, not one for each channel on a clock of
    /// its own.
    #[test]
    fn everything_wanted_is_asked_about_in_one_ask() {
        let mut kept = PaneStreams::default();
        let start = Instant::now();
        kept.asked(&logins(&["wren"]), start);

        let later = start + REASK / 4;
        let wanted = logins(&["wren", "kestrel"]);
        assert_eq!(kept.due(&wanted, later), wanted, "both, in one ask");
        kept.asked(&wanted, later);

        assert!(
            kept.due(&wanted, start + REASK).is_empty(),
            "wren came due with kestrel, not on its own clock"
        );
        assert_eq!(kept.due(&wanted, later + REASK), wanted);
    }

    /// An answer replaces what was kept for the channels it was about,
    /// drops one it leaves out (which is off), and keeps no stream it was
    /// not asked about.
    #[test]
    fn an_answer_replaces_and_drops_what_it_was_about() {
        let mut kept = PaneStreams::default();
        kept.answered(
            &logins(&["wren", "kestrel"]),
            vec![stream("wren", "old"), stream("kestrel", "on")],
        );
        kept.answered(
            &logins(&["wren", "kestrel"]),
            vec![stream("wren", "new"), stream("stranger", "unasked")],
        );
        let titles: Vec<_> = kept
            .streams
            .iter()
            .map(|stream| (stream.user_login.as_str(), stream.title.as_str()))
            .collect();
        assert_eq!(titles, vec![("wren", "new")]);
        let about: Vec<_> = kept
            .about(&logins(&["wren", "kestrel"]))
            .into_iter()
            .map(|stream| stream.user_login)
            .collect();
        assert_eq!(about, ["wren"], "kestrel is off");
    }

    /// A channel no longer wanted is forgotten, answer and ask alike, so
    /// one wanted again later is asked about at once.
    #[test]
    fn a_channel_no_longer_wanted_is_forgotten() {
        let mut kept = PaneStreams::default();
        let start = Instant::now();
        kept.asked(&logins(&["wren", "kestrel"]), start);
        kept.answered(
            &logins(&["wren", "kestrel"]),
            vec![stream("wren", "on"), stream("kestrel", "on")],
        );
        kept.keep(&logins(&["kestrel"]), &[]);
        assert_eq!(kept.streams.len(), 1);
        assert_eq!(kept.due(&logins(&["wren"]), start), logins(&["wren"]));
        assert!(kept.due(&logins(&["kestrel"]), start).is_empty());
    }

    /// A running pane's channel is held, answer and ask, while a list
    /// carries it and so it is not wanted: when the list stops carrying it,
    /// it is not asked about again until its time comes round.
    #[test]
    fn a_running_panes_channel_is_held_while_a_list_carries_it() {
        let mut kept = PaneStreams::default();
        let start = Instant::now();
        let wren = logins(&["wren"]);
        kept.asked(&wren, start);
        kept.answered(&wren, vec![stream("wren", "on")]);

        kept.keep(&[], &["wren"]);
        assert_eq!(kept.streams.len(), 1);
        assert!(kept.due(&wren, start + REASK / 2).is_empty());
        assert_eq!(kept.due(&wren, start + REASK), wren);
    }

    /// Who is followed, for these tests: `owl` and `heron`, live or not.
    fn followed(login: &str) -> bool {
        matches!(login, "owl" | "heron")
    }

    /// Who is followed, before the list of everyone followed has come back.
    const UNKNOWN: Option<fn(&str) -> bool> = None;

    /// A playing pane is asked about while no list carries it, whoever
    /// follows it; a starting one while nobody is known to follow it
    /// either; a stopped one and a pin while nobody follows it, whatever
    /// list carries it — and those two not at all before the list of
    /// everyone followed has come back.
    #[test]
    fn the_ask_covers_what_nothing_else_asks_about() {
        let panes = [
            Pane::Playing("listed"),
            Pane::Playing("byname"),
            Pane::Playing("owl"),
            Pane::Starting("heron"),
            Pane::Starting("fresh"),
            Pane::Starting("listed"),
            Pane::Stopped("owl"),
            Pane::Stopped("stranger"),
            Pane::Stopped("listed"),
        ];
        let pinned = logins(&["owl", "faraway"]);
        let listed = |login: &str| login == "listed";

        let known = covered(&panes, &pinned, listed, Some(followed));
        assert_eq!(
            known,
            ["byname", "owl", "fresh", "stranger", "listed", "faraway"]
        );

        let unknown = covered(&panes, &pinned, listed, UNKNOWN);
        assert_eq!(
            unknown,
            ["byname", "owl", "heron", "fresh"],
            "the running panes alone"
        );
    }

    /// A stopped pane is in the ask exactly when the rule `start_offered`
    /// reads says so ([`unfollowed`]), so the switch is offered on a
    /// channel nobody follows only where this ask will keep it.
    #[test]
    fn a_stopped_pane_is_asked_about_by_the_rule_the_switch_reads() {
        for channel in ["owl", "stranger"] {
            let panes = [Pane::Stopped(channel)];
            for known in [Some(followed as fn(&str) -> bool), UNKNOWN] {
                let asked = covered(&panes, &[], |_| false, known).contains(&channel.to_string());
                assert_eq!(asked, unfollowed(channel, known.as_ref()), "{channel}");
            }
        }
    }

    /// A pin on a pane's channel is asked about once, as the pane.
    #[test]
    fn a_channel_is_in_the_ask_once() {
        let panes = [Pane::Playing("wren"), Pane::Stopped("wren")];
        let pinned = logins(&["wren"]);
        let asked = covered(&panes, &pinned, |_| false, Some(followed));
        assert_eq!(asked, ["wren"]);
    }

    /// Past Helix's cap the pins give way, never a pane: every stopped pane
    /// on a channel nobody follows is in the ask however many pins there
    /// are, which `start_offered` counts on.
    #[test]
    fn the_cap_leaves_out_pins_never_panes() {
        let stopped = ["a", "b", "c", "d"];
        let panes: Vec<Pane> = stopped.iter().map(|&login| Pane::Stopped(login)).collect();
        assert_eq!(panes.len(), crate::watch::MAX_PANES);
        let pinned: Vec<String> = (0..2 * twitch_api::MAX_LOGINS_PER_REQUEST)
            .map(|n| format!("pin{n}"))
            .collect();
        let asked = covered(&panes, &pinned, |_| false, Some(followed));
        assert_eq!(asked.len(), twitch_api::MAX_LOGINS_PER_REQUEST);
        assert_eq!(&asked[..stopped.len()], stopped);
        assert_eq!(asked[stopped.len()], "pin0");
    }
}
