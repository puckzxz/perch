//! Persisted user settings.
//!
//! Stored as JSON next to the user's other roaming app data. Unknown fields are
//! ignored and missing fields fall back to defaults, so a settings file written
//! by an older or newer build still loads rather than resetting everything.
//!
//! Credentials live here too. They are stored in plain text, which is the same
//! thing every desktop Twitch client does, but it is a deliberate choice rather
//! than an oversight — see [`Credentials`].
//!
//! What has been watched is in a file of its own beside this one; see
//! [`history`].

pub mod history;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// One writer at a time, process-wide.
///
/// Two threads write the settings file: the UI saves preferences, and the
/// sign-in worker saves tokens as Twitch issues them. Each one reads the file
/// back first to keep the other's fields, which is right, and is also a
/// read-modify-write - so with nothing serialising them, a token save landing
/// between the UI's read and its write would be overwritten by the *old*
/// tokens the UI had just read, and a refresh token Twitch honours exactly
/// once would be gone. Both writers share one temporary filename too, so two
/// concurrent saves could also collide on the rename. Every path that writes
/// the file holds this across its read and its write.
static FILE_LOCK: Mutex<()> = Mutex::new(());

/// Hold the write lock, surviving a poisoned mutex: a panic in another writer
/// says nothing about the file, which every save rewrites whole.
fn file_lock() -> std::sync::MutexGuard<'static, ()> {
    FILE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// How many channels [`Settings::recent`] remembers.
///
/// Enough for a week of evenings, and few enough that the palette's first
/// screen is still mostly who is live.
pub const RECENT_LIMIT: usize = 8;

/// Which stream quality to pull.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "value", rename_all = "snake_case")]
pub enum QualityPreference {
    /// Choose based on the size of the video pane.
    ///
    /// Worth preferring: measurements showed cost tracks the ratio between
    /// source and pane far more than pixel count, so picking to land on a clean
    /// ratio is cheaper than simply taking the best available.
    #[default]
    Auto,
    /// Always ask for this streamlink quality name, e.g. `"720p60"`.
    Fixed(String),
}

impl QualityPreference {
    /// The preference in the words it is stored in: `auto`, or the quality
    /// name — `best`, `1080p`. A key to look a label up by, not the label:
    /// what somebody reads is the settings sheet's wording for it.
    pub fn name(&self) -> &str {
        match self {
            QualityPreference::Auto => "auto",
            QualityPreference::Fixed(name) => name,
        }
    }
}

/// How big chat's text is drawn, in four steps; see
/// [`Settings::chat_text_size`]. Names, not pixels: what each step comes to
/// is the app's to say (`perch`'s `chat_display`), so the file never holds a
/// number a later build would have to second-guess.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatTextSize {
    Small,
    /// The size chat always was.
    #[default]
    Default,
    Large,
    Larger,
}

impl ChatTextSize {
    /// Every step, smallest first: the order the menu lists them in.
    pub const ALL: [ChatTextSize; 4] = [
        ChatTextSize::Small,
        ChatTextSize::Default,
        ChatTextSize::Large,
        ChatTextSize::Larger,
    ];
}

/// Read [`Settings::chat_text_size`], taking anything that is not one of the
/// steps above as [`ChatTextSize::Default`] rather than failing the file.
///
/// A plain enum would refuse a name it does not know, and one bad field
/// fails the whole file: a step a later build adds, read by this one, or a
/// hand edit that wrote `"Larger"` with the capital the menu shows. The app
/// would then run on defaults, and every save after would fail too, since
/// saving reads the file first (`save_preferences`). The module promises a
/// file from a newer build still loads, so this one field gives way instead.
fn lenient_text_size<'de, D>(deserializer: D) -> Result<ChatTextSize, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Read {
        Known(ChatTextSize),
        Unknown(serde::de::IgnoredAny),
    }
    Ok(match Read::deserialize(deserializer)? {
        Read::Known(size) => size,
        Read::Unknown(_) => ChatTextSize::Default,
    })
}

/// Twitch credentials.
///
/// Two unrelated tokens, which is confusing enough to be worth spelling out:
///
/// - `auth_token` is the `auth-token` **cookie** from twitch.tv. streamlink
///   sends it as an API header to unlock subscriber-only qualities and suppress
///   ads. It is a full account credential.
/// - `oauth` is a proper Helix OAuth token obtained by device-code sign-in,
///   scoped to reading your follows. It cannot do what the cookie token does,
///   and the cookie token cannot call Helix.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Credentials {
    /// Twitch application client id, from dev.twitch.tv. Required for sign-in.
    pub client_id: Option<String>,
    /// The twitch.tv `auth-token` cookie. Optional.
    pub auth_token: Option<String>,
    /// Helix tokens from device-code sign-in.
    pub oauth: Option<OAuthTokens>,
}

/// Written by hand so that a `{:?}` of anything holding credentials - which
/// includes the whole of [`Settings`] - cannot put an account credential in a
/// log. The client id is public by design and stays legible; the cookie is
/// shown only as present or absent.
impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("client_id", &self.client_id)
            .field("auth_token", &self.auth_token.as_ref().map(|_| REDACTED))
            .field("oauth", &self.oauth)
            .finish()
    }
}

/// What a secret looks like in debug output.
const REDACTED: &str = "<redacted>";

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OAuthTokens {
    pub access_token: String,
    /// Twitch refresh tokens are single-use: every refresh returns a new one,
    /// and the old one stops working immediately. Persist after every refresh
    /// or the next launch is locked out.
    pub refresh_token: String,
    /// Unix seconds. Refresh a little before this rather than on failure.
    pub expires_at: u64,
    /// The signed-in user, cached so startup does not need an extra request.
    pub user_id: String,
    pub login: String,
}

/// See [`Credentials`]'s `Debug`: the tokens are the secret, the rest is not.
impl std::fmt::Debug for OAuthTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthTokens")
            .field("access_token", &REDACTED)
            .field("refresh_token", &REDACTED)
            .field("expires_at", &self.expires_at)
            .field("user_id", &self.user_id)
            .field("login", &self.login)
            .finish()
    }
}

/// What is remembered about one channel.
///
/// A struct rather than a bare number because per-channel *quality* is the
/// obvious next thing to want here, and a map of structs grows a field for free
/// where a map of numbers would need a migration.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelPrefs {
    /// 0-100. `None` means nobody has ever set a level here, which has to stay
    /// distinguishable from `Some(0)` — a channel you deliberately muted should
    /// reopen muted, and one you have never opened should not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<u8>,
    /// Whether this channel opens with its chat pane hidden.
    ///
    /// A plain `bool` rather than an `Option`, unlike `volume`, because there
    /// is no global default for it to fall back to: chat shown is simply what
    /// every channel does until you say otherwise about that one. Hiding chat
    /// on a channel you watch for the game says nothing about the next channel,
    /// which is the same argument that keeps muting from becoming the default
    /// level.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub chat_hidden: bool,
}

