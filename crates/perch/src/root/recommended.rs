//! The rail's Recommended group, as the root runs it: when to ask the worker
//! about the channels watched, and what its answer becomes on screen. What is
//! asked, when, and how the answers are put together is worked out, purely,
//! in `crate::recommended`; this is where that meets the panes, the follows,
//! the sign-in and the pointer.

use std::collections::HashSet;
use std::time::Instant;

use gpui::Context;
use settings::channel_key;

use super::follows::keep_order;
use super::RootView;
use crate::browse::SignIn;
use crate::recommended::{self, Suggestion};
use crate::twitch::{Recommendations, Request};

impl RootView {
    /// Bring the Recommended group up to date: rank what has been answered
    /// against what is followed and open now, and ask the worker about
    /// whatever is due.
    ///
    /// Called wherever any of that can change — a pane opened or closed (the
    /// seeds), the follows poll and the offline list (who is followed, and a
    /// minute gone by), sign-in landing, the rail unfolding, and an answer
    /// coming back — and cheap enough to be: whether an ask is due is
    /// `Recommended::next_ask`, which asks nothing when nothing has changed
    /// and the interval is not up, however often it is called. A place that
    /// changes the seeds and does not call this is caught by the next follows
    /// poll, a minute later at most.
    pub(super) fn update_recommended(&mut self) {
        self.rank_recommended();
        self.ask_recommended();
    }

    /// The seeds as they stand: what is open, then what was watched lately.
    /// A recording's pane counts as its channel, as it does for `recent`.
    fn recommendation_seeds(&self) -> Vec<String> {
        let open: Vec<&str> = self
            .slots
            .iter()
            .map(|slot| slot.channel.as_str())
            .collect();
        recommended::seeds(&open, &self.settings.recent)
    }

    /// Ask the worker about whatever seeds are due, if anything is.
    ///
    /// Only signed in, though the ask carries no token: the worker reads its
    /// requests only once sign-in has got it into its loop, so an ask sent
    /// before then would wait there, or die with a worker that never gets
    /// that far, and nothing else could go out meanwhile. `SignedIn` calls
    /// this again. And only while the group is on screen — the rail open, or
    /// the guide on its Recommended tab: nobody sees it otherwise, and
    /// unfolding the rail or showing that tab calls this. An ask the worker
    /// could not take is not noted as made, so the next chance tries again.
    fn ask_recommended(&mut self) {
        let seen = !self.settings.sidebar_collapsed || self.guide.shows_recommended();
        if !seen || !matches!(self.sign_in, SignIn::SignedIn(_)) {
            return;
        }
        let seeds = self.recommendation_seeds();
        let now = Instant::now();
        let Some(ask) = self.recommended.next_ask(&seeds, now) else {
            return;
        };
        if self.twitch.request(Request::Recommend {
            seeds: ask.seeds.clone(),
        }) {
            self.recommended.asked(ask, now);
        }
    }

    /// Work out what the group shows from the answers in hand: less everyone
    /// followed, live or not, and every channel open in a pane, each with the
    /// seed that led to it by the name it writes itself as.
    ///
    /// Nothing until a follows list has come back: without one, a channel
    /// you follow cannot be told from one you do not, and would be offered
    /// to you for the seconds the follows took. Held still while the pointer
    /// is on a list of follows, by the same probes as the follows themselves
    /// and their merge (see [`hold`]), so a fresh answer whose numbers would
    /// reorder the rows updates them where they stand, and a row just opened
    /// in a pane stays under the pointer that opened it; `hold_live` ranks
    /// them afresh when the pointer leaves.
    pub(super) fn rank_recommended(&mut self) {
        if !self.follows_loaded {
            self.recommended.shown.clear();
            return;
        }
        let seeds = self.recommendation_seeds();
        let exclude = self
            .follows
            .iter()
            .map(|stream| stream.user_login.as_str())
            .chain(self.offline.iter().map(|channel| channel.login.as_str()))
            .chain(self.slots.iter().map(|slot| slot.channel.as_str()));
        let fresh = self
            .recommended
            .suggestions(&seeds, exclude, |login| self.seed_name(login));
        self.recommended.shown = if self.live_held() {
            let open: HashSet<String> = self
                .slots
                .iter()
                .map(|slot| channel_key(&slot.channel))
                .collect();
            hold(&self.recommended.shown, fresh, |login| {
                open.contains(&channel_key(login))
            })
        } else {
            fresh
        };
    }

