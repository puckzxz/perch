//! The Twitch worker's events: sign-in progress, who is live, who is
//! followed, and the answers to the browse page's requests. One pump, one
//! `match`, so every reply lands in the state it belongs to.

use std::collections::HashSet;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use gpui::{Context, Task, Window};
use twitch_api::{LiveStream, Video};

use super::{LinkedVideo, RootView, ToastAction};
use crate::browse::{SearchResults, SignIn};
use crate::twitch::{Request, TwitchEvent, TwitchService};

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
                self.fill_tab();
                self.request_linked_videos();
            }
            TwitchEvent::Streams(streams) => self.on_streams(streams, window, cx),
            TwitchEvent::FollowedChannels(channels) => {
                // Filtered against the live list rather than trusted: the two
                // requests are seconds apart, so somebody can go live between
                // them and would otherwise appear in both places at once.
                self.offline = channels
                    .into_iter()
                    .filter(|channel| !self.known_live.contains(&channel.login))
                    .collect();
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
                // holding is ever going to be answered.
                self.refreshing = false;
                self.sign_in = SignIn::Error(reason.into());
            }

            TwitchEvent::Popular(page) => {
                self.discovery
                    .popular
                    .absorb(page.items, page.next, page.append);
                self.discovery.loading = false;
            }
            TwitchEvent::Categories(page) => {
                self.discovery
                    .categories
                    .absorb(page.items, page.next, page.append);
                self.discovery.loading = false;
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
                self.discovery.loading = false;
            }
            TwitchEvent::SearchResults {
                query,
                categories,
                streams,
            } => {
                // Same guard as a category: an answer to a question the user
                // has moved on from must not replace what they are reading now.
                let current = self
                    .discovery
                    .search
                    .as_ref()
                    .is_some_and(|open| open.query.as_ref() == query);
                if current {
                    self.discovery.search = Some(SearchResults {
                        query: query.into(),
                        categories,
                        streams,
                    });
                }
                self.discovery.loading = false;
            }
            TwitchEvent::Videos {
                login,
                user_id,
                videos,
            } => {
                // Same guard as a category: a reply for a channel the user
                // has already left must not repopulate the page behind them.
                if let Some(page) = self
                    .discovery
                    .channel
                    .as_mut()
                    .filter(|page| page.login == login)
                {
                    page.user_id = Some(user_id);
                    page.videos.absorb(videos.items, videos.next, videos.append);
                }
                self.discovery.loading = false;
            }
            TwitchEvent::Video { id, result } => self.on_linked_video(id, result, window, cx),
            TwitchEvent::BrowseError(reason) => {
                self.discovery.error = Some(reason.into());
                self.discovery.loading = false;
            }
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
                self.toast_with(
                    format!("{} went live · {}", stream.display_name, stream.game_name),
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
        self.follows = streams;
        self.follows_loaded = true;
        cx.notify();
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
                let start_at = linked.start_secs.unwrap_or(0) as f64;
                self.open_video_at(video, solo, start_at, window, cx);
            }
            Err(reason) => self.toast(format!("could not open recording {id}: {reason}"), cx),
        }
    }
}
