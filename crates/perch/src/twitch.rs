//! The Twitch worker: signing in, keeping the follows list fresh, and
//! answering the browse page's requests.
//!
//! One thread owns the session, and it has to. Refresh tokens are single-use,
//! so two things refreshing at once would spend the same token twice and lock
//! the user out. Everything that reads Helix goes through here for that reason,
//! not merely for tidiness.
//!
//! The thread alternates between a follows poll on a timer and whatever the UI
//! asks for in between, which is what the request channel is: `recv_timeout`
//! against the next poll deadline is both the wait and the mailbox.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::channel::mpsc;
use settings::{OAuthTokens, Settings};
use twitch_api::{Category, FollowedChannel, LiveStream, Session};

/// How often to re-ask Twitch who is live.
const POLL_INTERVAL: Duration = Duration::from_secs(60);

/// How much to slow the device-code poll by each time Twitch says `slow_down`.
/// Five seconds is what RFC 8628 §3.5 specifies, not a number we picked.
const SLOW_DOWN_STEP: Duration = Duration::from_secs(5);

/// Something the browse page wants fetched.
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
    /// Categories and live channels matching a name.
    Search(String),
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
    /// [`FollowedChannel`] for why merging them would be wrong three times
    /// over.
    FollowedChannels(Vec<FollowedChannel>),
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
    },
    /// Sign-in itself failed, so nothing works.
    Error(String),
    /// One browse request failed. The session is fine; only that list is empty,
    /// and saying so there beats blanking the whole page.
    BrowseError(String),
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
    let result = match request {
        // Intercepted by the caller, which owns the poll timer.
        Request::Follows => return,
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
    };

    let _ = tx.unbounded_send(result.unwrap_or_else(|e| TwitchEvent::BrowseError(e.to_string())));
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
    Ok(TwitchEvent::SearchResults {
        query,
        categories,
        streams,
    })
}

/// Ask who is live, then who is followed at all.
///
/// Two calls because Helix has no endpoint that answers both, and at most one
/// complaint: on a real outage they fail together, and saying so twice is twice
/// as much noise for the same fact.
///
/// `stop` is checked between them. Nothing waits for this thread any more, so
/// that is not about shutdown latency — it is about not spending a second
/// multi-page request, and not sending its results, for a service that has
/// already been dropped.
fn poll_follows(
    client_id: &str,
    session: &Session,
    tx: &mpsc::UnboundedSender<TwitchEvent>,
    stop: &AtomicBool,
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
            let _ = tx.unbounded_send(TwitchEvent::FollowedChannels(channels));
        }
        Err(e) => failure = failure.or_else(|| Some(e.to_string())),
    }

    if let Some(reason) = failure {
        let _ = tx.unbounded_send(TwitchEvent::FollowsError(reason));
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

    while !stop.load(Ordering::Relaxed) {
        if Instant::now() >= next_poll {
            if !keep_session_fresh(&mut session, &client_id, &settings_path, &tx, &stop) {
                return;
            }
            poll_follows(&client_id, &session, &tx, &stop);
            next_poll = Instant::now() + POLL_INTERVAL;
        }

        // The wait until the next poll is also the window in which requests are
        // answered, so browsing never has to queue behind a timer.
        let wait = next_poll.saturating_duration_since(Instant::now());
        match requests.recv_timeout(wait) {
            Ok(request) => {
                if !keep_session_fresh(&mut session, &client_id, &settings_path, &tx, &stop) {
                    return;
                }
                match request {
                    // Not left to `serve`, which cannot see the timer: a poll
                    // done by hand there would be repeated automatically a few
                    // seconds later, for two of everything.
                    Request::Follows => {
                        poll_follows(&client_id, &session, &tx, &stop);
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
