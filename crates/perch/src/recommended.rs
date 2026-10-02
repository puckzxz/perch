//! The rail's Recommended group, worked out: which channels to ask Twitch
//! about, when to ask, and what its answers come to on screen. Pure and
//! tested. The asking is `root::recommended`, the request is the worker's
//! `Request::Recommend`, the drawing is `sidebar`, and the data is
//! `twitch_api::recommend`, whose docs say where it comes from.
//!
//! The seeds are the channels on screen and the ones watched lately
//! ([`seeds`]). For each one, Twitch's own sidebar query names the live
//! channels that channel's viewers also watch; `twitch_api::recommend::rank`
//! puts those shelves together, less every channel followed or open, and the
//! rail shows the first few, each with the seed that led to it ([`reason`]).
//!
//! That query is unpublished, and the stance on it is the one chat replay
//! takes: ask rarely, and when it breaks, go quietly. So an ask goes out when
//! there is a seed nobody has asked about, and otherwise once every
//! [`REFRESH`] at most, one at a time ([`Recommended::next_ask`]); a refusal
//! hides the group for the rest of the session; and any other failure keeps
//! the last good answers and is asked again at the next interval
//! ([`Recommended::answered`]). Nothing here is saved: a list of live
//! channels is stale by the next launch.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use settings::channel_key;
use twitch_api::recommend::{rank, SimilarChannel};

use crate::twitch::{RecommendError, Recommendations};

/// How many channels the recommendations are asked about.
///
/// Each seed is one anonymous request to Twitch's unpublished sidebar query,
/// and the worker makes them one after another while every browse request
/// waits behind them, so this is a few rather than all of
/// `settings::RECENT_LIMIT`. Six is four open panes with room for the last
/// couple of evenings besides; past that, a seed costs another request for a
/// shelf that mostly repeats the ones before it.
pub const SEED_LIMIT: usize = 6;

/// How many recommendations the rail shows.
///
/// The group sits between who is live and the offline fold, so every row in
/// it pushes the channels you follow further down for one you do not. Five
/// is one shelf's worth, which is what a single seed brings.
pub const SHOWN: usize = 5;

/// How long the answers stand before every seed is asked about again.
///
/// The shelves are live channels with viewer counts, so they go stale: within
/// minutes somebody on them has ended and the numbers have moved. Five
/// minutes keeps them about as fresh as the rest of the rail looks, at six
/// requests a time rather than six a minute — the follows poll's pace would
/// be an ask about every seed every minute of every session.
pub const REFRESH: Duration = Duration::from_secs(5 * 60);

/// The channels to ask about, in the order they matter: what is open in a
/// pane first, most recently watched first, then the rest of what was
/// watched recently, newest first. Each once, by `settings::channel_key`,
/// and at most [`SEED_LIMIT`].
///
/// `recent` is `Settings::recent`, newest first. The panes come first so a
/// channel on screen is a seed whatever the cap; opening one notes it in
/// `recent` too, so the two orders usually agree, and a pane `recent` has
/// lost track of goes after the ones it still has. The order is also the
/// order a recommendation's [`reason`] names its seeds in.
pub fn seeds<S: AsRef<str>>(open: &[S], recent: &[String]) -> Vec<String> {
    let place = |login: &str| {
        let key = channel_key(login);
        recent
            .iter()
            .position(|watched| channel_key(watched) == key)
            .unwrap_or(usize::MAX)
    };
    let mut open: Vec<&str> = open.iter().map(AsRef::as_ref).collect();
    // Stable, so panes `recent` does not have keep the grid's order.
    open.sort_by_key(|login| place(login));

    let mut seeds: Vec<String> = Vec::new();
    for login in open.into_iter().chain(recent.iter().map(String::as_str)) {
        if seeds.len() == SEED_LIMIT {
            break;
        }
        let key = channel_key(login.trim());
        if !key.is_empty() && !seeds.contains(&key) {
            seeds.push(key);
        }
    }
    seeds
}

/// Why a recommendation is offered, in the seeds' own names: "Like forsen",
/// "Like forsen and nymn", or "Like forsen and 2 more" — the way the mini
/// player names what is playing.
///
/// `because` is `Recommendation::because`, in seed order; `name_of` is the
/// name a seed writes itself as, where the app knows one, and the login
/// stands in where it does not. Empty for no seeds, which `rank` never
/// hands back.
pub fn reason(because: &[String], name_of: impl Fn(&str) -> Option<String>) -> String {
    let name = |login: &String| {
        name_of(login)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| login.clone())
    };
    match because {
        [] => String::new(),
        [one] => format!("Like {}", name(one)),
        [one, two] => format!("Like {} and {}", name(one), name(two)),
        [one, rest @ ..] => format!("Like {} and {} more", name(one), rest.len()),
    }
}