impl ChannelPrefs {
    /// Whether this entry remembers anything at all. An entry that does not is
    /// dropped rather than written, so the file never fills with channels
    /// that have nothing to say about themselves.
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Where the window was when it was last closed, so it comes back there.
///
/// Logical pixels, the same units gpui reports. `maximized` is kept apart from
/// the bounds because a maximised window still has a restore size, and losing
/// it would make un-maximising land somewhere arbitrary.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowPlacement {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

impl Default for WindowPlacement {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            maximized: false,
        }
    }
}

impl WindowPlacement {
    /// Whether the size is one a window could actually be. A hand-edited file
    /// can say anything, and a zero or negative size would open nothing.
    pub fn is_usable(&self) -> bool {
        self.width.is_finite()
            && self.height.is_finite()
            && self.x.is_finite()
            && self.y.is_finite()
            && self.width >= 320.0
            && self.height >= 240.0
    }
}

/// How a channel is identified in [`Settings::channel_prefs`].
///
/// Twitch logins are case-insensitive and the app does not agree with itself
/// about case: the browse page hands over Helix's lowercase `user_login`, while
/// the command line hands over whatever was typed. Normalising in one place is
/// what stops `Forsen` and `forsen` remembering two different levels.
pub fn channel_key(channel: &str) -> String {
    channel.trim_start_matches('#').to_ascii_lowercase()
}

/// Whether `key` could be a channel's login: letters, digits and underscores,
/// which is all Twitch allows in one.
fn is_login(key: &str) -> bool {
    !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub quality: QualityPreference,
    /// 0-100. What a channel with no remembered level of its own opens at.
    ///
    /// mpv's own default is 100, which is startling for a window that starts
    /// playing as soon as it opens. It follows the last level you *chose*, so
    /// an unfamiliar channel opens near where you have been listening rather
    /// than back at the factory setting — but see [`Settings::set_volume_for`]
    /// for the one change that deliberately does not move it.
    pub volume: u8,
    /// Per-channel overrides, keyed by [`channel_key`].
    ///
    /// One global level meant the last channel you adjusted set the volume for
    /// every stream after it, and streamers are wildly inconsistent about how
    /// loud they run.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub channel_prefs: BTreeMap<String, ChannelPrefs>,
    pub credentials: Credentials,
    /// The channels watched most recently, newest first and at most
    /// [`RECENT_LIMIT`] of them, by [`channel_key`]. What the palette offers
    /// before anything has been typed: the most common thing to open is what
    /// was open yesterday.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent: Vec<String>,
    /// The channels pinned to the top of the rail, by [`channel_key`], in the
    /// order they were pinned.
    ///
    /// Pin order rather than viewers or names, because a pin is a statement
    /// that these few come first whoever else is on, and a group that
    /// reshuffled itself every minute would be the live list again. Tidied on
    /// load and on every write (see `tidy_pinned`), since the file is
    /// hand-editable and pins can be set there by hand.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pinned: Vec<String>,
    /// What `recent` was before it was a list. Every build up to 0.2.1 wrote
    /// the last channel opened here and nothing ever read it back; it is
    /// folded into `recent` on load and never written again.
    #[serde(default, skip_serializing)]
    last_channel: Option<String>,
    /// Width of the chat pane when it sits beside the video. There is no
    /// height counterpart: when chat sits below the video it takes whatever the
    /// video leaves, which on a tall window is the point.
    pub chat_width: f32,
    /// How many messages from before you joined to load into a new chat pane.
    /// Zero turns the request off.
    ///
    /// Twitch has no endpoint for this, so it comes from the same community
    /// service Chatterino uses — which means the request tells a third party
    /// which channels are being watched. That is the reason it is a setting
    /// rather than a constant.
    pub chat_history: usize,
    /// How big every chat's text is drawn: the chat options menu on a pane's
    /// header. One answer for every chat rather than one per pane or per
    /// channel, because it is about the reader's eyes and the screen, which
    /// are the same whichever channel is on. A file from before it existed
    /// loads as [`ChatTextSize::Default`], which is the size chat always was,
    /// and so does a step this build does not know ([`lenient_text_size`]).
    #[serde(default, deserialize_with = "lenient_text_size")]
    pub chat_text_size: ChatTextSize,
    /// Whether every chat message carries its own time at its start, in place
    /// of the once-a-minute breaks: the same menu's toggle. Off by default,
    /// and for a file from before it existed, which is how chat always was.
    #[serde(default)]
    pub chat_message_times: bool,
    /// Whether what is playing keeps playing, in the mini player in the
    /// corner of the browse page, while you browse. The field keeps its
    /// one-word name, which is what settings files already on disk say.
    ///
    /// Off is a real answer rather than a tidiness preference: a stream in the
    /// mini player is still decoding frames and still pulling bytes, and
    /// somebody who goes to the follows page to *pick the next thing* would
    /// rather it stopped. On, it is one click back into what you were watching.
    #[serde(default = "yes")]
    pub miniplayer: bool,
    /// How much of a stacked cell the video keeps, 0.0 for "work it out".
    ///
    /// Chat *beside* the video has a width and the video takes the rest;
    /// chat *below* it has the opposite arrangement, so this is the other half
    /// of [`chat_width`](Self::chat_width) rather than a second copy of it.
    /// Zero means nobody has dragged the divider and the video is a 16:9 box,
    /// which is what it always was.
    #[serde(default)]
    pub video_share: f32,
    /// Whether the follows rail is folded away.
    ///
    /// Remembered because the two ways to use this app want opposite answers:
    /// somebody switching between channels all evening wants the list there,
    /// and somebody watching one stream for three hours wants the window to be
    /// the stream. Neither should have to say so twice.
    #[serde(default)]
    pub sidebar_collapsed: bool,
    /// Where the window was last closed. `None` until it has been closed once,
    /// in which case the app picks a size that fits the display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowPlacement>,
}

/// `#[serde(default)]` for a `bool` is `false`, and this one defaults to true —
/// a settings file written before the field existed should keep the behaviour
/// it had.
fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            quality: QualityPreference::Auto,
            volume: 10,
            credentials: Credentials::default(),
            channel_prefs: BTreeMap::new(),
            recent: Vec::new(),
            pinned: Vec::new(),
            last_channel: None,
            chat_width: 340.0,
            // Enough that a busy channel opens mid-conversation and a quiet one
            // opens with something, without the pane starting scrolled through
            // an hour of backlog nobody asked for.
            chat_history: 100,
            chat_text_size: ChatTextSize::Default,
            chat_message_times: false,
            video_share: 0.0,
            miniplayer: true,
            sidebar_collapsed: false,
            window: None,
        }
    }
}