    /// The name a seed writes itself as, from whichever list knows it
    /// (`channel_name`), or from a recording open on it. `None` when none
    /// does, and the reason uses the login.
    fn seed_name(&self, login: &str) -> Option<String> {
        self.channel_name(login).map(str::to_string).or_else(|| {
            self.slots
                .iter()
                .filter_map(|slot| slot.recording())
                .find(|video| video.user_login == login)
                .map(|video| video.user_name.clone())
        })
    }

    /// The worker's answer to the ask that was out. Its failures go to the
    /// log and never to the screen: a refusal once, as the group goes for
    /// the session, and anything else each time, with the group left as it
    /// was. Then whatever came due meanwhile is asked — a seed added while
    /// this ask was out — and the group ranked again.
    pub(super) fn on_recommended(&mut self, result: Recommendations, cx: &mut Context<Self>) {
        if let Some(line) = self.recommended.answered(result) {
            eprintln!("recommended: {line}");
        }
        self.update_recommended();
        cx.notify();
    }
}

/// The group as it stands under the pointer, brought up to date by `fresh`
/// without moving anybody: `follows::keep_order`, as the follows hold — and a
/// row on screen that `fresh` has lost only because it is `open` in a pane
/// now stays where it stands, as it was.
///
/// A followed live row stays put when it is clicked and only takes the
/// watching highlight. A recommendation is left out of every fresh ranking
/// once it is open — it is a seed then, and the group leaves out what is
/// open — so without this the row clicked would go from under the pointer,
/// the next row would move up into its place, and the second click of a
/// double-click, or a second press of `+` there, would open a channel nobody
/// aimed at. Held, it takes the highlight like a follow; `hold_live` ranks it
/// away when the pointer leaves. A row gone from `fresh` for any other
/// reason — its channel ended, say — goes, as a follow that ends does.
fn hold(
    shown: &[Suggestion],
    mut fresh: Vec<Suggestion>,
    open: impl Fn(&str) -> bool,
) -> Vec<Suggestion> {
    let key = |suggestion: &Suggestion| channel_key(&suggestion.channel.login);
    let in_fresh: HashSet<String> = fresh.iter().map(key).collect();
    let opened: Vec<Suggestion> = shown
        .iter()
        .filter(|old| open(&old.channel.login) && !in_fresh.contains(&key(old)))
        .cloned()
        .collect();
    // Wherever they go in `fresh`, `keep_order` puts them back where they
    // stand, since each is on screen already.
    fresh.extend(opened);
    keep_order(shown, fresh, |suggestion| suggestion.channel.login.as_str())
}

#[cfg(test)]
mod tests {
    use twitch_api::recommend::SimilarChannel;

    use super::*;

    fn suggestion(login: &str, viewers: u64) -> Suggestion {
        Suggestion {
            channel: SimilarChannel {
                login: login.into(),
                user_id: String::new(),
                display_name: login.into(),
                title: String::new(),
                game_name: String::new(),
                game_id: String::new(),
                viewer_count: viewers,
                profile_image_url: String::new(),
                stream_id: String::new(),
            },
            reason: "Like forsen".into(),
        }
    }

    fn logins(suggestions: &[Suggestion]) -> Vec<&str> {
        suggestions
            .iter()
            .map(|suggestion| suggestion.channel.login.as_str())
            .collect()
    }

    /// The row just opened in a pane is missing from the fresh ranking, and
    /// stays where it stood rather than letting the row under it move up
    /// under the pointer; the rest keep their places and take the fresh
    /// numbers, and a newcomer joins the end.
    #[test]
    fn a_held_row_opened_in_a_pane_stays_under_the_pointer() {
        let shown = [
            suggestion("a", 300),
            suggestion("clicked", 200),
            suggestion("c", 100),
        ];
        let fresh = vec![
            suggestion("c", 900),
            suggestion("a", 10),
            suggestion("new", 5),
        ];

        let held = hold(&shown, fresh, |login| login.eq_ignore_ascii_case("CLICKED"));
        assert_eq!(logins(&held), ["a", "clicked", "c", "new"]);
        assert_eq!(held[0].channel.viewer_count, 10, "kept the old numbers");
        assert_eq!(held[1].channel.viewer_count, 200);
    }

    /// Only being open keeps a row the fresh ranking lost: one that ended
    /// goes, as a follow that ends does; and an open channel that was never
    /// on screen is not brought in.
    #[test]
    fn a_held_row_gone_for_any_other_reason_goes() {
        let shown = [suggestion("ended", 300), suggestion("b", 200)];
        let fresh = vec![suggestion("b", 250)];

        let held = hold(&shown, fresh, |login| login == "elsewhere");
        assert_eq!(logins(&held), ["b"]);
    }
}