/// One row of the group: a live channel, and why it is offered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub channel: SimilarChannel,
    /// See [`reason`]. Worked out once, when the list is ranked, rather than
    /// on every frame the rail is drawn.
    pub reason: String,
}

/// One ask for the worker: which seeds, and whether it is the full one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub seeds: Vec<String>,
    /// Every seed, which starts the [`REFRESH`] interval again. Otherwise
    /// only the seeds not asked about since the last full ask.
    pub full: bool,
}

/// The asks out and the answers in, and what the rail shows from them. Lives
/// on the root for the session and is never saved.
#[derive(Debug, Default)]
pub struct Recommended {
    /// Each seed's last good answer, by seed. Kept through a failed ask, so a
    /// dropped request does not empty the group.
    answers: HashMap<String, Vec<SimilarChannel>>,
    /// When the last full ask went out. `None` before the first, and after a
    /// worker that will never answer has been forgotten.
    full_at: Option<Instant>,
    /// Every seed asked about since the last full ask, answered or not: a
    /// seed whose ask failed waits for the next full ask rather than being
    /// asked again at every chance until it works.
    tried: HashSet<String>,
    /// The ask the worker has and has not answered yet.
    out: Option<Ask>,
    /// Twitch would not run the query. Nothing more is asked this session.
    refused: bool,
    /// Every channel an answer has named this session, by
    /// `settings::channel_key`, as the newest answer to name it had it; see
    /// [`Recommended::channel`]. Kept apart from `answers` because a full
    /// answer replaces those, and the seed that led to a channel can drop out
    /// of the seeds while the channel is still open. Never pruned: the
    /// shelves repeat, so it is a few dozen channels an evening.
    known: HashMap<String, SimilarChannel>,
    /// What the rail shows: ranked, or held still under the pointer. Set by
    /// the root, which owns the hold; see `RootView::rank_recommended`.
    pub shown: Vec<Suggestion>,
}

impl Recommended {
    /// What to ask the worker now, if anything, given the seeds as they stand
    /// at `now`.
    ///
    /// Nothing while an ask is out, so there is one at a time and an answer
    /// for an older set of seeds is waited for and used rather than raced;
    /// nothing with no seeds; and nothing ever again once Twitch has refused.
    /// Otherwise every seed when the last full ask is [`REFRESH`] old or
    /// there has been none, and failing that only the seeds nobody has asked
    /// about since — a pane opened on a new channel is one request, not six.
    /// A seed that has only moved, or gone, asks nothing. So the follows poll
    /// can call this every minute and it asks at most once in five.
    pub fn next_ask(&self, seeds: &[String], now: Instant) -> Option<Ask> {
        if self.refused || self.out.is_some() || seeds.is_empty() {
            return None;
        }
        let due = self
            .full_at
            .is_none_or(|at| now.saturating_duration_since(at) >= REFRESH);
        if due {
            return Some(Ask {
                seeds: seeds.to_vec(),
                full: true,
            });
        }
        let new: Vec<String> = seeds
            .iter()
            .filter(|seed| !self.tried.contains(*seed))
            .cloned()
            .collect();
        (!new.is_empty()).then_some(Ask {
            seeds: new,
            full: false,
        })
    }

    /// Note that `ask` has gone to the worker, at `now`. Only once it has:
    /// an ask nobody could take is not one to wait on.
    pub fn asked(&mut self, ask: Ask, now: Instant) {
        if ask.full {
            self.full_at = Some(now);
            self.tried = ask.seeds.iter().cloned().collect();
        } else {
            self.tried.extend(ask.seeds.iter().cloned());
        }
        self.out = Some(ask);
    }

