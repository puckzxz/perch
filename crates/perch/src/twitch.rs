//! The Twitch worker: signing in, keeping the follows list fresh, and
//! answering the browse page's requests, a stopped pane's, the rail's ask
//! for channels like the ones watched, when the offline follows were last
//! live, whose chat a Shared Chat line was copied from, and what the badges
//! in a chat look like.
//!
//! One thread owns the session, and it has to. Refresh tokens are single-use,
//! so two things refreshing at once would spend the same token twice and lock
//! the user out. Everything that reads Helix goes through here for that reason,
//! not merely for tidiness. The rail's ask reads no Helix and carries no token
//! (see [`Request::Recommend`]), and goes through here anyway, so what the app
//! asks of Helix and of the rail's query is all in one place. Chat is not:
//! its IRC, its history and a recording's replay — a GraphQL ask too — are
//! the chat side's, and never come through here.
//!
//! The thread alternates between a follows poll on a timer and whatever the UI
//! asks for in between, which is what the request channel is: `recv_timeout`
//! against the next poll deadline is both the wait and the mailbox.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::channel::mpsc;
use settings::{OAuthTokens, Settings};
use twitch_api::badges::BadgeSet;
use twitch_api::recommend::{LastBroadcast, SimilarChannel};
use twitch_api::{Category, Channel, LiveStream, Session, Video, VideoKind};

/// How often to re-ask Twitch who is live.
const POLL_INTERVAL: Duration = Duration::from_secs(60);

/// How much to slow the device-code poll by each time Twitch says `slow_down`.
/// Five seconds is what RFC 8628 §3.5 specifies, not a number we picked.
const SLOW_DOWN_STEP: Duration = Duration::from_secs(5);

/// How many of a channel's newest past broadcasts a stopped pane asks for.
/// The newest is what an offline pane offers, and the one that has just
/// ended is among the first few even when a reconnect split it in two; five
/// is room for that without asking for the shelf's hundred.
const BROADCASTS_ASKED: u8 = 5;

/// Something the browse page, or a pane, wants fetched.
#[derive(Debug, Clone)]
pub enum Request {
    /// Who is live and who is followed, now rather than at the next poll.
    ///
    /// Answered in [`run`] rather than in [`serve`], because it is the one
    /// request that has to move the timer — see the call site.
    Follows,
    /// The most-watched streams overall. `after` continues an existing list
    /// rather than starting one — see [`twitch_api::Page`].
    Popular { after: Option<String> },
    /// The categories with the most viewers.
    Categories { after: Option<String> },
    /// The most-watched streams inside one category.
    Category {
        category: Category,
        after: Option<String>,
    },
    /// Categories and channels matching a name, live and not.
    Search(String),
    /// One kind of a channel's videos — past broadcasts, highlights or
    /// uploads. `user_id` is what Helix lists them by; a channel that arrived
    /// with a name alone has it looked up first.
    Videos {
        login: String,
        user_id: Option<String>,
        kind: VideoKind,
        after: Option<String>,
    },
    /// One recording, by the id a link carries.
    Video { id: String },
    /// A channel's newest few past broadcasts, for a live pane that has
    /// stopped: the last one, to offer when the channel is off, and the one
    /// that just ended, to watch from its start. `user_id` as for
    /// [`Videos`](Request::Videos).
    ///
    /// Not `Videos`, though it reads the same endpoint. That fills a channel's
    /// page, and its answer would land on the page's shelf and end the page's
    /// wait; its failure would be said on the page. A pane's question is none
    /// of the page's business, so it fills no list and its failure travels in
    /// its answer.
    Broadcasts {
        login: String,
        user_id: Option<String>,
    },
    /// The live channels whose viewers also watch each of `seeds`, for the
    /// rail's Recommended group: one request per seed, in order, to Twitch's
    /// unpublished sidebar query (`twitch_api::recommend::similar_channels`).
    /// The root decides which seeds and how often; see `crate::recommended`.
    ///
    /// Anonymous: it carries no token and asks nothing of the session, so it
    /// is answered in [`run`] ahead of the session's upkeep, like
    /// [`Follows`](Request::Follows) — a refresh Twitch turned down ends the
    /// worker, and an ask that needs no session should not be what finds that
    /// out. It is still only read once the worker is in its loop, after
    /// sign-in, which is why the root asks it only signed in. It fills no
    /// browse list and touches no follows, and its failure travels in its
    /// answer.
    Recommend { seeds: Vec<String> },
    /// When each of `logins` last went live, for the words under an offline
    /// follow's name on Home and in the rail: up to a hundred logins a
    /// request to Twitch's unpublished GraphQL endpoint
    /// (`twitch_api::recommend::last_broadcasts`). The root decides which
    /// logins and how often; see `crate::last_live`.
    ///
    /// Anonymous, and answered in [`run`] ahead of the session's upkeep, for
    /// the reasons [`Recommend`](Request::Recommend) is. It fills no browse
    /// list, and its failure travels in its answer.
    LastLive { logins: Vec<String> },
    /// The channels behind these numeric ids, for the label on a line a
    /// chat pane was sent from a partner's chat in a Shared Chat session:
    /// Twitch's unpublished GraphQL endpoint again
    /// (`twitch_api::recommend::channel_names`). The root asks once per id a
    /// session; see `crate::shared_chat`.
    ///
    /// Anonymous, and answered in [`run`] ahead of the session's upkeep, for
    /// the reasons [`Recommend`](Request::Recommend) is. It fills no browse
    /// list, and its failure travels in its answer.
    ChannelNames { ids: Vec<String> },
    /// The chat badges every channel shares (`channel: None`), or one
    /// channel's own by its numeric id: Helix's Get Global Chat Badges and
    /// Get Channel Chat Badges (`twitch_api::badges`). The root asks for the
    /// global ones once a session and each channel's once, as chats meet
    /// them; see `crate::chat_badges`. Helix, with the token, so it goes
    /// through [`serve`] like the browse page's asks; it fills no browse
    /// list, and its failure travels in its answer.
    Badges { channel: Option<String> },
}

