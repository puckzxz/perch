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
use twitch_api::{Category, LiveStream, Session};

/// How often to re-ask Twitch who is live.
const POLL_INTERVAL: Duration = Duration::from_secs(60);

/// Something the browse page wants fetched.
#[derive(Debug, Clone)]
pub enum Request {
    /// The most-watched streams overall.
    Popular,
    /// The categories with the most viewers.
    Categories,
    /// The most-watched streams inside one category.
    Category(Category),
    /// Categories and live channels matching a name.
    Search(String),
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
    Popular(Vec<LiveStream>),
    Categories(Vec<Category>),
    CategoryStreams {
        category: Category,
        streams: Vec<LiveStream>,
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
    /// Dropped before joining, which wakes the worker out of `recv_timeout`
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
        self.requests.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
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

/// Persist a new session immediately.
///
/// Twitch refresh tokens are single use, so a session that is obtained and not
/// saved locks the user out on next launch. Settings are re-read first so this
/// never clobbers an unrelated change made in the UI meanwhile.
fn persist(settings_path: &Path, session: &Session) -> Result<(), String> {
    let mut settings = Settings::load(settings_path).map_err(|e| e.to_string())?;
    settings.credentials.oauth = Some(OAuthTokens {
        access_token: session.access_token.clone(),
        refresh_token: session.refresh_token.clone(),
        expires_at: session.expires_at,
        user_id: session.user_id.clone(),
        login: session.login.clone(),
    });
    settings.save(settings_path).map_err(|e| e.to_string())
}

/// Get a usable session, signing in or refreshing as needed.
fn establish_session(
    settings_path: &Path,
    client_id: &str,
    tx: &mpsc::UnboundedSender<TwitchEvent>,
    stop: &AtomicBool,
) -> Option<Session> {
    let settings = Settings::load(settings_path).ok()?;

    if let Some(stored) = settings.credentials.oauth.clone() {
        if !twitch_api::needs_refresh(stored.expires_at) {
            return Some(Session {
                access_token: stored.access_token,
                refresh_token: stored.refresh_token,
                expires_at: stored.expires_at,
                user_id: stored.user_id,
                login: stored.login,
            });
        }
        match twitch_api::refresh(client_id, &stored.refresh_token) {
            Ok(session) => {
                if let Err(e) = persist(settings_path, &session) {
                    let _ = tx.unbounded_send(TwitchEvent::Error(format!(
                        "signed in but could not save tokens: {e}"
                    )));
                }
                return Some(session);
            }
            // A dead refresh token means starting over, not giving up.
            Err(e) => {
                let _ = tx.unbounded_send(TwitchEvent::Error(format!("sign-in expired: {e}")));
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

    let interval = Duration::from_secs(device.interval.max(1));
    let deadline = Instant::now() + Duration::from_secs(device.expires_in);

    while Instant::now() < deadline {
        if !interruptible_sleep(interval, stop) {
            return None;
        }
        match twitch_api::poll_token(client_id, &device.device_code) {
            Ok(session) => {
                if let Err(e) = persist(settings_path, &session) {
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
/// Returns whether the session is still usable; a failed refresh is terminal,
/// because the refresh token it just spent is gone either way.
fn keep_session_fresh(
    session: &mut Session,
    client_id: &str,
    settings_path: &Path,
    tx: &mpsc::UnboundedSender<TwitchEvent>,
) -> bool {
    if !twitch_api::needs_refresh(session.expires_at) {
        return true;
    }
    match twitch_api::refresh(client_id, &session.refresh_token) {
        Ok(fresh) => {
            let _ = persist(settings_path, &fresh);
            *session = fresh;
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
        Request::Popular => {
            twitch_api::top_streams(client_id, token, None).map(TwitchEvent::Popular)
        }
        Request::Categories => {
            twitch_api::top_categories(client_id, token).map(TwitchEvent::Categories)
        }
        Request::Category(category) => {
            twitch_api::top_streams(client_id, token, Some(&category.id))
                .map(|streams| TwitchEvent::CategoryStreams { category, streams })
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
            if !keep_session_fresh(&mut session, &client_id, &settings_path, &tx) {
                return;
            }
            match twitch_api::followed_streams(&client_id, &session.access_token, &session.user_id)
            {
                Ok(streams) => {
                    let _ = tx.unbounded_send(TwitchEvent::Streams(streams));
                }
                Err(e) => {
                    let _ = tx.unbounded_send(TwitchEvent::Error(e.to_string()));
                }
            }
            next_poll = Instant::now() + POLL_INTERVAL;
        }

        // The wait until the next poll is also the window in which requests are
        // answered, so browsing never has to queue behind a timer.
        let wait = next_poll.saturating_duration_since(Instant::now());
        match requests.recv_timeout(wait) {
            Ok(request) => {
                if !keep_session_fresh(&mut session, &client_id, &settings_path, &tx) {
                    return;
                }
                serve(request, &client_id, &session, &tx);
            }
            Err(RecvTimeoutError::Timeout) => {}
            // The service was dropped, which is how shutdown reaches us.
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}