    /// Take the worker's answer to the ask that was out, and say what is
    /// worth a line in the log, if anything.
    ///
    /// A full answer replaces every seed's, so a seed no longer asked about
    /// stops counting; a partial one adds to them. Either way every channel
    /// named is noted for [`Recommended::channel`]. A refusal is the end:
    /// the answers go, the group with them, and the line says so once — a
    /// later refusal, from an ask already out, says nothing. What the answers
    /// said about the channels they named stays, for the panes open on them.
    /// Any other failure keeps the answers there are and says why, and the
    /// next full ask is when it is tried again.
    pub fn answered(&mut self, result: Recommendations) -> Option<String> {
        let ask = self.out.take();
        match result {
            Ok(answers) => {
                if ask.is_some_and(|ask| ask.full) {
                    self.answers.clear();
                }
                for channel in answers.iter().flat_map(|(_, similar)| similar) {
                    self.known
                        .insert(channel_key(&channel.login), channel.clone());
                }
                self.answers.extend(answers);
                None
            }
            Err(RecommendError::Refused(message)) => {
                if self.refused {
                    return None;
                }
                self.refused = true;
                self.answers.clear();
                self.shown.clear();
                Some(format!(
                    "Twitch would not run the query ({message}); \
                     no recommendations this session"
                ))
            }
            Err(RecommendError::Failed(message)) => Some(message),
        }
    }

    /// Forget the ask that was out, for a worker that will never answer it:
    /// one replaced by a new client id, or one stopped because sign-in
    /// failed. The interval starts over, so the next worker's first chance
    /// asks about every seed; the answers in hand stay on screen until then,
    /// and a refusal stays a refusal.
    pub fn forget(&mut self) {
        self.out = None;
        self.full_at = None;
        self.tried.clear();
    }

    /// Whether Twitch refused the query, which ends the recommendations for
    /// the session: the guide's Recommended tab says so, rather than that
    /// they are on their way.
    pub fn refused(&self) -> bool {
        self.refused
    }

    /// What the answers last said about `login`, in any case, whether or not
    /// the group shows it now: its name as it writes it, its title, game and
    /// viewers. `None` for a channel no answer has named this session.
    ///
    /// For a pane opened from the group. Once it is open the channel is a
    /// seed, and the group leaves it out; and no Helix list the root holds
    /// has it, since it is not followed. Without this the pane would call
    /// its channel by its login and say nothing about what is on, while the
    /// row it came from said both. A snapshot, as every list the root reads
    /// is: as fresh as the last answer that named the channel.
    pub fn channel(&self, login: &str) -> Option<&SimilarChannel> {
        self.known.get(&channel_key(login))
    }

