//! The Twitch worker's events: sign-in progress, who is live, who is
//! followed, and the answers to the browse page's requests. One pump, one
//! `match`, so every reply lands in the state it belongs to.

use std::collections::HashSet;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use gpui::{Context, Task, Window};
use twitch_api::{Channel, LiveStream, Video};

use super::{LinkedVideo, RootView, ToastAction};

/// A list of follows that the pointer can rest on, and hold still.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum LiveList {
    Rail,
    Following,
}

/// A list of follows as it stands on screen, brought up to date by a fresh
/// one without moving anybody: whoever is in both keeps their place, with the
/// fresh copy's numbers and titles; whoever is only on screen goes; whoever
/// is only in the fresh list joins the end, in the fresh list's order. `key`
/// says who is who — a login either way.
///
/// For while the pointer is over a list of them. Sorting by viewers every
/// minute swapped neighbours whose counts crossed, so a card or a rail row
/// could change under the pointer between aiming and clicking; and a channel
/// that went offline used to land in the middle of the offline names, pushing
/// down every name after it.
fn keep_order<T>(shown: &[T], fresh: Vec<T>, key: impl Fn(&T) -> &str) -> Vec<T> {
    let mut fresh: Vec<Option<T>> = fresh.into_iter().map(Some).collect();
    let mut kept = Vec::with_capacity(fresh.len());
    for old in shown {
        let still = fresh
            .iter_mut()
            .find(|entry| entry.as_ref().is_some_and(|entry| key(entry) == key(old)));
        if let Some(entry) = still.and_then(Option::take) {
            kept.push(entry);
        }
    }
    kept.extend(fresh.into_iter().flatten());
    kept
}

/// The offline follows once a fresh list of everyone followed has come in:
/// that list less whoever is `live`, in the order on screen while `held`,
/// and in the fresh list's name order otherwise.
///
/// Filtered against the live list rather than trusted: the two requests are
/// seconds apart, so somebody can go live between them and would otherwise
/// appear in both places at once.
fn offline_after(
    shown: &[Channel],
    fresh: Vec<Channel>,
    live: &HashSet<String>,
    held: bool,
) -> Vec<Channel> {
    let fresh: Vec<Channel> = fresh
        .into_iter()
        .filter(|channel| !live.contains(&channel.login))
        .collect();
    if held {
        keep_order(shown, fresh, |channel| channel.login.as_str())
    } else {
        fresh
    }
}
use crate::browse::{SearchResults, SignIn};
use crate::twitch::{ListKey, Request, TwitchEvent, TwitchService};

