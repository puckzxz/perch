//! What somebody typed or pasted, read as a place on Twitch.
//!
//! A channel arrives as a login, a `#login`, a `twitch.tv/login` or the whole
//! `https://www.twitch.tv/login`; a recording as its `/videos/<id>` link,
//! sometimes with a `?t=1h2m3s` in it saying where to start. The command
//! line and the palette both take these, and they agree on what a thing
//! means because they both ask here — and here asks the one definition of a
//! login, `twitch_chat::is_login`, rather than keeping a second one.

use settings::channel_key;
use twitch_api::parse_duration;
use twitch_chat::is_login;

/// Somewhere on Twitch the app can open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A channel, by its login, already normalised.
    Channel(String),
    /// A recording, by its id, and how far in to start if the link said.
    Video { id: String, start_secs: Option<u64> },
}

/// The hosts a Twitch link comes with. The clip and embed hosts are not
/// here: a clip is not a recording, and the player has its own URLs.
const HOSTS: [&str; 3] = ["twitch.tv", "www.twitch.tv", "m.twitch.tv"];

/// The site's own pages that sit where a login would in a link. Not every
/// one of them — that list is Twitch's to grow — but the ones a link is
/// likely to carry.
const RESERVED: [&str; 8] = [
    "directory",
    "videos",
    "search",
    "settings",
    "subscriptions",
    "downloads",
    "drops",
    "turbo",
];

/// Read `text` as a channel or a recording, or nothing if it is neither.
pub fn parse(text: &str) -> Option<Target> {
    let text = text.trim();
    let rest = text
        .strip_prefix("https://")
        .or_else(|| text.strip_prefix("http://"))
        .unwrap_or(text);
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));

    if !HOSTS.contains(&host.to_ascii_lowercase().as_str()) {
        // Not a link: a bare name, with or without the IRC hash. Anything
        // with a path or a dot in it is a link to somewhere else, not a
        // channel called that.
        if text.contains('/') || text.contains('.') {
            return None;
        }
        return channel(text);
    }

    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    match (parts.next(), parts.next(), parts.next()) {
        // twitch.tv/videos/<id>, with or without a start time.
        (Some("videos"), Some(id), None) => video(id, query),
        // The older twitch.tv/<login>/video/<id> and /v/<id> forms.
        (Some(_), Some("video" | "v"), Some(id)) => video(id, query),
        // twitch.tv/<login>, and the channel's own /videos page.
        (Some(login), None, None) | (Some(login), Some("videos"), None) => {
            if RESERVED.contains(&login.to_ascii_lowercase().as_str()) {
                return None;
            }
            channel(login)
        }
        _ => None,
    }
}

fn channel(text: &str) -> Option<Target> {
    let login = channel_key(text);
    is_login(&login).then_some(Target::Channel(login))
}

fn video(id: &str, query: &str) -> Option<Target> {
    let id = id.strip_prefix('v').unwrap_or(id);
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(Target::Video {
        id: id.to_string(),
        start_secs: start_of(query),
    })
}

/// The `t=1h2m3s` a link to a moment carries, in seconds. The grammar is
/// Helix's own duration, so the same reader serves both.
fn start_of(query: &str) -> Option<u64> {
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix("t="))
        .and_then(parse_duration)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(login: &str) -> Option<Target> {
        Some(Target::Channel(login.into()))
    }

    fn video(id: &str, start_secs: Option<u64>) -> Option<Target> {
        Some(Target::Video {
            id: id.into(),
            start_secs,
        })
    }

    #[test]
    fn a_name_is_a_channel_however_it_is_written() {
        assert_eq!(parse("forsen"), channel("forsen"));
        assert_eq!(parse("  #Forsen "), channel("forsen"));
        assert_eq!(parse("twitch.tv/forsen"), channel("forsen"));
        assert_eq!(parse("https://www.twitch.tv/Forsen/"), channel("forsen"));
        assert_eq!(
            parse("http://m.twitch.tv/forsen?referrer=x"),
            channel("forsen")
        );
        assert_eq!(
            parse("https://www.twitch.tv/forsen/videos"),
            channel("forsen")
        );
    }

    #[test]
    fn a_recording_link_names_the_video_and_where_to_start() {
        assert_eq!(
            parse("https://www.twitch.tv/videos/2868644730"),
            video("2868644730", None)
        );
        assert_eq!(
            parse("twitch.tv/videos/2868644730?t=1h2m3s&x=1"),
            video("2868644730", Some(3723))
        );
        assert_eq!(
            parse("https://www.twitch.tv/forsen/video/2868644730?t=30s"),
            video("2868644730", Some(30))
        );
        assert_eq!(parse("twitch.tv/videos/v123"), video("123", None));
        // A start the link garbles is no start, not a refusal.
        assert_eq!(parse("twitch.tv/videos/5?t=soon"), video("5", None));
    }

    #[test]
    fn what_is_neither_is_nothing() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("has space"), None);
        assert_eq!(parse("lol.jpg"), None);
        assert_eq!(parse("https://youtube.com/watch?v=1"), None);
        assert_eq!(parse("twitch.tv"), None);
        assert_eq!(parse("twitch.tv/videos/notanumber"), None);
        assert_eq!(parse("twitch.tv/directory/game/chess"), None);
        assert_eq!(
            parse("https://www.twitch.tv/directory"),
            None,
            "the site's own pages are not channels"
        );
    }
}
