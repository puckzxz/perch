//! Read-only Twitch chat over IRC.
//!
//! Twitch allows anonymous connections: log in as `justinfan<digits>` with no
//! password and you get a read-only feed of any public channel. That means chat
//! needs no OAuth, no client id, and no user account — it is by far the cheapest
//! real feature in the app.
//!
//! Connection lives on a blocking thread rather than an async runtime, matching
//! how video works and avoiding an executor seam with GPUI. Chat volume is low
//! enough that a thread parked on a socket read costs nothing.
//!
//! No UI types appear in this crate's API.

pub mod message;

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use futures::channel::mpsc;
pub use message::{ChatMessage, IrcMessage};

const HOST: &str = "irc.chat.twitch.tv";
const PORT: u16 = 6697;
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Something worth showing in the chat pane.
#[derive(Debug, Clone)]
pub enum ChatEvent {
    Connected { channel: String },
    /// The channel's numeric Twitch id, which third-party emote providers key
    /// their per-channel sets on. Arrives once, just after joining.
    RoomState { room_id: String },
    Message(Box<ChatMessage>),
    /// A moderator cleared chat, or a user was banned/timed out.
    Cleared { login: Option<String> },
    Disconnected { reason: String },
}

/// A live connection to one channel's chat. Dropping it disconnects.
pub struct ChatClient {
    stop: Arc<AtomicBool>,
    /// Kept so `Drop` can unblock the reader: shutting the socket down makes the
    /// blocking read return immediately, which a read timeout cannot do without
    /// corrupting a partially-buffered line.
    socket: Arc<Mutex<Option<TcpStream>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ChatClient {
    /// Join `channel` (with or without a leading `#`) and stream its messages.
    pub fn connect(channel: &str) -> (Self, mpsc::UnboundedReceiver<ChatEvent>) {
        let channel = channel.trim_start_matches('#').to_lowercase();
        let (tx, rx) = mpsc::unbounded();
        let stop = Arc::new(AtomicBool::new(false));
        let socket: Arc<Mutex<Option<TcpStream>>> = Arc::new(Mutex::new(None));

        let thread = std::thread::Builder::new()
            .name("twitch-chat".into())
            .spawn({
                let stop = stop.clone();
                let socket = socket.clone();
                move || run(channel, tx, stop, socket)
            })
            .expect("failed to spawn chat thread");

        (
            Self {
                stop,
                socket,
                thread: Some(thread),
            },
            rx,
        )
    }
}

impl Drop for ChatClient {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(sock) = self.socket.lock().unwrap().take() {
            let _ = sock.shutdown(std::net::Shutdown::Both);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Reconnect loop. Each pass is one full connection attempt.
fn run(
    channel: String,
    tx: mpsc::UnboundedSender<ChatEvent>,
    stop: Arc<AtomicBool>,
    socket: Arc<Mutex<Option<TcpStream>>>,
) {
    let mut backoff = Duration::from_secs(1);

    while !stop.load(Ordering::Relaxed) {
        // A panic here would otherwise kill only this thread, leaving the UI
        // showing "connecting…" forever with the reason on a stderr nobody sees
        // in a windowed app. Surface it as a normal disconnect instead.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            session(&channel, &tx, &stop, &socket)
        }))
        .unwrap_or_else(|_| Err("chat thread panicked".to_string()));

        match outcome {
            Ok(()) => backoff = Duration::from_secs(1),
            Err(reason) => {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let _ = tx.unbounded_send(ChatEvent::Disconnected { reason });
            }
        }

        // Interruptible sleep, so Drop does not have to wait out the backoff.
        let deadline = std::time::Instant::now() + backoff;
        while std::time::Instant::now() < deadline {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// Make sure rustls has a crypto provider before we build a config.
///
/// rustls only auto-selects one when exactly one provider feature is enabled.
/// Cargo unifies features across the whole graph, so as soon as anything else in
/// the binary links rustls with a different provider, auto-selection fails and
/// `ClientConfig::builder()` panics. Installing explicitly makes this crate
/// behave the same whoever else is in the build.
fn ensure_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            // Losing the race to another installer is fine; we need *a*
            // provider, not specifically ours.
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
    });
}