/// Which browse list a request fills, so its answer — or its failure — can
/// be told apart from another list's.
///
/// Not the request itself: `after` is left out, because a page of a list and
/// the list's first page fill the same list, and the page waits on it either
/// way. Two requests for one list are two of the same key, and each answer
/// takes one away; see `Discovery::finish`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListKey {
    Popular,
    Categories,
    /// A category's streams, by the category's id.
    Category(String),
    /// What a search found, by the query as it was sent.
    Search(String),
    /// One kind of one channel's videos — one shelf of its page.
    Videos {
        login: String,
        kind: VideoKind,
    },
}

impl Request {
    /// The browse list this fills, or `None` for the seven that fill none:
    /// the follows poll, whose lists are not the browse page's, a recording
    /// looked up for a link, whose failure is a toast, a pane's past
    /// broadcasts, which are the pane's, the rail's recommendations, which
    /// are the rail's, when the offline follows were last live, which is
    /// words on names already on screen, and the names of Shared Chat
    /// partners and the chat badges, which are the chats'.
    pub fn list_key(&self) -> Option<ListKey> {
        match self {
            Request::Follows
            | Request::Video { .. }
            | Request::Broadcasts { .. }
            | Request::Recommend { .. }
            | Request::LastLive { .. }
            | Request::ChannelNames { .. }
            | Request::Badges { .. } => None,
            Request::Popular { .. } => Some(ListKey::Popular),
            Request::Categories { .. } => Some(ListKey::Categories),
            Request::Category { category, .. } => Some(ListKey::Category(category.id.clone())),
            Request::Search(query) => Some(ListKey::Search(query.clone())),
            Request::Videos { login, kind, .. } => Some(ListKey::Videos {
                login: login.clone(),
                kind: *kind,
            }),
        }
    }
}

/// A page of a browse list, and what the UI should do with it.
///
/// `append` rather than letting the receiver work it out: a reply carries no
/// memory of the request that asked for it, and "was this a Load more or a
/// fresh tab" is exactly the difference between adding a hundred rows and
/// replacing them.
#[derive(Debug, Clone)]
pub struct Listing<T> {
    pub items: Vec<T>,
    pub next: Option<String>,
    pub append: bool,
}

impl<T> Listing<T> {
    fn from(page: twitch_api::Page<T>, append: bool) -> Self {
        Self {
            items: page.items,
            next: page.next,
            append,
        }
    }
}

#[derive(Debug, Clone)]
pub enum TwitchEvent {
    /// No client id in settings, so sign-in cannot even begin.
    NeedsClientId,
    /// Show this code and URL; the user types it at twitch.tv/activate.
    AwaitingCode {
        user_code: String,
        verification_uri: String,
    },
    SignedIn {
        login: String,
    },
    Streams(Vec<LiveStream>),
    /// Everyone the user follows, live or not. A separate list from
    /// [`Streams`](TwitchEvent::Streams) all the way to the screen — see
    /// [`Channel`] for why merging them would be wrong three times
    /// over.
    FollowedChannels(Vec<Channel>),
    /// Avatars for whoever is live, as `(login, url)`. Arrives after the live
    /// list it belongs to and is merged into what the UI already holds, so a
    /// rail that is already on screen fills in rather than blinking.
    Avatars(Vec<(String, String)>),
    /// A follows poll failed with a session that is otherwise fine — a network
    /// blip, or Twitch having a moment. Deliberately not
    /// [`Error`](TwitchEvent::Error), which the UI reads as "signed out" and
    /// which would blank the whole page over one dropped request.
    FollowsError(String),
    Popular(Listing<LiveStream>),
    Categories(Listing<Category>),
    CategoryStreams {
        category: Category,
        streams: Listing<LiveStream>,
    },
    SearchResults {
        query: String,
        categories: Vec<Category>,
        streams: Vec<LiveStream>,
        /// Channels that answered to the name and are not live: names, the
        /// way offline follows are.
        channels: Vec<Channel>,
    },
    /// A page of one kind of a channel's videos, with the id they were listed
    /// by so the next page need not look it up again.
    Videos {
        login: String,
        user_id: String,
        kind: VideoKind,
        videos: Listing<Video>,
    },
    /// The recording a link named, or why it could not be had. Its own
    /// event rather than a `BrowseError`, because the link was not a browse
    /// page's question and its failure belongs in a toast, not on the page.
    Video {
        id: String,
        result: Result<Video, String>,
    },
    /// A channel's newest past broadcasts, newest first, for the pane on
    /// its live stream — or why they could not be had. Its own event, with
    /// the failure inside it, for the reason [`Video`](TwitchEvent::Video)
    /// has one: a `BrowseError` would end a browse list's wait, and this was
    /// never a browse list's question.
    Broadcasts {
        login: String,
        result: Result<Vec<Video>, String>,
    },
    /// Each seed of a [`Request::Recommend`], in the order asked, with the
    /// live channels Twitch says its viewers also watch — or why not. Its own
    /// event with the failure inside it, for the reason
    /// [`Broadcasts`](TwitchEvent::Broadcasts) has one.
    Recommended(Recommendations),
    /// When each login of a [`Request::LastLive`] last went live, keyed by
    /// login as Twitch writes it, or why not. A login Twitch has no account
    /// for is simply absent. Its own event with the failure inside it, for
    /// the reason [`Broadcasts`](TwitchEvent::Broadcasts) has one; the
    /// failure is a [`RecommendError`] because it is the same endpoint, and
    /// a refusal means the same: stop asking.
    LastLive(Result<HashMap<String, LastBroadcast>, RecommendError>),
    /// The channels behind a [`Request::ChannelNames`]'s ids, or why not,
    /// with the ids asked, so a failure can be put down against them. An id
    /// Twitch has no account for is simply absent. Its own event with the
    /// failure inside it, for the reason [`Broadcasts`](TwitchEvent::Broadcasts)
    /// has one; a [`RecommendError`] for the reason
    /// [`LastLive`](TwitchEvent::LastLive)'s is.
    ChannelNames {
        ids: Vec<String>,
        result: Result<Vec<Channel>, RecommendError>,
    },
    /// The badge sets a [`Request::Badges`] asked for, the global ones
    /// (`channel: None`) or one channel's, or why not. Its own event with the
    /// failure inside it, for the reason [`Broadcasts`](TwitchEvent::Broadcasts)
    /// has one.
    Badges {
        channel: Option<String>,
        result: Result<Vec<BadgeSet>, String>,
    },
    /// Sign-in itself failed, so nothing works.
    Error(String),
    /// One browse request failed. The session is fine; only that list is empty,
    /// and saying so there beats blanking the whole page. `list` says which,
    /// so the failure is said on that list and nowhere else: by the time it
    /// arrives the user may be on another, and back and forward make that
    /// routine.
    BrowseError {
        list: Option<ListKey>,
        reason: String,
    },
}

