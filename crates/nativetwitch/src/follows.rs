//! Signing in and keeping the followed-streams list fresh.
//!
//! Runs on a worker thread like everything else that touches the network. The
//! device-code flow is inherently a polling loop — the app asks Twitch whether
//! the user has typed the code yet — so a thread parked on it is the natural
//! shape.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::channel::mpsc;
use settings::{OAuthTokens, Settings};
use twitch_api::{LiveStream, Session};

/// How often to re-ask Twitch who is live.
const POLL_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub enum FollowsEvent {
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
    Error(String),
}

pub struct FollowsService {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl FollowsService {
    pub fn start(settings_path: PathBuf) -> (Self, mpsc::UnboundedReceiver<FollowsEvent>) {
        let (tx, rx) = mpsc::unbounded();
        let stop = Arc::new(AtomicBool::new(false));

        let thread = std::thread::Builder::new()
            .name("twitch-follows".into())
            .spawn({
                let stop = stop.clone();
                move || run(settings_path, tx, stop)
            })
            .expect("failed to spawn follows service");

        (
            Self {
                stop,
                thread: Some(thread),
            },
            rx,
        )
    }
}

impl Drop for FollowsService {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Sleep in slices so shutdown does not wait out a full poll interval.
fn interruptible_sleep(total: Duration, stop: &AtomicBool) -> bool {
    let deadline = std::time::Instant::now() + total;
    while std::time::Instant::now() < deadline {
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
    tx: &mpsc::UnboundedSender<FollowsEvent>,
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
                    let _ = tx.unbounded_send(FollowsEvent::Error(format!(
                        "signed in but could not save tokens: {e}"
                    )));
                }
                return Some(session);
            }
            // A dead refresh token means starting over, not giving up.
            Err(e) => {
                let _ = tx.unbounded_send(FollowsEvent::Error(format!("sign-in expired: {e}")));
            }
        }
    }

    let device = match twitch_api::start_device_flow(client_id) {
        Ok(device) => device,
        Err(e) => {
            let _ = tx.unbounded_send(FollowsEvent::Error(e.to_string()));
            return None;
        }
    };

    let _ = tx.unbounded_send(FollowsEvent::AwaitingCode {
        user_code: device.user_code.clone(),
        verification_uri: device.verification_uri.clone(),
    });

    let interval = Duration::from_secs(device.interval.max(1));
    let deadline = std::time::Instant::now() + Duration::from_secs(device.expires_in);

    while std::time::Instant::now() < deadline {
        if !interruptible_sleep(interval, stop) {
            return None;
        }
        match twitch_api::poll_token(client_id, &device.device_code) {
            Ok(session) => {
                if let Err(e) = persist(settings_path, &session) {
                    let _ = tx.unbounded_send(FollowsEvent::Error(format!(
                        "signed in but could not save tokens: {e}"
                    )));
                }
                let _ = tx.unbounded_send(FollowsEvent::SignedIn {
                    login: session.login.clone(),
                });
                return Some(session);
            }
            Err(twitch_api::Error::Pending) => continue,
            Err(e) => {
                let _ = tx.unbounded_send(FollowsEvent::Error(e.to_string()));
                return None;
            }
        }
    }

    let _ = tx.unbounded_send(FollowsEvent::Error("the sign-in code expired".into()));
    None
}

fn run(settings_path: PathBuf, tx: mpsc::UnboundedSender<FollowsEvent>, stop: Arc<AtomicBool>) {
    let Ok(settings) = Settings::load(&settings_path) else {
        let _ = tx.unbounded_send(FollowsEvent::Error("could not read settings".into()));
        return;
    };
    let Some(client_id) = settings
        .credentials
        .client_id
        .clone()
        .filter(|id| !id.is_empty())
    else {
        let _ = tx.unbounded_send(FollowsEvent::NeedsClientId);
        return;
    };

    let Some(mut session) = establish_session(&settings_path, &client_id, &tx, &stop) else {
        return;
    };
    let _ = tx.unbounded_send(FollowsEvent::SignedIn {
        login: session.login.clone(),
    });

    while !stop.load(Ordering::Relaxed) {
        if twitch_api::needs_refresh(session.expires_at) {
            match twitch_api::refresh(&client_id, &session.refresh_token) {
                Ok(fresh) => {
                    let _ = persist(&settings_path, &fresh);
                    session = fresh;
                }
                Err(e) => {
                    let _ = tx.unbounded_send(FollowsEvent::Error(format!("refresh failed: {e}")));
                    return;
                }
            }
        }

        match twitch_api::followed_streams(&client_id, &session.access_token, &session.user_id) {
            Ok(streams) => {
                let _ = tx.unbounded_send(FollowsEvent::Streams(streams));
            }
            Err(e) => {
                let _ = tx.unbounded_send(FollowsEvent::Error(e.to_string()));
            }
        }

        if !interruptible_sleep(POLL_INTERVAL, &stop) {
            return;
        }
    }
}