/// What the settings sheet shows and saves: the client id, the auth-token
/// cookie, quality, chat history and the mini player — and nothing else of
/// [`Settings`].
///
/// The sheet is handed these when it opens ([`SheetFields::of`]) and hands
/// them back when it saves ([`Settings::adopt_sheet`]), so it cannot carry
/// back a field it does not own, however long it was open. One type for both
/// ends, built with a struct literal by the sheet and taken apart in full by
/// `adopt_sheet`: a field the sheet gains does not compile until both sides
/// handle it. Before, a comment at each end was all that kept the two lists
/// of fields in step.
///
/// No `Debug`: the auth-token cookie is a full account credential, and
/// [`Credentials`] keeps it out of logs the same way.
#[derive(Clone, PartialEq, Eq)]
pub struct SheetFields {
    pub client_id: Option<String>,
    pub auth_token: Option<String>,
    pub quality: QualityPreference,
    pub chat_history: usize,
    pub miniplayer: bool,
}

impl SheetFields {
    /// What the sheet shows for `settings` as they are.
    pub fn of(settings: &Settings) -> Self {
        Self {
            client_id: settings.credentials.client_id.clone(),
            auth_token: settings.credentials.auth_token.clone(),
            quality: settings.quality.clone(),
            chat_history: settings.chat_history,
            miniplayer: settings.miniplayer,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} is not valid settings JSON: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

/// Where settings live: wherever this platform keeps configuration a user is
/// expected to keep, unlike the image cache which is throwaway because it is
/// reproducible.
///
/// Each platform is asked in its own branch rather than by falling through one
/// list of variables. The list happened to give the right answer on Windows and
/// on Linux, but it got there by trying `APPDATA` first everywhere — and it had
/// no macOS branch at all, so a Mac landed on `~/.config`. That works; it is
/// simply not where a Mac user, or anything else on the system, would look.
///
/// `app_name` is passed in rather than baked in. This crate knows how settings
/// are *stored*, not what the product is called, and having both this file and
/// the app declare the directory name meant two constants that had to agree or
/// the app would silently start reading a different file from the one it wrote.
pub fn default_path(app_name: &str) -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);

    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support"));

    #[cfg(not(any(windows, target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));

    base.unwrap_or_else(std::env::temp_dir)
        .join(app_name)
        .join("settings.json")
}

/// The text of `path`, or `None` if there is no such file — which on a first
/// run is normal rather than an error.
///
/// A byte-order mark is stripped rather than parsed, because `serde_json` will
/// not have one and every obvious way to hand-edit a file on Windows writes
/// one: Notepad's "UTF-8", PowerShell's `Set-Content -Encoding utf8`, and most
/// of what an editor calls "UTF-8 with signature". Both files here are
/// documented as hand-editable, and refusing one over three invisible bytes
/// would mean the app silently starting without it.
fn read_if_present(path: &Path) -> Result<Option<String>, Error> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(
            text.strip_prefix('\u{feff}')
                .map(str::to_string)
                .unwrap_or(text),
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Write `text` to `path`, creating parent directories as needed.
///
/// Written to a temporary file and renamed, so an interrupted save cannot
/// leave a truncated file behind — which for the settings, holding the
/// sign-in, would mean silently signing the user out.
fn write_atomically(path: &Path, text: &str) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let temp = path.with_extension("json.part");
    std::fs::write(&temp, text).map_err(|source| Error::Write {
        path: temp.clone(),
        source,
    })?;
    std::fs::rename(&temp, path).map_err(|source| Error::Write {
        path: path.to_path_buf(),
        source,
    })
}

impl Settings {
    /// The level `channel` should start at.
    ///
    /// Clamped on the way out as well as in: this file is hand-editable, and a
    /// volume of 400 should be loud rather than a panic.
    pub fn volume_for(&self, channel: &str) -> u8 {
        self.channel_prefs
            .get(&channel_key(channel))
            .and_then(|prefs| prefs.volume)
            .unwrap_or(self.volume)
            .min(100)
    }

    /// Remember `volume` for `channel`, and carry it forward as the default.
    ///
    /// Returns whether anything actually changed, so a caller driven by a
    /// slider does not rewrite the file for a value it already holds.
    ///
    /// Muting is the one level that does *not* become the default. Mute is
    /// something you do to one stream — usually so you can hear another one —
    /// and a channel that opens silent reads as broken rather than as
    /// remembered.
    pub fn set_volume_for(&mut self, channel: &str, volume: u8) -> bool {
        let volume = volume.min(100);
        let mut changed = false;

        if volume > 0 && self.volume != volume {
            self.volume = volume;
            changed = true;
        }

        let entry = self.channel_prefs.entry(channel_key(channel)).or_default();
        if entry.volume != Some(volume) {
            entry.volume = Some(volume);
            changed = true;
        }
        changed
    }

    /// Whether `channel` opens with chat hidden.
    pub fn chat_hidden_for(&self, channel: &str) -> bool {
        self.channel_prefs
            .get(&channel_key(channel))
            .is_some_and(|prefs| prefs.chat_hidden)
    }

    /// Remember whether `channel` shows chat. Returns whether anything changed,
    /// so a no-op toggle does not rewrite the file.
    ///
    /// Deliberately not carried forward as a default the way a volume is — see
    /// [`ChannelPrefs::chat_hidden`].
    pub fn set_chat_hidden_for(&mut self, channel: &str, hidden: bool) -> bool {
        // Checked before an entry exists, not after: `or_default` on a no-op
        // used to leave an empty record behind, which the next save wrote out
        // as a channel with nothing remembered about it.
        if self.chat_hidden_for(channel) == hidden {
            return false;
        }
        let key = channel_key(channel);
        let entry = self.channel_prefs.entry(key.clone()).or_default();
        entry.chat_hidden = hidden;
        if entry.is_empty() {
            self.channel_prefs.remove(&key);
        }
        true
    }

    /// Put `channel` at the front of the recently watched list.
    ///
    /// Returns whether anything changed, so opening the channel that is
    /// already first does not rewrite the file. Keyed like everything else,
    /// so `Forsen` and `forsen` are one entry.
    pub fn note_watched(&mut self, channel: &str) -> bool {
        let key = channel_key(channel);
        if self.recent.first() == Some(&key) {
            return false;
        }
        self.recent.retain(|login| *login != key);
        self.recent.insert(0, key);
        self.recent.truncate(RECENT_LIMIT);
        true
    }

    /// Whether `channel` is pinned to the top of the rail.
    pub fn is_pinned(&self, channel: &str) -> bool {
        self.pinned.contains(&channel_key(channel))
    }

    /// Pin `channel` to the top of the rail, after the ones already there, or
    /// take it off.
    ///
    /// Returns whether anything changed, like [`note_watched`](Self::note_watched),
    /// so pinning what is already pinned does not rewrite the file. Keyed like
    /// everything else, so `Forsen` and `forsen` are one pin. Something that
    /// could not be a login changes nothing: `tidy_pinned` would only drop it
    /// again on the way to disk.
    pub fn set_pinned(&mut self, channel: &str, pinned: bool) -> bool {
        let key = channel_key(channel);
        if pinned {
            if !is_login(&key) || self.pinned.contains(&key) {
                return false;
            }
            self.pinned.push(key);
            true
        } else {
            let before = self.pinned.len();
            self.pinned.retain(|login| *login != key);
            self.pinned.len() != before
        }
    }