    /// What the answers come to for `seeds` as they stand: their shelves put
    /// together in seed order by `twitch_api::recommend::rank`, less
    /// `exclude` and the seeds themselves, at most [`SHOWN`], each with its
    /// [`reason`].
    ///
    /// Only the current seeds count, so a channel that has dropped out of
    /// them takes its shelf with it, though its answer is kept in case it
    /// comes back before the next full ask. Every seed is left out, not only
    /// those with an answer, which are all `rank` knows of: a seed whose ask
    /// is still out, or failed, is a channel watched lately all the same.
    pub fn suggestions<S: AsRef<str>>(
        &self,
        seeds: &[String],
        exclude: impl IntoIterator<Item = S>,
        name_of: impl Fn(&str) -> Option<String>,
    ) -> Vec<Suggestion> {
        let per_seed: Vec<(String, Vec<SimilarChannel>)> = seeds
            .iter()
            .filter_map(|seed| {
                self.answers
                    .get(seed)
                    .map(|similar| (seed.clone(), similar.clone()))
            })
            .collect();
        let exclude: Vec<String> = exclude
            .into_iter()
            .map(|login| login.as_ref().to_string())
            .chain(seeds.iter().cloned())
            .collect();
        rank(&per_seed, exclude, SHOWN)
            .into_iter()
            .map(|found| Suggestion {
                reason: reason(&found.because, &name_of),
                channel: found.channel,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn logins(list: &[&str]) -> Vec<String> {
        list.iter().map(|login| login.to_string()).collect()
    }

    fn channel(login: &str, viewers: u64) -> SimilarChannel {
        SimilarChannel {
            login: login.into(),
            user_id: String::new(),
            display_name: login.into(),
            title: String::new(),
            game_name: String::new(),
            game_id: String::new(),
            viewer_count: viewers,
            profile_image_url: String::new(),
            stream_id: String::new(),
        }
    }

    fn shown(suggestions: &[Suggestion]) -> Vec<&str> {
        suggestions
            .iter()
            .map(|suggestion| suggestion.channel.login.as_str())
            .collect()
    }

    fn no_names(_: &str) -> Option<String> {
        None
    }

    /// What is open comes first, in the order it was last watched, then the
    /// rest of the recent list: each channel once, whatever case it was
    /// written in, with the `#` an IRC-minded command line may give it gone.
    #[test]
    fn seeds_lead_with_the_panes_and_name_each_channel_once() {
        let recent = logins(&["xqc", "forsen", "nymn", "lirik"]);
        let open = ["Nymn", "#forsen"];
        assert_eq!(seeds(&open, &recent), ["forsen", "nymn", "xqc", "lirik"]);

        // A pane the recent list has lost track of goes after the ones it
        // has, still ahead of everything only watched.
        let open = ["gone_from_recent", "lirik"];
        assert_eq!(
            seeds(&open, &recent),
            ["lirik", "gone_from_recent", "xqc", "forsen", "nymn"]
        );

        assert!(seeds::<&str>(&[], &[]).is_empty());
        assert!(seeds(&["", "  "], &[]).is_empty());
    }

    /// One request per seed, so the cap holds whatever is open and however
    /// much was watched — and the panes are what survive it.
    #[test]
    fn seeds_stop_at_the_cap_and_keep_the_panes() {
        let recent = logins(&["a", "b", "c", "d", "e", "f", "g", "h"]);
        let open = ["h", "g"];
        let found = seeds(&open, &recent);
        assert_eq!(found.len(), SEED_LIMIT);
        assert_eq!(found, ["g", "h", "a", "b", "c", "d"]);
    }

    fn at(base: Instant, secs: u64) -> Instant {
        base + Duration::from_secs(secs)
    }

    /// The first chance asks about everything; then nothing until the
    /// interval is up, however often it is asked, and everything again once
    /// it is.
    #[test]
    fn every_seed_is_asked_at_first_and_then_once_an_interval() {
        let base = Instant::now();
        let seeds = logins(&["forsen", "nymn"]);
        let mut state = Recommended::default();

        let first = state.next_ask(&seeds, base).expect("never asked");
        assert_eq!(
            first,
            Ask {
                seeds: seeds.clone(),
                full: true
            }
        );
        state.asked(first, base);
        assert_eq!(state.next_ask(&seeds, at(base, 1)), None, "one at a time");

        assert_eq!(state.answered(Ok(vec![])), None);
        for secs in [1, 60, 120, REFRESH.as_secs() - 1] {
            assert_eq!(state.next_ask(&seeds, at(base, secs)), None, "{secs}s on");
        }
        let again = state.next_ask(&seeds, at(base, REFRESH.as_secs()));
        assert_eq!(again, Some(Ask { seeds, full: true }));
    }

    /// A new seed is asked about at once, and alone; a seed that only moved
    /// or went asks nothing. An answer for an older set of seeds is waited
    /// for, and the new seed is asked after it rather than beside it.
    #[test]
    fn a_new_seed_is_asked_about_alone_and_after_the_ask_that_is_out() {
        let base = Instant::now();
        let mut state = Recommended::default();
        let before = logins(&["forsen", "nymn"]);
        let ask = state.next_ask(&before, base).unwrap();
        state.asked(ask, base);

        let after = logins(&["xqc", "forsen", "nymn"]);
        assert_eq!(state.next_ask(&after, at(base, 2)), None, "an ask is out");
        state.answered(Ok(vec![("forsen".into(), vec![channel("a", 1)])]));
        let new = state.next_ask(&after, at(base, 3)).unwrap();
        assert_eq!(
            new,
            Ask {
                seeds: logins(&["xqc"]),
                full: false
            }
        );
        state.asked(new, at(base, 3));
        state.answered(Ok(vec![]));

        let moved = logins(&["nymn", "xqc"]);
        assert_eq!(state.next_ask(&moved, at(base, 4)), None);
        assert_eq!(state.next_ask(&[], at(base, 4)), None, "no seeds");
        // A partial ask does not restart the interval.
        assert!(state
            .next_ask(&moved, at(base, REFRESH.as_secs()))
            .is_some_and(|ask| ask.full));
    }

    /// A failure keeps the answers there were and waits for the interval; a
    /// refusal empties the group and is the last ask of the session, and is
    /// said once.
    #[test]
    fn a_failure_waits_for_the_interval_and_a_refusal_ends_it() {
        let base = Instant::now();
        let seeds = logins(&["forsen"]);
        let mut state = Recommended::default();
        state.asked(state.next_ask(&seeds, base).unwrap(), base);
        state.answered(Ok(vec![("forsen".into(), vec![channel("a", 1)])]));

        state.asked(
            state.next_ask(&seeds, at(base, REFRESH.as_secs())).unwrap(),
            at(base, REFRESH.as_secs()),
        );
        let failed = state.answered(Err(RecommendError::Failed("timed out".into())));
        assert!(failed.is_some_and(|line| line.contains("timed out")));
        assert_eq!(shown(&state.suggestions(&seeds, [""; 0], no_names)), ["a"]);
        assert_eq!(
            state.next_ask(&seeds, at(base, REFRESH.as_secs() + 60)),
            None
        );
        // A new seed whose ask fails is not asked again until then either.
        let more = logins(&["forsen", "nymn"]);
        let ask = state
            .next_ask(&more, at(base, REFRESH.as_secs() + 61))
            .unwrap();
        state.asked(ask, at(base, REFRESH.as_secs() + 61));
        state.answered(Err(RecommendError::Failed("timed out".into())));
        assert_eq!(
            state.next_ask(&more, at(base, REFRESH.as_secs() + 62)),
            None
        );

        let ask = state
            .next_ask(&more, at(base, 2 * REFRESH.as_secs()))
            .unwrap();
        state.asked(ask, at(base, 2 * REFRESH.as_secs()));
        state.shown = state.suggestions(&more, [""; 0], no_names);
        let refused = state.answered(Err(RecommendError::Refused(
            "PersistedQueryNotFound".into(),
        )));
        assert!(refused.is_some_and(|line| line.contains("PersistedQueryNotFound")));
        assert!(state.refused());
        assert!(state.shown.is_empty());
        assert!(state.suggestions(&more, [""; 0], no_names).is_empty());
        assert_eq!(
            state.next_ask(&more, at(base, 10 * REFRESH.as_secs())),
            None
        );
        assert_eq!(
            state.answered(Err(RecommendError::Refused("again".into()))),
            None,
            "a refusal is logged once"
        );
    }

    /// A worker that will never answer is forgotten: the next chance asks
    /// about everything again, and the answers in hand stay meanwhile.
    #[test]
    fn a_forgotten_ask_starts_the_interval_over() {
        let base = Instant::now();
        let seeds = logins(&["forsen"]);
        let mut state = Recommended::default();
        state.asked(state.next_ask(&seeds, base).unwrap(), base);
        state.answered(Ok(vec![("forsen".into(), vec![channel("a", 1)])]));
        state.asked(
            state
                .next_ask(&logins(&["forsen", "nymn"]), at(base, 1))
                .unwrap(),
            at(base, 1),
        );

        state.forget();
        let ask = state.next_ask(&seeds, at(base, 2));
        assert_eq!(
            ask,
            Some(Ask {
                seeds: seeds.clone(),
                full: true
            })
        );
        assert_eq!(shown(&state.suggestions(&seeds, [""; 0], no_names)), ["a"]);
    }

    /// A full answer replaces the old ones, so a seed that was not asked
    /// about this time stops counting; a partial one adds to them.
    #[test]
    fn a_full_answer_replaces_and_a_partial_one_adds() {
        let base = Instant::now();
        let mut state = Recommended::default();
        let both = logins(&["forsen", "nymn"]);
        state.asked(state.next_ask(&both, base).unwrap(), base);
        state.answered(Ok(vec![
            ("forsen".into(), vec![channel("from_forsen", 1)]),
            ("nymn".into(), vec![channel("from_nymn", 1)]),
        ]));

        let more = logins(&["forsen", "nymn", "xqc"]);
        state.asked(state.next_ask(&more, at(base, 1)).unwrap(), at(base, 1));
        state.answered(Ok(vec![("xqc".into(), vec![channel("from_xqc", 1)])]));
        let found = state.suggestions(&more, [""; 0], no_names);
        let mut all = shown(&found);
        all.sort();
        assert_eq!(all, ["from_forsen", "from_nymn", "from_xqc"]);

        let one = logins(&["forsen"]);
        let full = at(base, REFRESH.as_secs());
        state.asked(state.next_ask(&one, full).unwrap(), full);
        state.answered(Ok(vec![("forsen".into(), vec![channel("from_forsen", 1)])]));
        assert_eq!(
            shown(&state.suggestions(&more, [""; 0], no_names)),
            ["from_forsen"],
            "the answers from before the full ask were kept"
        );
    }

    /// A channel an answer named is known by its own name, in any case, once
    /// the group has let it go — a pane open on it, which makes it a seed —
    /// and after a full answer that no longer names it, and a refusal. The
    /// newest answer to name it is the one that counts.
    #[test]
    fn a_channel_named_once_stays_known_after_the_group_lets_it_go() {
        let base = Instant::now();
        let mut state = Recommended::default();
        let named = |login: &str, name: &str, viewers| SimilarChannel {
            display_name: name.into(),
            ..channel(login, viewers)
        };
        assert_eq!(state.channel("admiralbulldog"), None);

        let seeds = logins(&["forsen"]);
        state.asked(state.next_ask(&seeds, base).unwrap(), base);
        state.answered(Ok(vec![(
            "forsen".into(),
            vec![named("admiralbulldog", "AdmiralBulldog", 10)],
        )]));
        let opened = logins(&["admiralbulldog", "forsen"]);
        assert!(state.suggestions(&opened, [""; 0], no_names).is_empty());
        let known = state.channel("AdmiralBulldog").expect("named by an answer");
        assert_eq!(known.display_name, "AdmiralBulldog");

        state.asked(state.next_ask(&opened, at(base, 1)).unwrap(), at(base, 1));
        state.answered(Ok(vec![(
            "admiralbulldog".into(),
            vec![named("forsen", "Forsen", 20), named("other", "Other", 5)],
        )]));
        let full = at(base, REFRESH.as_secs());
        state.asked(state.next_ask(&seeds, full).unwrap(), full);
        state.answered(Ok(vec![(
            "forsen".into(),
            vec![named("other", "OTHER", 7)],
        )]));
        assert_eq!(
            state.channel("admiralbulldog").map(|c| c.viewer_count),
            Some(10),
            "a full answer that does not name it forgot it"
        );
        assert_eq!(
            state.channel("other").map(|c| c.display_name.as_str()),
            Some("OTHER")
        );

        let later = at(base, 2 * REFRESH.as_secs());
        state.asked(state.next_ask(&seeds, later).unwrap(), later);
        state.answered(Err(RecommendError::Refused("gone".into())));
        assert!(
            state.channel("admiralbulldog").is_some(),
            "a refusal forgot it"
        );
    }

    /// The followed and the open are left out, and so are the seeds, in any
    /// case; what several seeds agree on comes first; the cap is kept; and
    /// only the current seeds' shelves count.
    #[test]
    fn suggestions_leave_out_the_followed_and_the_open() {
        let base = Instant::now();
        let mut state = Recommended::default();
        let seeds = logins(&["forsen", "nymn"]);
        state.asked(state.next_ask(&seeds, base).unwrap(), base);
        state.answered(Ok(vec![
            (
                "forsen".into(),
                vec![
                    channel("nymn", 9000),
                    channel("followed", 800),
                    channel("shared", 5),
                    channel("open_pane", 700),
                    channel("big", 600),
                ],
            ),
            (
                "nymn".into(),
                (0..6)
                    .map(|n| channel(&format!("small{n}"), n))
                    .chain([channel("shared", 5)])
                    .collect(),
            ),
        ]));

        let exclude = ["FOLLOWED", "open_pane"];
        let found = state.suggestions(&seeds, exclude, no_names);
        assert_eq!(
            shown(&found),
            ["shared", "big", "small5", "small4", "small3"]
        );
        assert_eq!(found.len(), SHOWN);
        assert_eq!(found[0].reason, "Like forsen and nymn");

        let only_nymn = logins(&["nymn"]);
        let found = state.suggestions(&only_nymn, exclude, no_names);
        assert!(
            !shown(&found).contains(&"big"),
            "a seed that went still counted"
        );

        // A seed whose own ask has not been answered yet is left out all the
        // same: `rank` knows only the seeds with an answer.
        let unanswered = logins(&["forsen", "nymn", "Big"]);
        let found = state.suggestions(&unanswered, exclude, no_names);
        assert!(
            !shown(&found).contains(&"big"),
            "a seed with no answer yet was recommended"
        );
    }

    /// One seed, two, and more, each by the name it writes itself as where
    /// the app knows one and by its login where it does not.
    #[test]
    fn the_reason_names_the_seeds() {
        let names = |login: &str| match login {
            "maxylobes" => Some("Maxylobes".to_string()),
            "blank" => Some("  ".to_string()),
            _ => None,
        };
        assert_eq!(reason(&logins(&["maxylobes"]), names), "Like Maxylobes");
        assert_eq!(reason(&logins(&["forsen"]), names), "Like forsen");
        assert_eq!(
            reason(&logins(&["maxylobes", "forsen"]), names),
            "Like Maxylobes and forsen"
        );
        assert_eq!(
            reason(&logins(&["forsen", "a", "b"]), names),
            "Like forsen and 2 more"
        );
        assert_eq!(reason(&logins(&["blank"]), names), "Like blank");
        assert_eq!(reason(&[], names), "");
    }
}