/// What a [`Request::Recommend`] comes back as: each seed beside the live
/// channels Twitch says its viewers also watch, in the order asked, or why
/// there are none.
pub type Recommendations = Result<Vec<(String, Vec<SimilarChannel>)>, RecommendError>;

/// Why a [`Request::Recommend`], a [`Request::LastLive`] or a
/// [`Request::ChannelNames`] came back with nothing. Two kinds, because the
/// root does different things with them; see
/// `recommended::Recommended::answered`, `last_live::LastLive::answered` and
/// `shared_chat::SourceRooms::answered`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecommendError {
    /// Twitch would not run the query (`twitch_api::Error::QueryRefused`):
    /// most likely the website's query has a new hash and this one is
    /// retired. It will not work again this session.
    Refused(String),
    /// Anything else — the network, a failure Twitch says passes, a 4xx —
    /// which a later ask may not meet.
    Failed(String),
}

pub struct TwitchService {
    stop: Arc<AtomicBool>,
    /// Dropped on teardown, which wakes the worker out of `recv_timeout`
    /// immediately rather than after the poll interval.
    requests: Option<Sender<Request>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl TwitchService {
    pub fn start(settings_path: PathBuf) -> (Self, mpsc::UnboundedReceiver<TwitchEvent>) {
        let (tx, rx) = mpsc::unbounded();
        let (requests_tx, requests_rx) = std::sync::mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));

        let thread = std::thread::Builder::new()
            .name("twitch".into())
            .spawn({
                let stop = stop.clone();
                move || run(settings_path, tx, stop, requests_rx)
            })
            .expect("failed to spawn twitch service");

        (
            Self {
                stop,
                requests: Some(requests_tx),
                thread: Some(thread),
            },
            rx,
        )
    }

    /// Ask for something, reporting whether anyone is there to answer.
    ///
    /// The send only fails once the worker has returned, which it does when
    /// sign-in fails. Callers need to know, or they show a spinner for a reply
    /// that is never coming.
    pub fn request(&self, request: Request) -> bool {
        self.requests
            .as_ref()
            .is_some_and(|requests| requests.send(request).is_ok())
    }
}

impl Drop for TwitchService {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Dropping the sender wakes a worker parked in `recv_timeout`
        // immediately, rather than at the next poll deadline.
        self.requests.take();
        // Deliberately not joined. Both of those wake a worker that is waiting;
        // neither reaches one that is inside a Helix request, and a follows
        // poll is two requests that *each* walk up to ten pages at a 20s
        // timeout apiece. Joining meant a settings save on a stalled network
        // froze the window for minutes — at exactly the moment the user had
        // just clicked Save.
        //
        // The worker holds nothing that must be torn down in order: its events
        // go to an unbounded channel whose receiver going away is not an error,
        // its main loop tests `stop` on every pass, and `persist` declines to
        // write once the flag is set — so a worker still in flight can finish
        // the request it is in without writing anything that now belongs to
        // its replacement.
        drop(self.thread.take());
    }
}