    /// Take what the settings sheet saved, and change nothing else.
    ///
    /// The sheet is open for as long as somebody leaves it, and the app can
    /// go on writing the live settings meanwhile: a handed-over launch puts
    /// the channel it opens into `recent`. The sheet used to work on a copy
    /// of the whole of `Settings` taken when it opened, and handing all of it
    /// back put that back the way it was. Now it only ever holds
    /// [`SheetFields`], so everything else the app writes outside the sheet,
    /// a pin or a volume as much as a recent channel, now or in a later
    /// build, survives saving it.
    pub fn adopt_sheet(&mut self, sheet: &SheetFields) {
        // Named in full, with no `..`, so a field the sheet gains does not
        // compile until it is taken here too.
        let SheetFields {
            client_id,
            auth_token,
            quality,
            chat_history,
            miniplayer,
        } = sheet;
        self.credentials.client_id = client_id.clone();
        self.credentials.auth_token = auth_token.clone();
        self.quality = quality.clone();
        self.chat_history = *chat_history;
        self.miniplayer = *miniplayer;
    }

    /// Fold a pre-0.2.2 file's single last channel into the list.
    fn adopt_last_channel(&mut self) {
        if let Some(last) = self.last_channel.take() {
            if self.recent.is_empty() {
                self.recent.push(channel_key(&last));
            }
        }
    }

    /// Drop per-channel entries that remember nothing, or that are not about a
    /// channel at all.
    ///
    /// Run on load and before every save, so a file written by an older build
    /// that left such entries behind is cleaned on its way through. Builds up
    /// to 0.2.1 remembered a recording's volume against the recording's pane
    /// key — `vod:2860004234` — which nothing ever read back; a login is
    /// letters, digits and underscores, so anything else is one of those.
    fn prune_empty_prefs(&mut self) {
        self.channel_prefs
            .retain(|key, prefs| !prefs.is_empty() && is_login(key));
    }

    /// Put the pins in the one shape the rail reads them in: keyed by
    /// [`channel_key`], logins only, each once, the first mention keeping its
    /// place.
    ///
    /// Run on load and before every write, like `prune_empty_prefs`. Pins can
    /// be written into the file by hand, and `"#Forsen"` beside `"forsen"`
    /// would otherwise be two rows for one channel, and a typo with a space
    /// in it a row for nobody.
    fn tidy_pinned(&mut self) {
        let mut tidy: Vec<String> = Vec::with_capacity(self.pinned.len());
        for key in self.pinned.iter().map(|pin| channel_key(pin)) {
            if is_login(&key) && !tidy.contains(&key) {
                tidy.push(key);
            }
        }
        self.pinned = tidy;
    }

