//! Persisted user settings.
//!
//! Stored as JSON next to the user's other roaming app data. Unknown fields are
//! ignored and missing fields fall back to defaults, so a settings file written
//! by an older or newer build still loads rather than resetting everything.
//!
//! Credentials live here too. They are stored in plain text, which is the same
//! thing every desktop Twitch client does, but it is a deliberate choice rather
//! than an oversight — see [`Credentials`].

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub quality: QualityPreference,
    /// 0-100. mpv's own default is 100, which is startling for a window that
    /// starts playing as soon as it opens.
    pub volume: u8,
    pub credentials: Credentials,
    /// Reopened on launch when no channel is given on the command line.
    pub last_channel: Option<String>,
    /// Width of the chat pane in pixels.
    pub chat_width: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            quality: QualityPreference::Auto,
            volume: 10,
            credentials: Credentials::default(),
            last_channel: None,
            chat_width: 340.0,
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

/// Where settings live: roaming app data, unlike the image cache which is
/// local-only because it is reproducible.
pub fn default_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("nativetwitch").join("settings.json")
}

impl Settings {
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
        serde_json::from_str(&text).map_err(|source| Error::Parse {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Write to `path`, creating parent directories as needed.
    ///
    /// Writes to a temporary file and renames, so an interrupted save cannot
    /// leave truncated settings — which for a file holding credentials would
    /// mean silently signing the user out.
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

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join("nativetwitch-tests")
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

    /// A file from an older build must not reset every other field.
    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"volume": 55}"#).unwrap();
        assert_eq!(settings.volume, 55);
        assert_eq!(settings.quality, QualityPreference::Auto);
        assert_eq!(settings.chat_width, 340.0);
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