fn tls_config() -> Arc<rustls::ClientConfig> {
    ensure_crypto_provider();
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

/// Anonymous Twitch logins are `justinfan` plus digits. The number only has to
/// be unlikely to collide, not unpredictable.
fn anonymous_nick() -> String {
    let n = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(12345)
        % 100_000;
    format!("justinfan{n}")
}

/// One connection, from TCP handshake until the socket closes or errors.
fn session(
    channel: &str,
    tx: &mpsc::UnboundedSender<ChatEvent>,
    stop: &AtomicBool,
    socket: &Arc<Mutex<Option<TcpStream>>>,
) -> Result<(), String> {
    let tcp = TcpStream::connect((HOST, PORT)).map_err(|e| format!("connect failed: {e}"))?;
    tcp.set_nodelay(true).ok();

    // Publish a handle before blocking, so Drop can interrupt us.
    let shutdown_handle = tcp.try_clone().map_err(|e| format!("clone failed: {e}"))?;
    *socket.lock().unwrap() = Some(shutdown_handle);

    let server_name = HOST
        .to_string()
        .try_into()
        .map_err(|_| "bad server name".to_string())?;
    let conn = rustls::ClientConnection::new(tls_config(), server_name)
        .map_err(|e| format!("TLS setup failed: {e}"))?;

    let mut reader = BufReader::new(rustls::StreamOwned::new(conn, tcp));

    // Tags carry colour, display name and emotes; commands carry CLEARCHAT and
    // friends. Without both, chat is just plain text.
    let handshake = format!(
        "CAP REQ :twitch.tv/tags twitch.tv/commands\r\nNICK {}\r\nJOIN #{channel}\r\n",
        anonymous_nick()
    );
    // rustls buffers plaintext inside the connection. Without an explicit flush
    // the handshake can sit there unsent while we block reading, and the server
    // waits for a NICK that never arrives.
    let stream = reader.get_mut();
    stream
        .write_all(handshake.as_bytes())
        .map_err(|e| format!("handshake failed: {e}"))?;
    stream
        .flush()
        .map_err(|e| format!("handshake flush failed: {e}"))?;

    let mut line = Vec::new();
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }

        line.clear();
        let read = reader
            .read_until(b'\n', &mut line)
            .map_err(|e| format!("read failed: {e}"))?;
        if read == 0 {
            return Err("server closed the connection".into());
        }

        // Chat occasionally carries invalid UTF-8; lossy beats dropping the line.
        let text = String::from_utf8_lossy(&line);

        // Set TWITCH_CHAT_DEBUG=1 to see the wire protocol. Invaluable when a
        // channel looks silent and you need to know whether that is the channel
        // or the parser.
        if std::env::var_os("TWITCH_CHAT_DEBUG").is_some() {
            eprintln!("<< {}", text.trim_end());
        }
        let Some(irc) = message::parse_line(&text) else {
            continue;
        };

        match irc.command.as_str() {
            // Twitch pings every few minutes and disconnects if we go quiet.
            "PING" => {
                let token = irc.param(0).unwrap_or("tmi.twitch.tv");
                let stream = reader.get_mut();
                stream
                    .write_all(format!("PONG :{token}\r\n").as_bytes())
                    .map_err(|e| format!("pong failed: {e}"))?;
                stream
                    .flush()
                    .map_err(|e| format!("pong flush failed: {e}"))?;
            }
            "PRIVMSG" => {
                if let Some(chat) = ChatMessage::from_irc(&irc) {
                    if tx
                        .unbounded_send(ChatEvent::Message(Box::new(chat)))
                        .is_err()
                    {
                        return Ok(()); // receiver gone; nobody is listening
                    }
                }
            }
            "CLEARCHAT" => {
                let _ = tx.unbounded_send(ChatEvent::Cleared {
                    login: irc.param(1).map(str::to_string),
                });
            }
            // 366 is the end of the name list, i.e. the join actually completed.
            "366" => {
                let _ = tx.unbounded_send(ChatEvent::Connected {
                    channel: channel.to_string(),
                });
            }
            "ROOMSTATE" => {
                if let Some(room_id) = irc.tag("room-id") {
                    let _ = tx.unbounded_send(ChatEvent::RoomState {
                        room_id: room_id.to_string(),
                    });
                }
            }
            "RECONNECT" => return Err("server asked us to reconnect".into()),
            _ => {}
        }
    }
}
