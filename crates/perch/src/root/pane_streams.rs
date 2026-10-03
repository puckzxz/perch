//! What an open live pane's channel is doing when no list the app has
//! fetched carries it: a pane opened by name, from a link, or from the rail's
//! Recommended group. Those answers say no start, no title and no count, so
//! the pane had no timeline to rewind along (`rewind`) and a header with
//! nothing after the name. Asked of Helix by login (`Request::PaneStreams`)
//! for just those channels, and asked again every [`REASK`] while a pane
//! still plays one and still nothing else carries it, so its count and title
//! do not go stale.
//!
//! Only signed in, as every Helix ask is; signed out such a pane goes on
//! without, as it always did. The answers count as one more list in
//! `RootView::stream_info`, after every list the app fetches for its own
//! reasons.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::Context;
use twitch_api::LiveStream;

use super::RootView;
use crate::browse::SignIn;
use crate::twitch::Request;
use crate::watch::StreamState;

/// How long before a channel asked about is asked about again, while a pane
/// still plays it and nothing else carries it: what keeps its header's count
/// and title from going stale, at one request every two minutes for at most
/// four panes, beside the follows poll's one a minute.
const REASK: Duration = Duration::from_secs(120);

/// The streams asked for, and when each channel was last asked about.
#[derive(Default)]
pub(super) struct PaneStreams {
    pub(super) streams: Vec<LiveStream>,
    asked: HashMap<String, Instant>,
}

impl PaneStreams {
    /// Which of `uncovered` — the channels of live panes no fetched list
    /// carries — to ask about at `now`: those never asked about, and those
    /// asked about more than [`REASK`] ago.
    fn due(&self, uncovered: &[String], now: Instant) -> Vec<String> {
        uncovered
            .iter()
            .filter(|login| {
                self.asked
                    .get(login.as_str())
                    .is_none_or(|at| now.duration_since(*at) >= REASK)
            })
            .cloned()
            .collect()
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

    /// Forget every channel no live pane plays any more: what was kept for
    /// it, and when it was asked about, so a pane opened on it later asks
    /// afresh.
    fn keep(&mut self, open: &[String]) {
        self.streams
            .retain(|stream| open.contains(&stream.user_login));
        self.asked.retain(|login, _| open.contains(login));
    }
}

impl RootView {
    /// Ask about the channels of playing live panes that no fetched list
    /// carries, when they are due ([`PaneStreams::due`]); and forget the ones
    /// no pane plays any more.
    ///
    /// Called from `restage`, the funnel every change of a pane ends in, so
    /// a pane is asked about as it starts playing, and after every answer
    /// from the worker, which is what brings it round again every
    /// [`REASK`]. Cheap when nothing is due: a scan of at most four panes.
    /// An ask the worker could not take is not noted, so the next call tries
    /// again.
    pub(super) fn ask_pane_streams(&mut self) {
        let open: Vec<String> = self
            .slots
            .iter()
            .filter(|slot| slot.is_live() && matches!(slot.state, StreamState::Playing(_)))
            .map(|slot| slot.channel.clone())
            .collect();
        self.pane_streams.keep(&open);
        if !matches!(self.sign_in, SignIn::SignedIn(_)) {
            return;
        }
        let uncovered: Vec<String> = open
            .into_iter()
            .filter(|login| self.listed_stream(login).is_none())
            .collect();
        let now = Instant::now();
        let due = self.pane_streams.due(&uncovered, now);
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
    /// and changes nothing, and the next [`REASK`] asks again. What a pane
    /// does with an answer — its timeline's start, its header — follows at
    /// the end of the event, with every other answer (`sync_live_since`).
    pub(super) fn on_pane_streams(
        &mut self,
        logins: Vec<String>,
        result: Result<Vec<LiveStream>, String>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(streams) => self.pane_streams.answered(&logins, streams),
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
    }

    /// A channel no pane plays any more is forgotten, answer and ask alike,
    /// so a pane opened on it later asks at once.
    #[test]
    fn a_channel_no_pane_plays_is_forgotten() {
        let mut kept = PaneStreams::default();
        let start = Instant::now();
        kept.asked(&logins(&["wren", "kestrel"]), start);
        kept.answered(
            &logins(&["wren", "kestrel"]),
            vec![stream("wren", "on"), stream("kestrel", "on")],
        );
        kept.keep(&logins(&["kestrel"]));
        assert_eq!(kept.streams.len(), 1);
        assert_eq!(kept.due(&logins(&["wren"]), start), logins(&["wren"]));
        assert!(kept.due(&logins(&["kestrel"]), start).is_empty());
    }
}