impl RootView {
    pub(super) fn spawn_twitch(
        settings_path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (TwitchService, Task<()>) {
        let (service, mut events) = TwitchService::start(settings_path);
        let pump = cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;
            while let Some(event) = events.next().await {
                if this
                    .update_in(cx, |this: &mut RootView, window, cx| {
                        this.apply_twitch_event(event, window, cx)
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        (service, pump)
    }

    pub(super) fn apply_twitch_event(
        &mut self,
        event: TwitchEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TwitchEvent::NeedsClientId => self.sign_in = SignIn::NeedsClientId,
            TwitchEvent::AwaitingCode {
                user_code,
                verification_uri,
            } => {
                self.sign_in = SignIn::AwaitingCode {
                    user_code: user_code.into(),
                    verification_uri: verification_uri.into(),
                }
            }
            TwitchEvent::SignedIn { login } => {
                self.sign_in = SignIn::SignedIn(login.into());
                // Whatever the user opened while signed out can be fetched now.
                self.fill_shown();
                self.request_linked_videos();
            }
            TwitchEvent::Streams(streams) => self.on_streams(streams, window, cx),
            TwitchEvent::FollowedChannels(channels) => {
                // Held still under the pointer like the live list, in the
                // rail and on the Following tab alike; see `hold_live`.
                self.offline =
                    offline_after(&self.offline, channels, &self.known_live, self.live_held());
                self.refreshing = false;
                self.follows_loaded = true;
                cx.notify();
            }
            TwitchEvent::Avatars(images) => {
                self.avatars.extend(images);
                cx.notify();
            }
            TwitchEvent::FollowsError(reason) => {
                // Only worth saying when somebody asked. The poll runs every
                // minute, and an outage that lasts an hour should not be sixty
                // toasts about a list that is still on screen.
                if self.refreshing {
                    self.toast(format!("could not refresh: {reason}"), cx);
                } else {
                    eprintln!("follows: {reason}");
                }
                self.refreshing = false;
                cx.notify();
            }
            TwitchEvent::Error(reason) => {
                // Terminal: the worker has returned, so no refresh it was
                // holding is ever going to be answered — and no browse
                // request either, including one it took off the queue and
                // dropped on the way out. Left pending, a list would pulse
                // "Loading…" for good, and `fill_shown` would never ask for it
                // again. The list on screen, if it is empty, then says why.
                self.refreshing = false;
                self.sign_in = SignIn::Error(reason.into());
                self.discovery.pending.clear();
                self.fill_shown();
            }

            // Each answer ends the wait for its own list and no other: a
            // reply for a list the user has left must not take "Loading…"
            // off the one they went to. See `Discovery::pending`.
            TwitchEvent::Popular(page) => {
                self.discovery
                    .popular
                    .absorb(page.items, page.next, page.append);
                self.discovery.finish(&ListKey::Popular);
            }
            TwitchEvent::Categories(page) => {
                self.discovery
                    .categories
                    .absorb(page.items, page.next, page.append);
                self.discovery.finish(&ListKey::Categories);
            }
            TwitchEvent::CategoryStreams { category, streams } => {
                // A reply for a category the user has already left must not
                // repopulate the page behind them.
                let still_open = self
                    .discovery
                    .open
                    .as_ref()
                    .is_some_and(|open| open.id == category.id);
                if still_open {
                    self.discovery
                        .streams
                        .absorb(streams.items, streams.next, streams.append);
                }
                self.discovery.finish(&ListKey::Category(category.id));
            }
            TwitchEvent::SearchResults {
                query,
                categories,
                streams,
                channels,
            } => {
                // Same guard as a category: an answer to a question the user
                // has moved on from must not replace what they are reading now.
                let current = self
                    .discovery
                    .search
                    .as_ref()
                    .is_some_and(|open| open.query.as_ref() == query);
                self.discovery.finish(&ListKey::Search(query.clone()));
                if current {
                    self.discovery.search = Some(SearchResults {
                        query: query.into(),
                        categories,
                        streams,
                        channels,
                    });
                }
            }
            TwitchEvent::Videos {
                login,
                user_id,
                kind,
                videos,
            } => {
                // Whatever the history holds of these, this is the newer
                // word on it, wherever the user has got to since.
                self.refresh_history(&videos.items, cx);
                // Same guard as a category: a reply for a channel the user
                // has already left must not repopulate the page behind them.
                if let Some(page) = self
                    .discovery
                    .channel
                    .as_mut()
                    .filter(|page| page.login == login)
                {
                    page.user_id = Some(user_id);
                    page.shelf_mut(kind)
                        .absorb(videos.items, videos.next, videos.append);
                }
                self.discovery.finish(&ListKey::Videos { login, kind });
            }
            TwitchEvent::Video { id, result } => self.on_linked_video(id, result, window, cx),
            // Said on the list that failed, which may not be the one on
            // screen by now; see `Discovery::shown_error`.
            TwitchEvent::BrowseError {
                list: Some(list),
                reason,
            } => {
                self.discovery.finish(&list);
                self.discovery.error = Some((list, reason.into()));
            }
            // Nothing the worker sends: every request that fails into a
            // `BrowseError` names its list. Kept out of the page all the same.
            TwitchEvent::BrowseError { list: None, reason } => eprintln!("browse: {reason}"),
        }
        cx.notify();
    }

    /// Take a fresh follows list and announce anyone who just came online.
    ///
    /// The first poll seeds the known set silently: on launch everyone is
    /// "newly" live, and eight toasts at once would be worse than none.
    pub(super) fn on_streams(
        &mut self,
        streams: Vec<LiveStream>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let now_live: HashSet<String> = streams.iter().map(|s| s.user_login.clone()).collect();

        if !self.known_live.is_empty() {
            let mut newly: Vec<&LiveStream> = streams
                .iter()
                .filter(|s| !self.known_live.contains(&s.user_login))
                .collect();
            newly.sort_by_key(|stream| std::cmp::Reverse(stream.viewer_count));
            for stream in newly {
                // A stream with no category set has an empty game, and joined
                // regardless the notice ended in a dangling " · ".
                let text = [
                    format!("{} went live", stream.display_name),
                    stream.game_name.clone(),
                ]
                .into_iter()
                .filter(|part| !part.trim().is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
                self.toast_with(
                    text,
                    Some(ToastAction::Watch(stream.user_login.clone())),
                    cx,
                );
            }
        }

        // A pane that found nothing to play, whose channel this poll lists as
        // broadcasting since then, tries again by itself: the channel was
        // opened before the stream started, or the broadcast dropped and came
        // back. `started_at` against when the pane stalled is what tells a
        // new broadcast from a list that is simply a minute behind the pane —
        // a stream that ended a moment ago is still on this list, and a retry
        // against it would only find it gone.
        let resumed: Vec<String> = self
            .slots
            .iter()
            .filter(|slot| slot.is_live())
            .filter_map(|slot| {
                let stalled = slot.stalled_at?;
                let stream = streams.iter().find(|s| s.user_login == slot.channel)?;
                let started = DateTime::parse_from_rfc3339(&stream.started_at).ok()?;
                (started.with_timezone(&Utc) > stalled).then(|| slot.key.clone())
            })
            .collect();
        for key in resumed {
            self.retry_stream(&key, window, cx);
        }

        // Anyone who just went live is no longer offline. The offline list
        // arrives from its own request moments later and will agree, but not
        // before a repaint that would show them in both lists.
        self.offline
            .retain(|channel| !now_live.contains(&channel.login));

        self.known_live = now_live;
        self.follows = if self.live_held() {
            keep_order(&self.follows, streams, |stream| stream.user_login.as_str())
        } else {
            streams
        };
        self.follows_loaded = true;
        cx.notify();
    }

    /// Whether the pointer is over a list of follows: the rail, or the
    /// Following tab. Each shows the offline follows as well as who is live,
    /// so either holds both.
    fn live_held(&self) -> bool {
        self.rail_pointed || self.following_pointed
    }

    /// Note whether the pointer is over one of the lists of follows, from
    /// that list's probe — or, for a list that is not on screen, from the page
    /// that is not drawing it, since a probe that is not painted says nothing.
    ///
    /// The lists hold still while pointed at, the way chat does: a poll that
    /// lands meanwhile updates them in place (see [`keep_order`]), the offline
    /// names as well as who is live. When the last of them is let go they are
    /// put back in order — who is live by viewers, the rest by name — out of
    /// the way of the pointer rather than under it.
    pub(super) fn hold_live(&mut self, list: LiveList, pointed: bool, cx: &mut Context<Self>) {
        let was = self.live_held();
        match list {
            LiveList::Rail => self.rail_pointed = pointed,
            LiveList::Following => self.following_pointed = pointed,
        }
        if was && !self.live_held() {
            twitch_api::by_viewers(&mut self.follows);
            twitch_api::by_name(&mut self.offline);
            cx.notify();
        }
    }

    /// Open a recording named by its link, once Twitch has said what it is.
    ///
    /// The link carries an id and a start time and nothing else the pane
    /// needs — no title, no channel — so the video is looked up first, which
    /// needs a session. Signed in, that is one request; still signing in, the
    /// link waits for it; with no sign-in coming, there is nothing to wait
    /// for and the toast says so.
    pub(super) fn open_video_link(
        &mut self,
        id: String,
        start_secs: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        if matches!(self.sign_in, SignIn::NeedsClientId | SignIn::Error(_)) {
            self.toast(format!("sign in to open recording {id} by its link"), cx);
            return;
        }
        if self.linked_videos.iter().any(|linked| linked.id == id) {
            return;
        }
        self.linked_videos.push(LinkedVideo {
            id,
            start_secs,
            requested: false,
        });
        self.request_linked_videos();
        cx.notify();
    }

    /// Ask the worker about every linked recording not yet asked about. Only
    /// once signed in: before that the request would sit behind the
    /// device-code poll, and `SignedIn` calls this again.
    fn request_linked_videos(&mut self) {
        if !matches!(self.sign_in, SignIn::SignedIn(_)) {
            return;
        }
        for linked in self
            .linked_videos
            .iter_mut()
            .filter(|linked| !linked.requested)
        {
            linked.requested = self.twitch.request(Request::Video {
                id: linked.id.clone(),
            });
        }
    }

    /// The worker's answer for a linked recording: the video, or why not.
    fn on_linked_video(
        &mut self,
        id: String,
        result: Result<Video, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.linked_videos.iter().position(|linked| linked.id == id) else {
            return;
        };
        let linked = self.linked_videos.remove(index);
        match result {
            Ok(video) => {
                // Alone if nothing is playing, beside it otherwise: the same
                // answer the command line gives a second channel.
                let solo = self.slots.is_empty();
                // Where the link points, if it points anywhere; otherwise
                // where it was left, as from any other way in.
                let start_at = linked
                    .start_secs
                    .map(|secs| secs as f64)
                    .unwrap_or_else(|| self.resume_point(&video.id));
                self.open_video_at(video, solo, start_at, window, cx);
            }
            Err(reason) => self.toast(format!("could not open recording {id}: {reason}"), cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(login: &str, viewers: u64) -> LiveStream {
        LiveStream {
            user_login: login.into(),
            user_id: String::new(),
            display_name: login.into(),
            title: String::new(),
            game_name: String::new(),
            viewer_count: viewers,
            thumbnail_url: String::new(),
            started_at: String::new(),
        }
    }

    fn logins(streams: &[LiveStream]) -> Vec<&str> {
        streams.iter().map(|s| s.user_login.as_str()).collect()
    }

    fn by_login(stream: &LiveStream) -> &str {
        &stream.user_login
    }

    fn channel(login: &str) -> Channel {
        Channel {
            login: login.into(),
            user_id: String::new(),
            display_name: login.into(),
        }
    }

    fn channel_logins(channels: &[Channel]) -> Vec<&str> {
        channels.iter().map(|c| c.login.as_str()).collect()
    }

    /// A poll whose counts would reorder the list leaves it where it stands,
    /// with the new counts.
    #[test]
    fn a_held_list_keeps_its_order_and_takes_the_new_numbers() {
        let shown = [stream("a", 300), stream("b", 200), stream("c", 100)];
        let fresh = vec![stream("c", 900), stream("b", 250), stream("a", 10)];

        let kept = keep_order(&shown, fresh, by_login);
        assert_eq!(logins(&kept), ["a", "b", "c"]);
        assert_eq!(kept[0].viewer_count, 10, "kept the old numbers");
        assert_eq!(kept[2].viewer_count, 900);
    }

    /// Whoever ended goes; whoever started joins the end, most-watched first,
    /// so nothing already on screen moves down to make room.
    #[test]
    fn a_held_list_drops_who_ended_and_adds_who_started_at_the_end() {
        let shown = [stream("a", 300), stream("b", 200), stream("c", 100)];
        let fresh = vec![
            stream("new_big", 5000),
            stream("c", 100),
            stream("a", 300),
            stream("new_small", 5),
        ];

        let kept = keep_order(&shown, fresh, by_login);
        assert_eq!(logins(&kept), ["a", "c", "new_big", "new_small"]);
    }

    /// Somebody whose stream just ended comes back in the fresh list among
    /// the names. Held, they join the end and nobody already on screen moves;
    /// let go, the list is the fresh one, in name order.
    #[test]
    fn offline_channels_keep_their_places_while_held() {
        let shown = [channel("alice"), channel("carol"), channel("erin")];
        let fresh = || {
            vec![
                channel("alice"),
                channel("bob"),
                channel("carol"),
                channel("erin"),
            ]
        };
        let nobody_live = HashSet::new();

        let held = offline_after(&shown, fresh(), &nobody_live, true);
        assert_eq!(channel_logins(&held), ["alice", "carol", "erin", "bob"]);

        let let_go = offline_after(&shown, fresh(), &nobody_live, false);
        assert_eq!(channel_logins(&let_go), ["alice", "bob", "carol", "erin"]);
    }

    /// Somebody who went live between the two requests is in the live list,
    /// and must not stay among the names as well — held or not.
    #[test]
    fn channels_that_went_live_leave_the_held_offline_list() {
        let shown = [channel("alice"), channel("bob"), channel("carol")];
        let fresh = || vec![channel("alice"), channel("bob"), channel("carol")];
        let live = HashSet::from(["bob".to_string()]);

        let held = offline_after(&shown, fresh(), &live, true);
        assert_eq!(channel_logins(&held), ["alice", "carol"]);

        let let_go = offline_after(&shown, fresh(), &live, false);
        assert_eq!(channel_logins(&let_go), ["alice", "carol"]);
    }
}