    /// Load from `path`, returning defaults if the file does not exist yet.
    ///
    /// A missing file is normal on first run and is not an error. A corrupt
    /// file *is* an error rather than a silent reset, because silently
    /// discarding someone's credentials is worse than refusing to start.
    pub fn load(path: &Path) -> Result<Self, Error> {
        // A byte-order mark is stripped on the way in; see `read_if_present`.
        // Refusing one here would be worse than for any other file: the app
        // would start on defaults and then be unable to save either, since
        // every write reads this file back first to keep the tokens another
        // thread put there.
        let Some(text) = read_if_present(path)? else {
            return Ok(Self::default());
        };

        let mut settings: Self = serde_json::from_str(&text).map_err(|source| Error::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        settings.prune_empty_prefs();
        settings.tidy_pinned();
        settings.adopt_last_channel();
        Ok(settings)
    }

    /// Save these settings, keeping whatever sign-in is already on disk.
    ///
    /// The UI holds a snapshot of `Settings` taken at launch, but the sign-in
    /// worker writes OAuth tokens to the same file from its own thread. Saving
    /// that snapshot with [`save`](Self::save) put `oauth` back to whatever it
    /// was at startup — erasing a fresh sign-in, and worse, restoring an
    /// already-spent refresh token, which Twitch honours exactly once. Either
    /// way the next launch had to run the device flow again.
    ///
    /// Everything the UI owns is written as-is, so adding a field needs no
    /// change here. Only the field somebody else owns is named.
    pub fn save_preferences(&self, path: &Path) -> Result<(), Error> {
        let _guard = file_lock();
        let mut out = self.clone();
        out.credentials.oauth = Self::load(path)?.credentials.oauth;
        out.write(path)
    }

    /// Save these settings and discard any stored sign-in.
    ///
    /// Tokens are issued against one client id, so changing that id makes them
    /// useless. Dropping them turns the next sign-in into a clean prompt rather
    /// than a confusing "sign-in expired".
    pub fn save_forgetting_sign_in(&self, path: &Path) -> Result<(), Error> {
        let _guard = file_lock();
        let mut out = self.clone();
        out.credentials.oauth = None;
        out.write(path)
    }

    /// Record a sign-in, keeping everything else the file already says.
    ///
    /// The mirror image of [`save_preferences`](Self::save_preferences), for
    /// the thread that owns the tokens and nothing else: it reads the file,
    /// replaces the one field it owns, and writes it back, all under the same
    /// lock the UI's saves take. Twitch refresh tokens are single-use, so this
    /// has to land, and land intact, every time one is issued.
    pub fn save_sign_in(path: &Path, oauth: Option<OAuthTokens>) -> Result<(), Error> {
        let _guard = file_lock();
        let mut settings = Self::load(path)?;
        settings.credentials.oauth = oauth;
        settings.write(path)
    }

    /// Forget the stored sign-in, keeping everything else the file says:
    /// the settings sheet's Sign out, and the first half of signing in
    /// again for a scope the old sign-in lacks.
    ///
    /// [`save_sign_in`](Self::save_sign_in) with nothing, under its lock and
    /// its re-read, so a preference saved a moment ago survives it, and
    /// named apart so a reader finds the one way the UI clears the tokens
    /// without changing the client id. The caller stops the sign-in worker
    /// first: one still running would persist its next refresh over this.
    pub fn sign_out(path: &Path) -> Result<(), Error> {
        Self::save_sign_in(path, None)
    }

    /// Write to `path` as-is, creating parent directories as needed.
    ///
    /// Everything in `self` wins, including the sign-in. That is right for a
    /// test or a first write and wrong for the running app, whose two writers
    /// each own different fields; they use the two `save_*` methods above.
    pub fn save(&self, path: &Path) -> Result<(), Error> {
        let _guard = file_lock();
        self.write(path)
    }

    /// The write itself, with the lock already held. Atomic; see
    /// `write_atomically`.
    fn write(&self, path: &Path) -> Result<(), Error> {
        let mut out = self.clone();
        out.prune_empty_prefs();
        out.tidy_pinned();
        let text = serde_json::to_string_pretty(&out).expect("settings are always serialisable");
        write_atomically(path, &text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A settings file saved by Notepad, PowerShell, or anything else on
    /// Windows that calls a leading byte-order mark "UTF-8".
    ///
    /// This is not hypothetical: the file is documented as hand-editable, and
    /// the failure it caused was silent twice over — the app started on
    /// defaults, and then every save failed too, because a save reads the file
    /// back first to keep somebody else's tokens.
    #[test]
    fn a_byte_order_mark_does_not_make_the_file_unreadable() {
        let path = temp_file("bom");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();

        let settings = Settings {
            volume: 42,
            ..Settings::default()
        };
        settings.save(&path).unwrap();

        let json = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("\u{feff}{json}")).unwrap();

        let loaded = Settings::load(&path).expect("a BOM should not be fatal");
        assert_eq!(loaded.volume, 42);
        let _ = std::fs::remove_file(&path);
    }

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join("perch-tests")
            .join(format!("{name}.json"))
    }

    #[test]
    fn missing_file_yields_defaults() {
        let path = temp_file("definitely-absent");
        let _ = std::fs::remove_file(&path);
        let settings = Settings::load(&path).unwrap();
        assert_eq!(settings, Settings::default());
        assert_eq!(settings.volume, 10);
        assert_eq!(settings.quality, QualityPreference::Auto);
    }

    #[test]
    fn round_trips_through_disk() {
        let path = temp_file("round-trip");
        let settings = Settings {
            volume: 42,
            quality: QualityPreference::Fixed("720p60".into()),
            recent: vec!["forsen".into()],
            channel_prefs: BTreeMap::from([(
                "forsen".to_string(),
                ChannelPrefs {
                    volume: Some(35),
                    ..Default::default()
                },
            )]),
            credentials: Credentials {
                client_id: Some("abc123".into()),
                ..Credentials::default()
            },
            ..Settings::default()
        };

        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), settings);
        let _ = std::fs::remove_file(&path);
    }

    fn a_sign_in() -> OAuthTokens {
        OAuthTokens {
            access_token: "access".into(),
            refresh_token: "refresh".into(),
            expires_at: 4_102_444_800,
            user_id: "1234".into(),
            login: "someone".into(),
        }
    }

    /// The bug this exists to prevent: the worker signs in on its own thread
    /// after the UI has already read the file, so the UI's copy has no tokens
    /// in it. Writing that copy back wholesale erased the sign-in, and the next
    /// launch asked for the device code all over again.
    #[test]
    fn saving_preferences_keeps_a_sign_in_written_since_launch() {
        let path = temp_file("preferences-keep-sign-in");
        let _ = std::fs::remove_file(&path);

        // What the UI read at launch.
        let at_launch = Settings::default();
        at_launch.save(&path).unwrap();

        // The worker signs in and persists, while the UI holds its snapshot.
        let mut signed_in = Settings::load(&path).unwrap();
        signed_in.credentials.oauth = Some(a_sign_in());
        signed_in.save(&path).unwrap();

        // The UI saves a preference from its now-stale copy.
        let mut ui = at_launch.clone();
        ui.volume = 42;
        ui.save_preferences(&path).unwrap();

        let stored = Settings::load(&path).unwrap();
        assert_eq!(stored.volume, 42, "the preference did not take");
        assert!(
            stored.credentials.oauth.is_some(),
            "saving a preference erased the sign-in"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// Tokens belong to the client id they were issued against, so changing it
    /// has to drop them rather than leave a sign-in that can only fail.
    #[test]
    fn forgetting_a_sign_in_drops_the_tokens() {
        let path = temp_file("preferences-forget-sign-in");
        let _ = std::fs::remove_file(&path);

        let mut settings = Settings::default();
        settings.credentials.oauth = Some(a_sign_in());
        settings.save(&path).unwrap();

        settings.credentials.client_id = Some("a-different-app".into());
        settings.save_forgetting_sign_in(&path).unwrap();

        let stored = Settings::load(&path).unwrap();
        assert!(stored.credentials.oauth.is_none());
        assert_eq!(
            stored.credentials.client_id.as_deref(),
            Some("a-different-app")
        );
        let _ = std::fs::remove_file(&path);
    }

    /// Signing out drops the tokens and nothing else: a preference saved
    /// from a stale copy just before it, and the client id, both survive.
    #[test]
    fn signing_out_drops_the_tokens_and_keeps_the_rest() {
        let path = temp_file("sign-out");
        let _ = std::fs::remove_file(&path);

        let mut settings = Settings::default();
        settings.credentials.client_id = Some("the-app".into());
        settings.credentials.oauth = Some(a_sign_in());
        settings.save(&path).unwrap();

        let mut ui = settings.clone();
        ui.credentials.oauth = None;
        ui.volume = 42;
        ui.save_preferences(&path).unwrap();

        Settings::sign_out(&path).unwrap();

        let stored = Settings::load(&path).unwrap();
        assert!(stored.credentials.oauth.is_none(), "still signed in");
        assert_eq!(stored.volume, 42, "signing out lost a preference");
        assert_eq!(stored.credentials.client_id.as_deref(), Some("the-app"));
        let _ = std::fs::remove_file(&path);
    }

    /// Newest first, no repeats, and a bounded length: the three things a
    /// "recently watched" list has to get right.
    #[test]
    fn recently_watched_is_newest_first_without_repeats() {
        let mut settings = Settings::default();
        assert!(settings.note_watched("forsen"));
        assert!(settings.note_watched("xqc"));
        assert_eq!(settings.recent, ["xqc", "forsen"]);
        assert!(!settings.note_watched("xqc"), "already first");
        assert!(settings.note_watched("Forsen"));
        assert_eq!(settings.recent, ["forsen", "xqc"], "one entry per channel");

        for n in 0..(RECENT_LIMIT * 2) {
            settings.note_watched(&format!("channel{n}"));
        }
        assert_eq!(settings.recent.len(), RECENT_LIMIT);
        assert_eq!(
            settings.recent[0],
            format!("channel{}", RECENT_LIMIT * 2 - 1)
        );
    }

    /// Builds up to 0.2.1 wrote one `last_channel` and read it back nowhere.
    /// It becomes the list's first entry, once, and is not written again.
    #[test]
    fn an_old_last_channel_becomes_the_recent_list() {
        let path = temp_file("last-channel");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"last_channel": "Forsen"}"#).unwrap();

        let loaded = Settings::load(&path).unwrap();
        assert_eq!(loaded.recent, ["forsen"]);

        loaded.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("last_channel"),
            "the old field came back: {text}"
        );
        assert!(
            text.contains("\"recent\""),
            "the list was not written: {text}"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// A file from an older build must not reset every other field.
    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"volume": 55}"#).unwrap();
        assert_eq!(settings.volume, 55);
        assert_eq!(settings.quality, QualityPreference::Auto);
        assert_eq!(settings.chat_width, 340.0);
        assert_eq!(settings.chat_history, 100);
        assert_eq!(settings.chat_text_size, ChatTextSize::Default);
        assert!(!settings.chat_message_times);
        assert!(settings.channel_prefs.is_empty());
        assert!(settings.pinned.is_empty());
    }

    /// The chat options go to disk by name and come back as they were set.
    #[test]
    fn chat_display_options_round_trip_by_name() {
        let path = temp_file("chat-display");
        let _ = std::fs::remove_file(&path);
        let settings = Settings {
            chat_text_size: ChatTextSize::Larger,
            chat_message_times: true,
            ..Settings::default()
        };
        settings.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(r#""chat_text_size": "larger""#), "got {text}");
        let loaded = Settings::load(&path).unwrap();
        assert_eq!(loaded.chat_text_size, ChatTextSize::Larger);
        assert!(loaded.chat_message_times);
        let _ = std::fs::remove_file(&path);
    }

    /// A text size this build does not know, from a newer build or a hand
    /// edit, loads as Default and takes nothing else in the file down with
    /// it.
    #[test]
    fn an_unknown_chat_text_size_loads_as_default() {
        for size in [r#""largest""#, r#""Larger""#, "18", "null"] {
            let text = format!(
                r#"{{"volume": 55, "chat_text_size": {size}, "chat_message_times": true}}"#
            );
            let settings: Settings = serde_json::from_str(&text).unwrap();
            assert_eq!(settings.chat_text_size, ChatTextSize::Default, "{size}");
            assert_eq!(settings.volume, 55, "{size}");
            assert!(settings.chat_message_times, "{size}");
        }
        let known: Settings = serde_json::from_str(r#"{"chat_text_size": "small"}"#).unwrap();
        assert_eq!(known.chat_text_size, ChatTextSize::Small);
    }

    /// Pins go to disk and come back in the order they were made, which is
    /// the order the rail shows them in.
    #[test]
    fn pins_round_trip_through_disk() {
        let path = temp_file("pins-round-trip");
        let mut settings = Settings::default();
        assert!(settings.set_pinned("xqc", true));
        assert!(settings.set_pinned("forsen", true));
        settings.save(&path).unwrap();

        let loaded = Settings::load(&path).unwrap();
        assert_eq!(loaded.pinned, ["xqc", "forsen"]);
        assert!(loaded.is_pinned("forsen"));
        assert_eq!(loaded, settings);
        let _ = std::fs::remove_file(&path);
    }

    /// A file from before pins existed has none, and a file with none does
    /// not grow an empty list on its way through.
    #[test]
    fn a_file_without_pins_loads_none() {
        let path = temp_file("no-pins");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"volume": 30}"#).unwrap();

        let loaded = Settings::load(&path).unwrap();
        assert!(loaded.pinned.is_empty());
        assert!(!loaded.is_pinned("forsen"));

        loaded.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("pinned"),
            "an empty list was written: {text}"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// One key rule for pins as for everything else, or `Forsen` from the
    /// command line and `forsen` from Helix would be two pins for one channel.
    #[test]
    fn pin_keys_ignore_case_and_a_leading_hash() {
        let mut settings = Settings::default();
        assert!(settings.set_pinned("#Forsen", true));
        assert!(settings.is_pinned("forsen"));
        assert!(settings.is_pinned("FORSEN"));
        assert!(
            !settings.set_pinned("forsen", true),
            "the same channel pinned twice"
        );
        assert_eq!(settings.pinned, ["forsen"]);

        assert!(settings.set_pinned("FORSEN", false));
        assert!(settings.pinned.is_empty());
    }

    /// A repeat is not a change, so it neither rewrites the file nor moves a
    /// pin; a new pin goes last; and something that is not a login is not
    /// pinned at all.
    #[test]
    fn pinning_twice_changes_nothing_and_keeps_pin_order() {
        let mut settings = Settings::default();
        assert!(settings.set_pinned("a", true));
        assert!(settings.set_pinned("b", true));
        assert!(settings.set_pinned("c", true));
        assert!(!settings.set_pinned("a", true), "already pinned");
        assert_eq!(settings.pinned, ["a", "b", "c"], "a repeat moved a pin");

        assert!(settings.set_pinned("b", false));
        assert!(!settings.set_pinned("b", false), "already unpinned");
        assert_eq!(settings.pinned, ["a", "c"]);

        assert!(settings.set_pinned("b", true));
        assert_eq!(settings.pinned, ["a", "c", "b"], "a new pin goes last");

        assert!(!settings.set_pinned("vod:2860004234", true), "not a login");
        assert_eq!(settings.pinned, ["a", "c", "b"]);
    }

    /// Pins written into the file by hand are tidied on the way in: one key
    /// rule, logins only, each channel once where it was first named.
    #[test]
    fn pins_that_are_not_logins_are_dropped_on_load() {
        let path = temp_file("pins-not-logins");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r##"{"pinned": ["Forsen", "vod:2860004234", "", "two words", "#forsen", "xqc"]}"##,
        )
        .unwrap();

        let loaded = Settings::load(&path).unwrap();
        assert_eq!(loaded.pinned, ["forsen", "xqc"]);
        let _ = std::fs::remove_file(&path);
    }

    /// The sign-in trap again, for pins: the UI's copy is stale by the time it
    /// saves, and saving it must keep the pin it made and the sign-in it did
    /// not.
    #[test]
    fn saving_preferences_keeps_pins() {
        let path = temp_file("preferences-keep-pins");
        let _ = std::fs::remove_file(&path);

        let at_launch = Settings::default();
        at_launch.save(&path).unwrap();

        let mut signed_in = Settings::load(&path).unwrap();
        signed_in.credentials.oauth = Some(a_sign_in());
        signed_in.save(&path).unwrap();

        let mut ui = at_launch.clone();
        ui.set_pinned("forsen", true);
        ui.save_preferences(&path).unwrap();

        let stored = Settings::load(&path).unwrap();
        assert_eq!(stored.pinned, ["forsen"]);
        assert!(stored.credentials.oauth.is_some());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_channel_with_no_level_of_its_own_uses_the_default() {
        let settings = Settings::default();
        assert_eq!(settings.volume_for("forsen"), settings.volume);
    }

    /// The whole point: coming back to a channel finds it where you left it,
    /// and does not drag every other channel along with it.
    #[test]
    fn a_remembered_channel_keeps_its_own_level() {
        let mut settings = Settings::default();
        assert!(settings.set_volume_for("forsen", 42));
        assert_eq!(settings.volume_for("forsen"), 42);
        // Setting the same value again is not a reason to rewrite the file.
        assert!(!settings.set_volume_for("forsen", 42));
    }

    /// The app does not agree with itself about case — Helix says `forsen`,
    /// the command line says whatever was typed — so without one key rule the
    /// same streamer accumulates an entry per spelling.
    #[test]
    fn channel_keys_ignore_case_and_a_leading_hash() {
        let mut settings = Settings::default();
        settings.set_volume_for("Forsen", 42);
        assert_eq!(settings.volume_for("forsen"), 42);
        assert_eq!(settings.volume_for("#FORSEN"), 42);
        assert_eq!(settings.channel_prefs.len(), 1);
    }

    /// Muting one stream to hear another must not make silence the default,
    /// and must still be remembered for the stream it was done to.
    /// Hiding chat is per channel and never becomes a default, for the same
    /// reason muting does not: it is a statement about one stream.
    #[test]
    fn hiding_chat_is_remembered_per_channel_only() {
        let mut settings = Settings::default();
        assert!(!settings.chat_hidden_for("forsen"));

        assert!(settings.set_chat_hidden_for("forsen", true));
        assert!(settings.chat_hidden_for("forsen"));
        assert!(
            !settings.chat_hidden_for("xqc"),
            "hiding one channel's chat hid another's"
        );

        // A repeated value is not a reason to rewrite the file.
        assert!(!settings.set_chat_hidden_for("forsen", true));
        assert!(settings.set_chat_hidden_for("forsen", false));
        assert!(!settings.chat_hidden_for("forsen"));
    }

    /// It travels with the channel's other preferences, and shares their
    /// case-insensitive key.
    #[test]
    fn hidden_chat_round_trips_and_ignores_case() {
        let path = temp_file("chat-hidden");
        let mut settings = Settings::default();
        settings.set_chat_hidden_for("Forsen", true);
        settings.set_volume_for("Forsen", 40);
        settings.save(&path).unwrap();

        let loaded = Settings::load(&path).unwrap();
        assert!(loaded.chat_hidden_for("forsen"));
        assert!(loaded.chat_hidden_for("#FORSEN"));
        assert_eq!(loaded.volume_for("forsen"), 40);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn muting_is_remembered_but_never_becomes_the_default() {
        let mut settings = Settings::default();
        settings.set_volume_for("forsen", 60);
        settings.set_volume_for("forsen", 0);

        assert_eq!(settings.volume_for("forsen"), 0, "the mute was forgotten");
        assert_eq!(settings.volume, 60, "muting one channel muted the default");
        assert_eq!(
            settings.volume_for("never-opened"),
            60,
            "a new channel would have opened silent"
        );
    }

    /// Hand-editable file, so a nonsense level is clamped rather than trusted.
    #[test]
    fn an_impossible_level_is_clamped_on_the_way_out() {
        let settings: Settings =
            serde_json::from_str(r#"{"channel_prefs": {"forsen": {"volume": 250}}}"#).unwrap();
        assert_eq!(settings.volume_for("forsen"), 100);
    }

    /// Same class of bug as the sign-in erasure above: the UI's snapshot is
    /// stale by the time it saves, and only the sign-in is somebody else's.
    #[test]
    fn saving_preferences_keeps_channel_levels() {
        let path = temp_file("preferences-keep-volumes");
        let _ = std::fs::remove_file(&path);

        let at_launch = Settings::default();
        at_launch.save(&path).unwrap();

        let mut signed_in = Settings::load(&path).unwrap();
        signed_in.credentials.oauth = Some(a_sign_in());
        signed_in.save(&path).unwrap();

        let mut ui = at_launch.clone();
        ui.set_volume_for("forsen", 42);
        ui.save_preferences(&path).unwrap();

        let stored = Settings::load(&path).unwrap();
        assert_eq!(stored.volume_for("forsen"), 42);
        assert!(stored.credentials.oauth.is_some());
        let _ = std::fs::remove_file(&path);
    }

    /// What the settings sheet saves is as old as the sheet. Adopting it
    /// takes the five fields the sheet shows, and leaves every other field as
    /// the live settings have it: a recent channel, which can change while
    /// the sheet is open, and a pin, the rail, the window and a channel's
    /// level, which cannot today but are not the sheet's either.
    #[test]
    fn adopting_the_sheet_keeps_everything_it_does_not_own() {
        let mut live = Settings::default();
        let mut sheet = SheetFields::of(&live);

        // Everything the sheet does not own moves on in the live copy.
        live.set_pinned("forsen", true);
        live.note_watched("xqc");
        live.window = Some(WindowPlacement {
            x: 10.0,
            y: 20.0,
            width: 1280.0,
            height: 720.0,
            maximized: false,
        });
        live.sidebar_collapsed = true;
        live.chat_text_size = ChatTextSize::Large;
        live.chat_message_times = true;
        live.set_volume_for("forsen", 42);
        live.credentials.oauth = Some(a_sign_in());

        // And the sheet is saved with its own changes.
        sheet.client_id = Some("a-new-app".into());
        sheet.auth_token = Some("a-new-cookie".into());
        sheet.quality = QualityPreference::Fixed("720p".into());
        sheet.chat_history = 0;
        sheet.miniplayer = false;

        let before = live.clone();
        live.adopt_sheet(&sheet);

        assert_eq!(live.credentials.client_id.as_deref(), Some("a-new-app"));
        assert_eq!(live.credentials.auth_token.as_deref(), Some("a-new-cookie"));
        assert_eq!(live.quality, QualityPreference::Fixed("720p".into()));
        assert_eq!(live.chat_history, 0);
        assert!(!live.miniplayer);

        assert_eq!(live.pinned, ["forsen"], "saving the sheet unpinned");
        assert_eq!(live.recent, ["xqc"], "saving the sheet forgot a channel");
        assert_eq!(
            live.window, before.window,
            "saving the sheet moved the window"
        );
        assert!(live.sidebar_collapsed, "saving the sheet unfolded the rail");
        assert_eq!(
            live.chat_text_size,
            ChatTextSize::Large,
            "saving the sheet reset chat's text size"
        );
        assert!(
            live.chat_message_times,
            "saving the sheet took the times off chat"
        );
        assert_eq!(live.volume_for("forsen"), 42);
        assert_eq!(live.volume, before.volume);
        assert!(
            live.credentials.oauth.is_some(),
            "the sheet owns no sign-in"
        );

        // Everything but the five, exactly as it was.
        let mut untouched = live.clone();
        untouched.adopt_sheet(&SheetFields::of(&before));
        assert_eq!(untouched, before);
    }

    /// A sheet saved without a change changes nothing: what it is handed is
    /// exactly what adopting it puts back.
    #[test]
    fn a_sheet_saved_as_it_opened_changes_nothing() {
        let mut settings = Settings::default();
        settings.credentials.client_id = Some("an-app".into());
        settings.credentials.auth_token = Some("a-cookie".into());
        settings.quality = QualityPreference::Fixed("1080p".into());
        settings.chat_history = 250;
        settings.miniplayer = false;
        settings.set_pinned("forsen", true);

        let before = settings.clone();
        settings.adopt_sheet(&SheetFields::of(&before));
        assert_eq!(settings, before);
    }

    /// A file from a newer build must not fail to load here.
    #[test]
    fn unknown_fields_are_ignored() {
        let settings: Settings =
            serde_json::from_str(r#"{"volume": 7, "future_option": {"a": 1}}"#).unwrap();
        assert_eq!(settings.volume, 7);
    }

    #[test]
    fn quality_preference_serialises_legibly() {
        let json = serde_json::to_string(&QualityPreference::Fixed("1080p60".into())).unwrap();
        assert!(json.contains("fixed"), "got {json}");
        assert!(json.contains("1080p60"), "got {json}");

        let auto = serde_json::to_string(&QualityPreference::Auto).unwrap();
        assert!(auto.contains("auto"), "got {auto}");
    }

    /// A recording's volume was remembered against its pane key, where
    /// nothing read it back. Those entries go on load, and only those.
    #[test]
    fn remembered_prefs_that_are_not_about_a_channel_are_dropped() {
        let path = temp_file("not-a-channel");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"channel_prefs": {
                "vod:2860004234": {"volume": 68},
                "spicy_sushi_poe": {"volume": 51},
                "Forsen": {"chat_hidden": true}
            }}"#,
        )
        .unwrap();

        let settings = Settings::load(&path).unwrap();
        assert!(!settings.channel_prefs.contains_key("vod:2860004234"));
        assert!(settings.channel_prefs.contains_key("spicy_sushi_poe"));
        assert!(
            settings.channel_prefs.contains_key("Forsen"),
            "a login from before keys were lowercased is still a login"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// A pane's quality menu names the preference it hands the pane back to,
    /// so the name has to be the word the preference is stored as.
    #[test]
    fn a_quality_preference_is_named_the_way_it_is_stored() {
        assert_eq!(QualityPreference::Auto.name(), "auto");
        assert_eq!(QualityPreference::Fixed("best".into()).name(), "best");
        assert_eq!(QualityPreference::Fixed("1080p".into()).name(), "1080p");
    }

    /// Toggling chat off and back on again for a channel used to leave an
    /// entry behind that remembered nothing, and the next save wrote it out as
    /// `"volume": null`. The file is documented as hand-editable, and a record
    /// with nothing in it is the kind of thing a reader stops to wonder about.
    #[test]
    fn a_toggle_that_changes_nothing_leaves_no_entry_behind() {
        let mut settings = Settings::default();
        assert!(!settings.set_chat_hidden_for("forsen", false));
        assert!(
            settings.channel_prefs.is_empty(),
            "a no-op created an entry"
        );

        assert!(settings.set_chat_hidden_for("forsen", true));
        assert!(settings.set_chat_hidden_for("forsen", false));
        assert!(
            settings.channel_prefs.is_empty(),
            "hiding and unhiding left an empty entry"
        );
    }

    #[test]
    fn empty_entries_are_pruned_on_the_way_through() {
        let path = temp_file("empty-prefs");
        std::fs::write(
            &path,
            r#"{"channel_prefs":{"ghost":{"volume":null},"real":{"volume":30}}}"#,
        )
        .unwrap();

        let loaded = Settings::load(&path).unwrap();
        assert!(!loaded.channel_prefs.contains_key("ghost"));
        assert_eq!(loaded.volume_for("real"), 30);

        loaded.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("ghost"), "the empty entry came back: {text}");
        assert!(
            !text.contains("\"volume\": null"),
            "a None level was written out: {text}"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// The worker's save and the UI's save each keep the other's fields, and
    /// neither may lose what the other wrote while it was in the middle of a
    /// read-modify-write. Hammered from two threads rather than reasoned
    /// about: with the lock removed this fails within a few hundred rounds.
    #[test]
    fn two_writers_never_lose_each_others_fields() {
        let path = temp_file("two-writers");
        let _ = std::fs::remove_file(&path);
        Settings::default().save(&path).unwrap();

        const ROUNDS: u8 = 200;
        let ui = std::thread::spawn({
            let path = path.clone();
            move || {
                let mut settings = Settings::default();
                for round in 1..=ROUNDS {
                    settings.volume = round;
                    settings.save_preferences(&path).unwrap();
                }
            }
        });
        let worker = std::thread::spawn({
            let path = path.clone();
            move || {
                for round in 1..=ROUNDS {
                    let tokens = OAuthTokens {
                        refresh_token: format!("refresh-{round}"),
                        ..Default::default()
                    };
                    Settings::save_sign_in(&path, Some(tokens)).unwrap();
                }
            }
        });
        ui.join().unwrap();
        worker.join().unwrap();

        let after = Settings::load(&path).unwrap();
        assert_eq!(after.volume, ROUNDS, "the UI's last save was lost");
        assert_eq!(
            after.credentials.oauth.map(|t| t.refresh_token),
            Some(format!("refresh-{ROUNDS}")),
            "the worker's last token was overwritten by an older one"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// One `dbg!` on a `Settings` must not put an account credential in the
    /// log. The client id is public and may show; the tokens may not.
    #[test]
    fn debug_output_never_contains_a_secret() {
        let settings = Settings {
            credentials: Credentials {
                client_id: Some("public-client-id".into()),
                auth_token: Some("cookie-secret".into()),
                oauth: Some(OAuthTokens {
                    access_token: "access-secret".into(),
                    refresh_token: "refresh-secret".into(),
                    login: "someone".into(),
                    ..Default::default()
                }),
            },
            ..Default::default()
        };
        let text = format!("{settings:?}");
        for secret in ["cookie-secret", "access-secret", "refresh-secret"] {
            assert!(!text.contains(secret), "{secret} leaked into: {text}");
        }
        assert!(text.contains("public-client-id"));
        assert!(text.contains("someone"));
    }

    #[test]
    fn a_window_placement_round_trips_and_rejects_nonsense() {
        let path = temp_file("window");
        let settings = Settings {
            window: Some(WindowPlacement {
                x: 10.0,
                y: 20.0,
                width: 1280.0,
                height: 720.0,
                maximized: true,
            }),
            ..Default::default()
        };
        settings.save(&path).unwrap();
        let loaded = Settings::load(&path).unwrap();
        assert_eq!(loaded.window, settings.window);
        assert!(loaded.window.unwrap().is_usable());

        let tiny = WindowPlacement {
            width: 10.0,
            height: 10.0,
            ..Default::default()
        };
        assert!(!tiny.is_usable());
        let nan = WindowPlacement {
            x: f32::NAN,
            width: 800.0,
            height: 600.0,
            ..Default::default()
        };
        assert!(!nan.is_usable());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn corrupt_file_is_an_error_not_a_silent_reset() {
        let path = temp_file("corrupt");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not json").unwrap();
        assert!(matches!(Settings::load(&path), Err(Error::Parse { .. })));
        let _ = std::fs::remove_file(&path);
    }
}
