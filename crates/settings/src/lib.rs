//! Persisted user settings.
//!
//! Stored as JSON next to the user's other roaming app data. Unknown fields are
//! ignored and missing fields fall back to defaults, so a settings file written
//! by an older or newer build still loads rather than resetting everything.
//!
//! Credentials live here too. They are stored in plain text, which is the same
//! thing every desktop Twitch client does, but it is a deliberate choice rather
//! than an oversight — see [`Credentials`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Credentials {
    /// Twitch application client id, from dev.twitch.tv. Required for sign-in.
    pub client_id: Option<String>,
    /// The twitch.tv `auth-token` cookie. Optional.
    pub auth_token: Option<String>,
    /// Helix tokens from device-code sign-in.
    pub oauth: Option<OAuthTokens>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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

/// How a channel is identified in [`Settings::channel_prefs`].
///
/// Twitch logins are case-insensitive and the app does not agree with itself
/// about case: the browse page hands over Helix's lowercase `user_login`, while
/// the command line hands over whatever was typed. Normalising in one place is
/// what stops `Forsen` and `forsen` remembering two different levels.
pub fn channel_key(channel: &str) -> String {
    channel.trim_start_matches('#').to_ascii_lowercase()
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
    /// Reopened on launch when no channel is given on the command line.
    pub last_channel: Option<String>,
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
    /// Whether what is playing keeps playing while you browse.
    ///
    /// Off is a real answer rather than a tidiness preference: a backgrounded
    /// stream is still decoding frames and still pulling bytes, and somebody
    /// who goes to the follows page to *pick the next thing* would rather it
    /// stopped. On, it is one click back into what you were watching.
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
            last_channel: None,
            chat_width: 340.0,
            // Enough that a busy channel opens mid-conversation and a quiet one
            // opens with something, without the pane starting scrolled through
            // an hour of backlog nobody asked for.
            chat_history: 100,
            video_share: 0.0,
            miniplayer: true,
            sidebar_collapsed: false,
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
        let entry = self.channel_prefs.entry(channel_key(channel)).or_default();
        if entry.chat_hidden == hidden {
            return false;
        }
        entry.chat_hidden = hidden;
        true
    }

    /// Load from `path`, returning defaults if the file does not exist yet.
    ///
    /// A missing file is normal on first run and is not an error. A corrupt
    /// file *is* an error rather than a silent reset, because silently
    /// discarding someone's credentials is worse than refusing to start.
    pub fn load(path: &Path) -> Result<Self, Error> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => {
                return Err(Error::Read {
                    path: path.to_path_buf(),
                    source,
                })
            }
        };
        // A byte-order mark is stripped rather than parsed, because
        // `serde_json` will not have one and every obvious way to hand-edit
        // this file on Windows writes one: Notepad's "UTF-8", PowerShell's
        // `Set-Content -Encoding utf8`, and most of what an editor calls
        // "UTF-8 with signature". The file is documented as hand-editable, so
        // refusing it over three invisible bytes means the app silently starts
        // on defaults — and then cannot save either, since every write reads
        // the file back first to keep the tokens another thread put there.
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);

        serde_json::from_str(text).map_err(|source| Error::Parse {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Write to `path`, creating parent directories as needed.
    ///
    /// Writes to a temporary file and renames, so an interrupted save cannot
    /// leave truncated settings — which for a file holding credentials would
    /// mean silently signing the user out.
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
        let mut out = self.clone();
        out.credentials.oauth = Self::load(path)?.credentials.oauth;
        out.save(path)
    }

    /// Save these settings and discard any stored sign-in.
    ///
    /// Tokens are issued against one client id, so changing that id makes them
    /// useless. Dropping them turns the next sign-in into a clean prompt rather
    /// than a confusing "sign-in expired".
    pub fn save_forgetting_sign_in(&self, path: &Path) -> Result<(), Error> {
        let mut out = self.clone();
        out.credentials.oauth = None;
        out.save(path)
    }

    pub fn save(&self, path: &Path) -> Result<(), Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| Error::Write {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let text = serde_json::to_string_pretty(self).expect("settings are always serialisable");
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
            last_channel: Some("forsen".into()),
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

    /// A file from an older build must not reset every other field.
    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"volume": 55}"#).unwrap();
        assert_eq!(settings.volume, 55);
        assert_eq!(settings.quality, QualityPreference::Auto);
        assert_eq!(settings.chat_width, 340.0);
        assert_eq!(settings.chat_history, 100);
        assert!(settings.channel_prefs.is_empty());
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

    #[test]
    fn corrupt_file_is_an_error_not_a_silent_reset() {
        let path = temp_file("corrupt");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not json").unwrap();
        assert!(matches!(Settings::load(&path), Err(Error::Parse { .. })));
        let _ = std::fs::remove_file(&path);
    }
}