/// Sleep in slices so shutdown does not wait out a full poll interval.
fn interruptible_sleep(total: Duration, stop: &AtomicBool) -> bool {
    let deadline = Instant::now() + total;
    while Instant::now() < deadline {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    !stop.load(Ordering::Relaxed)
}

/// Persist a new session immediately, unless the service has been dropped.
///
/// Twitch refresh tokens are single use, so a session that is obtained and not
/// saved locks the user out on next launch. The write goes through
/// `Settings::save_sign_in`, which re-reads the file under the same lock the
/// UI's saves take, so it never clobbers a preference changed meanwhile and a
/// preference save can never clobber it.
///
/// The `stop` check is what makes the non-joining [`TwitchService::drop`] safe.
/// A worker that is mid-request when its service is replaced — which is exactly
/// what a client-id change does — would otherwise finish, and write tokens
/// belonging to the *old* client id over the new worker's settings. Teardown no
/// longer waits for this thread, so the thread has to decline instead.
fn persist(settings_path: &Path, session: &Session, stop: &AtomicBool) -> Result<(), String> {
    if stop.load(Ordering::Relaxed) {
        return Ok(());
    }
    let tokens = OAuthTokens {
        access_token: session.access_token.clone(),
        refresh_token: session.refresh_token.clone(),
        expires_at: session.expires_at,
        user_id: session.user_id.clone(),
        login: session.login.clone(),
    };
    Settings::save_sign_in(settings_path, Some(tokens)).map_err(|e| e.to_string())
}

/// How many times a refresh that never reached Twitch is retried at startup
/// before the worker gives up for this launch, and how long it waits between.
///
/// Startup is the one place a refresh cannot simply be deferred to the next
/// poll: there is no session yet to poll with. But the alternative to
/// retrying - starting a new device flow - throws away a refresh token that a
/// dropped packet has not spent, and forces the user back through twitch.tv
/// for no reason. A few tries over a few seconds covers wifi coming up after
/// a resume, which is when this happens.
const STARTUP_REFRESH_ATTEMPTS: u32 = 3;
const STARTUP_REFRESH_WAIT: Duration = Duration::from_secs(3);

/// Get a usable session, signing in or refreshing as needed.
fn establish_session(
    settings_path: &Path,
    client_id: &str,
    tx: &mpsc::UnboundedSender<TwitchEvent>,
    stop: &AtomicBool,
) -> Option<Session> {
    let settings = Settings::load(settings_path).ok()?;

    if let Some(stored) = settings.credentials.oauth.clone() {
        let stored_session = Session {
            access_token: stored.access_token.clone(),
            refresh_token: stored.refresh_token.clone(),
            expires_at: stored.expires_at,
            user_id: stored.user_id.clone(),
            login: stored.login.clone(),
        };
        if !twitch_api::needs_refresh(stored.expires_at) {
            return Some(stored_session);
        }

        let mut attempt = 0;
        loop {
            attempt += 1;
            match twitch_api::refresh(
                client_id,
                &stored.refresh_token,
                &stored.user_id,
                &stored.login,
            ) {
                Ok(session) => {
                    if let Err(e) = persist(settings_path, &session, stop) {
                        let _ = tx.unbounded_send(TwitchEvent::Error(format!(
                            "signed in but could not save tokens: {e}"
                        )));
                    }
                    return Some(session);
                }
                // The request never reached Twitch, or Twitch itself was down,
                // so the refresh token is unspent and still good. Retry a few
                // times; then, if the access token has life left, run on it
                // and let the poll loop refresh later. Only a token that has
                // actually run out ends the launch here - and even then with
                // a message about the network, not a fresh device flow that
                // would burn a token nothing has invalidated.
                Err(twitch_api::Error::Network(reason)) => {
                    eprintln!("refresh attempt {attempt}: {reason}");
                    if attempt < STARTUP_REFRESH_ATTEMPTS {
                        if !interruptible_sleep(STARTUP_REFRESH_WAIT, stop) {
                            return None;
                        }
                        continue;
                    }
                    if !twitch_api::has_expired(stored.expires_at) {
                        return Some(stored_session);
                    }
                    let _ = tx.unbounded_send(TwitchEvent::Error(format!(
                        "could not reach Twitch to renew the sign-in: {reason}"
                    )));
                    return None;
                }
                // Twitch answered and said no: the refresh token is dead, and
                // starting over is the only way forward.
                Err(e) => {
                    let _ = tx.unbounded_send(TwitchEvent::Error(format!("sign-in expired: {e}")));
                    break;
                }
            }
        }
    }

    let device = match twitch_api::start_device_flow(client_id) {
        Ok(device) => device,
        Err(e) => {
            let _ = tx.unbounded_send(TwitchEvent::Error(e.to_string()));
            return None;
        }
    };

    let _ = tx.unbounded_send(TwitchEvent::AwaitingCode {
        user_code: device.user_code.clone(),
        verification_uri: device.verification_uri.clone(),
    });

    let mut interval = Duration::from_secs(device.interval.max(1));
    let deadline = Instant::now() + Duration::from_secs(device.expires_in);

    while Instant::now() < deadline {
        if !interruptible_sleep(interval, stop) {
            return None;
        }
        match twitch_api::poll_token(client_id, &device.device_code) {
            Ok(session) => {
                if let Err(e) = persist(settings_path, &session, stop) {
                    let _ = tx.unbounded_send(TwitchEvent::Error(format!(
                        "signed in but could not save tokens: {e}"
                    )));
                }
                let _ = tx.unbounded_send(TwitchEvent::SignedIn {
                    login: session.login.clone(),
                });
                return Some(session);
            }
            Err(twitch_api::Error::Pending) => continue,
            // RFC 8628 §3.5: back off five seconds each time, or Twitch keeps
            // answering slow_down until the code expires underneath us.
            Err(twitch_api::Error::SlowDown) => {
                interval += SLOW_DOWN_STEP;
                continue;
            }
            // A sign-in window runs for minutes and the user is watching a code
            // on screen. One dropped packet is not a reason to tear the worker
            // down and make them restart the app; the deadline above is what
            // ends this loop.
            //
            // Only a failure on the *poll itself* qualifies. A transport
            // failure after the exchange has succeeded arrives as
            // `IdentityLookup` instead, and falls to the terminal arm below —
            // the device code is spent by then, so retrying it can only spend
            // the rest of the window on a code that cannot work again.
            Err(twitch_api::Error::Network(_)) => continue,
            Err(e) => {
                let _ = tx.unbounded_send(TwitchEvent::Error(e.to_string()));
                return None;
            }
        }
    }

    let _ = tx.unbounded_send(TwitchEvent::Error("the sign-in code expired".into()));
    None
}

/// Renew the access token if it is close to expiry.
///
/// Returns whether the session is still usable. A refresh Twitch *rejected* is
/// terminal, because the refresh token is dead either way. A refresh that
/// never got an answer is not: the token is unspent, the current access token
/// is good for a while yet - renewal starts five minutes ahead of expiry - and
/// the next poll will simply try again. Treating both alike meant one dropped
/// packet signed the user out for the rest of the session, which is the same
/// bad-wifi failure the post-exchange lookup used to have.
fn keep_session_fresh(
    session: &mut Session,
    client_id: &str,
    settings_path: &Path,
    tx: &mpsc::UnboundedSender<TwitchEvent>,
    stop: &AtomicBool,
) -> bool {
    if !twitch_api::needs_refresh(session.expires_at) {
        return true;
    }
    match twitch_api::refresh(
        client_id,
        &session.refresh_token,
        &session.user_id,
        &session.login,
    ) {
        Ok(fresh) => {
            let _ = persist(settings_path, &fresh, stop);
            *session = fresh;
            true
        }
        Err(twitch_api::Error::Network(reason)) => {
            eprintln!("refresh deferred: {reason}");
            true
        }
        Err(e) => {
            let _ = tx.unbounded_send(TwitchEvent::Error(format!("refresh failed: {e}")));
            false
        }
    }
}

fn serve(
    request: Request,
    client_id: &str,
    session: &Session,
    tx: &mpsc::UnboundedSender<TwitchEvent>,
) {
    let token = &session.access_token;
    // Read before the request is taken apart below, for a failure to say
    // which list it was.
    let list = request.list_key();
    let result = match request {
        // Intercepted by the caller: the first because it owns the poll
        // timer, the other three because they need no session.
        Request::Follows
        | Request::Recommend { .. }
        | Request::LastLive { .. }
        | Request::ChannelNames { .. } => return,
        Request::Popular { after } => {
            twitch_api::top_streams(client_id, token, None, after.as_deref())
                .map(|page| TwitchEvent::Popular(Listing::from(page, after.is_some())))
        }
        Request::Categories { after } => {
            twitch_api::top_categories(client_id, token, after.as_deref())
                .map(|page| TwitchEvent::Categories(Listing::from(page, after.is_some())))
        }
        Request::Category { category, after } => {
            twitch_api::top_streams(client_id, token, Some(&category.id), after.as_deref()).map(
                |page| TwitchEvent::CategoryStreams {
                    category,
                    streams: Listing::from(page, after.is_some()),
                },
            )
        }
        Request::Search(query) => search(client_id, token, query),
        Request::Videos {
            login,
            user_id,
            kind,
            after,
        } => channel_videos(client_id, token, login, user_id, kind, after),
        Request::Video { id } => {
            let result = match twitch_api::video(client_id, token, &id) {
                Ok(Some(video)) => Ok(video),
                Ok(None) => Err("Twitch has no recording by that id".to_string()),
                Err(e) => Err(e.to_string()),
            };
            Ok(TwitchEvent::Video { id, result })
        }
        Request::Broadcasts { login, user_id } => {
            let result = user_id_or_lookup(client_id, token, &login, user_id)
                .and_then(|user_id| {
                    twitch_api::recent_videos(
                        client_id,
                        token,
                        &user_id,
                        VideoKind::Archive,
                        BROADCASTS_ASKED,
                    )
                })
                .map_err(|e| e.to_string());
            Ok(TwitchEvent::Broadcasts { login, result })
        }
        Request::Badges { channel } => {
            let result = match &channel {
                None => twitch_api::badges::global_chat_badges(client_id, token),
                Some(id) => twitch_api::badges::channel_chat_badges(client_id, token, id),
            }
            .map_err(|e| e.to_string());
            Ok(TwitchEvent::Badges { channel, result })
        }
    };

    let _ = tx.unbounded_send(result.unwrap_or_else(|e| TwitchEvent::BrowseError {
        list,
        reason: e.to_string(),
    }));
}

/// Three requests behind one result.
///
/// `/search/channels` answers with a profile picture and no viewer count, which
/// is a different shape from every other list in the app, so its logins are fed
/// back through `/streams` to get ordinary stream records. Categories are
/// searched in the same breath because a name like "zomboid" is as likely to
/// mean the game as a channel.
fn search(client_id: &str, token: &str, query: String) -> Result<TwitchEvent, twitch_api::Error> {
    let categories = twitch_api::search_categories(client_id, token, &query)?;
    let logins = twitch_api::search_channels(client_id, token, &query)?;
    let streams = twitch_api::streams_by_login(client_id, token, &logins)?;
    // The channels that are not on: the way to a channel's page when you know
    // its name and it is not streaming, which the search used to have none of.
    // Worth less than the rest of the answer, so a failure here costs these
    // names and not the page. Anyone who went live between the two requests
    // is already among the streams, and is not listed twice.
    let channels = match twitch_api::search_offline_channels(client_id, token, &query) {
        Ok(channels) => channels
            .into_iter()
            .filter(|channel| !streams.iter().any(|s| s.user_login == channel.login))
            .collect(),
        Err(e) => {
            eprintln!("search: offline channels: {e}");
            Vec::new()
        }
    };
    Ok(TwitchEvent::SearchResults {
        query,
        categories,
        streams,
        channels,
    })
}

/// One kind of a channel's videos, looking its id up first when the caller
/// had only a name — the palette and the command line know channels by login,
/// and Helix lists videos by id alone.
fn channel_videos(
    client_id: &str,
    token: &str,
    login: String,
    user_id: Option<String>,
    kind: VideoKind,
    after: Option<String>,
) -> Result<TwitchEvent, twitch_api::Error> {
    let user_id = user_id_or_lookup(client_id, token, &login, user_id)?;
    let page = twitch_api::videos(client_id, token, &user_id, kind, after.as_deref())?;
    Ok(TwitchEvent::Videos {
        login,
        user_id,
        kind,
        videos: Listing::from(page, after.is_some()),
    })
}

/// The channel's id: `user_id` when the caller had it, or looked up from
/// `login` when it did not. Helix lists videos by id alone, and a channel can
/// arrive with a name and nothing else.
fn user_id_or_lookup(
    client_id: &str,
    token: &str,
    login: &str,
    user_id: Option<String>,
) -> Result<String, twitch_api::Error> {
    match user_id {
        Some(id) => Ok(id),
        None => twitch_api::user_id_for(client_id, token, login)?
            .map(|(id, _)| id)
            .ok_or_else(|| twitch_api::Error::Api(format!("there is no channel called {login}"))),
    }
}

/// Ask `similar` about each seed in turn, for [`Request::Recommend`]: every
/// seed with its answer, in order, or the first failure.
///
/// A refusal stops it, since the next seed would only be refused too, and so
/// does any other failure, dropping the answers before it: a network that
/// failed one request is likely failing the next, and the root asks about
/// every seed again at its next interval. `stop` is checked before each seed,
/// as `poll_follows` checks it between its two calls, and a service dropped
/// part-way sends nothing (`None`). Takes the asking as an argument so the
/// tests can answer for Twitch.
fn similar_to_each(
    seeds: Vec<String>,
    stop: &AtomicBool,
    mut similar: impl FnMut(&str) -> Result<Vec<SimilarChannel>, twitch_api::Error>,
) -> Option<Recommendations> {
    let mut answers = Vec::with_capacity(seeds.len());
    for seed in seeds {
        if stop.load(Ordering::Relaxed) {
            return None;
        }
        match similar(&seed) {
            Ok(channels) => answers.push((seed, channels)),
            Err(twitch_api::Error::QueryRefused(message)) => {
                return Some(Err(RecommendError::Refused(message)))
            }
            Err(e) => return Some(Err(RecommendError::Failed(e.to_string()))),
        }
    }
    Some(Ok(answers))
}

/// The followed channels whose picture has not been asked for this session.
///
/// A live channel's picture comes with every poll, because the live list is
/// short and changes; an offline channel's is asked once and kept, because the
/// list is long — a hundred and more — and a profile picture changes about as
/// often as a name. So a hundred offline follows cost two requests on the
/// first poll and none after it, until somebody new is followed.
fn unpictured(channels: &[Channel], asked: &HashSet<String>) -> Vec<String> {
    channels
        .iter()
        .map(|channel| channel.login.to_lowercase())
        .filter(|login| !asked.contains(login))
        .collect()
}

/// Ask who is live, then who is followed at all, then the pictures of the
/// followed channels the rail has not yet got one for.
///
/// Two calls because Helix has no endpoint that answers both, and at most one
/// complaint: on a real outage they fail together, and saying so twice is twice
/// as much noise for the same fact.
///
/// `stop` is checked between them. Nothing waits for this thread any more, so
/// that is not about shutdown latency — it is about not spending a second
/// multi-page request, and not sending its results, for a service that has
/// already been dropped.
///
/// `pictured` is the worker's record of the offline pictures already asked
/// for; see [`unpictured`].
fn poll_follows(
    client_id: &str,
    session: &Session,
    tx: &mpsc::UnboundedSender<TwitchEvent>,
    stop: &AtomicBool,
    pictured: &mut HashSet<String>,
) {
    let token = &session.access_token;
    let mut failure = None;

    match twitch_api::followed_streams(client_id, token, &session.user_id) {
        Ok(streams) => {
            // Asked for before the list is handed over, so the logins are still
            // to hand; the pictures follow in their own event because they are
            // another round trip and the names should not wait on them.
            let live: Vec<String> = streams
                .iter()
                .map(|stream| stream.user_login.clone())
                .collect();
            let _ = tx.unbounded_send(TwitchEvent::Streams(streams));

            if !live.is_empty() && !stop.load(Ordering::Relaxed) {
                match twitch_api::profile_images(client_id, token, &live) {
                    Ok(images) => {
                        let _ = tx.unbounded_send(TwitchEvent::Avatars(images));
                    }
                    // Not worth a failure of its own. Every row this feeds
                    // still has a name, which is the part that identifies it.
                    Err(e) => eprintln!("avatars: {e}"),
                }
            }
        }
        Err(e) => failure = Some(e.to_string()),
    }

    if stop.load(Ordering::Relaxed) {
        return;
    }

    match twitch_api::followed_channels(client_id, token, &session.user_id) {
        Ok(channels) => {
            let missing = unpictured(&channels, pictured);
            let _ = tx.unbounded_send(TwitchEvent::FollowedChannels(channels));

            // After the names, for the same reason as the live pictures: the
            // rail shows the names at once and fills the faces in. Marked as
            // asked only once answered, so a failed request is tried again at
            // the next poll rather than leaving the rail faceless all session.
            if !missing.is_empty() && !stop.load(Ordering::Relaxed) {
                match twitch_api::profile_images(client_id, token, &missing) {
                    Ok(images) => {
                        pictured.extend(missing);
                        let _ = tx.unbounded_send(TwitchEvent::Avatars(images));
                    }
                    Err(e) => eprintln!("avatars (offline): {e}"),
                }
            }
        }
        Err(e) => failure = failure.or_else(|| Some(e.to_string())),
    }

    if let Some(reason) = failure {
        let _ = tx.unbounded_send(TwitchEvent::FollowsError(reason));
    }
}

/// What a failure of the unpublished GraphQL endpoint comes to for the root:
/// a refusal, which ends the asking, or anything else, which does not.
fn recommend_error(e: twitch_api::Error) -> RecommendError {
    match e {
        twitch_api::Error::QueryRefused(message) => RecommendError::Refused(message),
        e => RecommendError::Failed(e.to_string()),
    }
}

fn run(
    settings_path: PathBuf,
    tx: mpsc::UnboundedSender<TwitchEvent>,
    stop: Arc<AtomicBool>,
    requests: Receiver<Request>,
) {
    let Ok(settings) = Settings::load(&settings_path) else {
        let _ = tx.unbounded_send(TwitchEvent::Error("could not read settings".into()));
        return;
    };
    let Some(client_id) = settings
        .credentials
        .client_id
        .clone()
        .filter(|id| !id.is_empty())
    else {
        let _ = tx.unbounded_send(TwitchEvent::NeedsClientId);
        return;
    };

    let Some(mut session) = establish_session(&settings_path, &client_id, &tx, &stop) else {
        return;
    };
    let _ = tx.unbounded_send(TwitchEvent::SignedIn {
        login: session.login.clone(),
    });

    let mut next_poll = Instant::now();
    // Whose offline picture has been asked for; see `unpictured`. Per worker,
    // so a new sign-in starts it over, which is when the follows can change.
    let mut pictured = HashSet::new();

    while !stop.load(Ordering::Relaxed) {
        if Instant::now() >= next_poll {
            if !keep_session_fresh(&mut session, &client_id, &settings_path, &tx, &stop) {
                return;
            }
            poll_follows(&client_id, &session, &tx, &stop, &mut pictured);
            next_poll = Instant::now() + POLL_INTERVAL;
        }

        // The wait until the next poll is also the window in which requests are
        // answered, so browsing never has to queue behind a timer.
        let wait = next_poll.saturating_duration_since(Instant::now());
        match requests.recv_timeout(wait) {
            // Anonymous, so ahead of the session's upkeep; see the request.
            // One request per seed, one after another, which is the worker's
            // time every request behind it waits for — why the root caps the
            // seeds and asks rarely.
            Ok(Request::Recommend { seeds }) => {
                if let Some(answer) =
                    similar_to_each(seeds, &stop, twitch_api::recommend::similar_channels)
                {
                    let _ = tx.unbounded_send(TwitchEvent::Recommended(answer));
                }
            }
            // Anonymous too, and two requests for a couple of hundred
            // follows, asked once in fifteen minutes; see `crate::last_live`.
            Ok(Request::LastLive { logins }) => {
                let answer =
                    twitch_api::recommend::last_broadcasts(&logins).map_err(recommend_error);
                if !stop.load(Ordering::Relaxed) {
                    let _ = tx.unbounded_send(TwitchEvent::LastLive(answer));
                }
            }
            // Anonymous too, and one request per partner channel a session,
            // a handful at most; see `crate::shared_chat`.
            Ok(Request::ChannelNames { ids }) => {
                let result = twitch_api::recommend::channel_names(&ids).map_err(recommend_error);
                if !stop.load(Ordering::Relaxed) {
                    let _ = tx.unbounded_send(TwitchEvent::ChannelNames { ids, result });
                }
            }
            Ok(request) => {
                if !keep_session_fresh(&mut session, &client_id, &settings_path, &tx, &stop) {
                    return;
                }
                match request {
                    // Not left to `serve`, which cannot see the timer: a poll
                    // done by hand there would be repeated automatically a few
                    // seconds later, for two of everything.
                    Request::Follows => {
                        poll_follows(&client_id, &session, &tx, &stop, &mut pictured);
                        next_poll = Instant::now() + POLL_INTERVAL;
                    }
                    other => serve(other, &client_id, &session, &tx),
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            // The service was dropped, which is how shutdown reaches us.
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(login: &str) -> Channel {
        Channel {
            login: login.to_string(),
            user_id: String::new(),
            display_name: login.to_string(),
        }
    }

    /// An offline follow's picture is asked for once a session, so the rail
    /// fills in on the first poll and a hundred follows cost two requests,
    /// not two a minute. Compared lowercase, the way Helix answers.
    #[test]
    fn offline_pictures_are_asked_for_once() {
        let follows = [channel("Asmongold"), channel("forsen"), channel("Lirik")];
        let mut asked = HashSet::new();
        assert_eq!(
            unpictured(&follows, &asked),
            ["asmongold", "forsen", "lirik"]
        );

        asked.extend(unpictured(&follows, &asked));
        assert!(unpictured(&follows, &asked).is_empty());

        let more = [channel("forsen"), channel("NewFollow")];
        assert_eq!(unpictured(&more, &asked), ["newfollow"]);
    }

    /// Every request the browse page makes names the list it fills, and the
    /// seven that fill none say so. A request with no key would leave its list
    /// with nothing to wait on, and its failure with nowhere to be said; a
    /// pane's or the rail's request with one would end a browse list's wait.
    #[test]
    fn every_browse_request_names_its_list() {
        let category = Category {
            id: "509658".into(),
            name: "Just Chatting".into(),
            box_art_url: String::new(),
        };
        let cases = [
            (Request::Follows, None),
            (Request::Video { id: "1".into() }, None),
            (
                Request::Broadcasts {
                    login: "someone".into(),
                    user_id: None,
                },
                None,
            ),
            (
                Request::Recommend {
                    seeds: vec!["forsen".into()],
                },
                None,
            ),
            (
                Request::LastLive {
                    logins: vec!["forsen".into()],
                },
                None,
            ),
            (
                Request::ChannelNames {
                    ids: vec!["12826".into()],
                },
                None,
            ),
            (Request::Badges { channel: None }, None),
            (
                Request::Badges {
                    channel: Some("12826".into()),
                },
                None,
            ),
            (Request::Popular { after: None }, Some(ListKey::Popular)),
            (
                Request::Popular {
                    after: Some("page2".into()),
                },
                Some(ListKey::Popular),
            ),
            (
                Request::Categories { after: None },
                Some(ListKey::Categories),
            ),
            (
                Request::Category {
                    category,
                    after: Some("page2".into()),
                },
                Some(ListKey::Category("509658".into())),
            ),
            (
                Request::Search("zomboid".into()),
                Some(ListKey::Search("zomboid".into())),
            ),
            (
                Request::Videos {
                    login: "someone".into(),
                    user_id: None,
                    kind: VideoKind::Highlight,
                    after: None,
                },
                Some(ListKey::Videos {
                    login: "someone".into(),
                    kind: VideoKind::Highlight,
                }),
            ),
        ];
        for (request, key) in cases {
            assert_eq!(request.list_key(), key, "{request:?}");
        }
    }

    fn similar(login: &str) -> SimilarChannel {
        SimilarChannel {
            login: login.into(),
            user_id: String::new(),
            display_name: login.into(),
            title: String::new(),
            game_name: String::new(),
            game_id: String::new(),
            viewer_count: 1,
            profile_image_url: String::new(),
            stream_id: String::new(),
        }
    }

    fn seeds(logins: &[&str]) -> Vec<String> {
        logins.iter().map(|login| login.to_string()).collect()
    }

    /// Every seed is asked about, in order, and comes back beside its own
    /// answer.
    #[test]
    fn recommendations_ask_about_every_seed_in_order() {
        let mut asked = Vec::new();
        let answer = similar_to_each(
            seeds(&["forsen", "nymn"]),
            &AtomicBool::new(false),
            |seed| {
                asked.push(seed.to_string());
                Ok(vec![similar(&format!("like_{seed}"))])
            },
        );
        assert_eq!(asked, ["forsen", "nymn"]);
        let answers = answer.expect("nothing stopped it").expect("nothing failed");
        assert_eq!(answers.len(), 2);
        assert_eq!(answers[0].0, "forsen");
        assert_eq!(answers[0].1[0].login, "like_forsen");
        assert_eq!(answers[1].0, "nymn");
    }

    /// A refusal is told apart from every other failure, and either stops
    /// the asking there: the seeds after it are not asked about at all.
    #[test]
    fn recommendations_stop_at_the_first_failure_and_say_which_kind() {
        let mut asked = 0;
        let refused = similar_to_each(seeds(&["a", "b", "c"]), &AtomicBool::new(false), |_| {
            asked += 1;
            if asked == 2 {
                Err(twitch_api::Error::QueryRefused(
                    "PersistedQueryNotFound".into(),
                ))
            } else {
                Ok(Vec::new())
            }
        });
        assert_eq!(asked, 2, "asked on past a refusal");
        assert_eq!(
            refused,
            Some(Err(RecommendError::Refused(
                "PersistedQueryNotFound".into()
            )))
        );

        for error in [
            twitch_api::Error::Network("timed out".into()),
            twitch_api::Error::Api("HTTP 403".into()),
            twitch_api::Error::Shape("not JSON".into()),
        ] {
            let message = error.to_string();
            let mut error = Some(error);
            let failed = similar_to_each(seeds(&["a", "b"]), &AtomicBool::new(false), |_| {
                Err(error.take().expect("asked on past a failure"))
            });
            assert_eq!(failed, Some(Err(RecommendError::Failed(message))));
        }
    }

    /// A service dropped before the asking asks nothing, and one dropped
    /// part-way asks no further; neither sends anything.
    #[test]
    fn a_stopped_worker_asks_and_sends_nothing() {
        let answer = similar_to_each(seeds(&["a"]), &AtomicBool::new(true), |_| {
            panic!("asked Twitch for a service that has gone")
        });
        assert_eq!(answer, None);

        let stop = AtomicBool::new(false);
        let mut asked = 0;
        let answer = similar_to_each(seeds(&["a", "b"]), &stop, |_| {
            asked += 1;
            stop.store(true, Ordering::Relaxed);
            Ok(Vec::new())
        });
        assert_eq!(answer, None);
        assert_eq!(asked, 1);
    }

    fn a_session(login: &str) -> Session {
        Session {
            access_token: format!("access-{login}"),
            refresh_token: format!("refresh-{login}"),
            expires_at: 9_999_999_999,
            user_id: "1".into(),
            login: login.into(),
        }
    }

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join("perch-twitch-tests")
            .join(format!("{name}.json"))
    }

    /// What makes the non-joining `Drop` safe.
    ///
    /// Teardown no longer waits for this thread, so a worker that is mid-refresh
    /// when its service is replaced — which is exactly what changing the client
    /// id does — would otherwise land tokens belonging to the *old* client id on
    /// top of the new worker's settings. It has to decline instead.
    #[test]
    fn a_stopped_worker_does_not_write_the_tokens_it_was_holding() {
        let path = temp_file("stopped-worker");
        let _ = std::fs::remove_file(&path);
        Settings::default().save(&path).unwrap();

        let stop = AtomicBool::new(true);
        persist(&path, &a_session("ghost"), &stop).expect("declining is not an error");

        let after = Settings::load(&path).unwrap();
        assert!(
            after.credentials.oauth.is_none(),
            "a stopped worker wrote credentials anyway"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// The other half: a running worker must still save, or a single-use refresh
    /// token is spent and lost and the next launch is locked out.
    #[test]
    fn a_running_worker_still_writes_its_tokens() {
        let path = temp_file("running-worker");
        let _ = std::fs::remove_file(&path);
        Settings::default().save(&path).unwrap();

        let stop = AtomicBool::new(false);
        persist(&path, &a_session("real"), &stop).unwrap();

        let after = Settings::load(&path).unwrap();
        let oauth = after.credentials.oauth.expect("tokens were not saved");
        assert_eq!(oauth.login, "real");
        assert_eq!(oauth.refresh_token, "refresh-real");
        let _ = std::fs::remove_file(&path);
    }
}
