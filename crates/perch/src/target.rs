//! What somebody typed or pasted, read as a place on Twitch.
//!
//! A channel arrives as a login, a `#login`, a `twitch.tv/login` or the whole
//! `https://www.twitch.tv/login`; a recording as its `/videos/<id>` link,
//! sometimes with a `?t=1h2m3s` in it saying where to start. The command
//! line and the palette both take these, and they agree on what a thing
//! means because they both ask here — and here asks the one definition of a
//! login, `twitch_chat::is_login`, rather than keeping a second one.
//!
//! The other way round lives here too: [`link`] is the one place Perch writes
//! a twitch.tv URL, and it is the inverse of [`parse`], so a link Perch hands
//! out — a recording's, at the moment you were at — opens the same thing at
//! the same moment when it is pasted back.

use settings::channel_key;
use twitch_api::{format_duration, parse_duration};
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

/// The twitch.tv page for `target`: what [`parse`] reads back as `target`.
///
/// The `www` host, which is what the site itself links to; `parse` takes it
/// with or without. A recording carries `?t=` only from its first second on:
/// a start at zero is the start, and writing it would hand out a link that
/// says less than it seems to.
pub fn link(target: &Target) -> String {
    match target {
        Target::Channel(login) => format!("https://www.twitch.tv/{login}"),
        Target::Video { id, start_secs } => match start_secs {
            Some(secs) if *secs > 0 => {
                format!(
                    "https://www.twitch.tv/videos/{id}?t={}",
                    format_duration(*secs)
                )
            }
            _ => format!("https://www.twitch.tv/videos/{id}"),
        },
    }
}

/// The scheme of perch's own links, which a desktop notification opens
/// (`notifications`). Windows hands a link in it to the program registered
/// for it, which is perch, as the one argument of a launch; see
/// [`app_link`].
pub const APP_SCHEME: &str = "perch";

/// perch's own link to watch `login`: `perch://watch/<login>`, what a
/// go-live notification opens when it is clicked. A link of perch's rather
/// than twitch.tv's, because Windows opens a twitch.tv link in the browser;
/// this one comes back to perch as a launch, which `instance` hands to the
/// running window like any other. [`parse_app_link`] reads it back.
/// Written only by the Windows notifications, so dead code to the macOS
/// leg's clippy, which runs with `-D warnings`.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn app_link(login: &str) -> String {
    format!("{APP_SCHEME}://watch/{}", channel_key(login))
}

/// Whether `arg` is one of perch's own links, of any kind: the scheme, in
/// any case, and a colon. The launch it comes in is the link and nothing
/// else (see `launch`), however well or badly the rest reads.
pub fn is_app_link(arg: &str) -> bool {
    arg.len() > APP_SCHEME.len()
        && arg.is_char_boundary(APP_SCHEME.len())
        && arg[..APP_SCHEME.len()].eq_ignore_ascii_case(APP_SCHEME)
        && arg[APP_SCHEME.len()..].starts_with(':')
}

/// The channel one of perch's own links names ([`app_link`]), or nothing for
/// a link that names none: anything but `watch/` and a login.
///
/// Read strictly, since anything on the machine — a web page, through the
/// browser's prompt — can open a link in the scheme once it is registered:
/// all a link can do is open a channel, which a twitch.tv link already can.
/// The slashes after the colon, and one after the login, are taken or left,
/// because what hands the link over may write them either way.
pub fn parse_app_link(text: &str) -> Option<Target> {
    let text = text.trim();
    if !is_app_link(text) {
        return None;
    }
    let rest = &text[APP_SCHEME.len() + 1..];
    let rest = rest.trim_start_matches('/');
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    let (verb, login) = rest.split_once('/')?;
    if !verb.eq_ignore_ascii_case("watch") || login.starts_with('#') {
        return None;
    }
    channel(login)
}

/// The moment a link to a recording `position` seconds in starts at: the
/// whole second it is in, or none in the first second, which [`link`] writes
/// as no time anyway. Read by the link a pane hands out and by the words that
/// offer it (`Copy link at 1:02:03`), so the two cannot disagree about when
/// there is a time to speak of.
pub fn moment(position: f64) -> Option<u64> {
    // `as` floors, and saturates anything below zero (and NaN) to zero.
    let secs = position as u64;
    (secs > 0).then_some(secs)
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

    /// Every link Perch writes opens what it names, at the moment it names.
    #[test]
    fn a_link_reads_back_as_what_it_names() {
        let targets = [
            Target::Channel("forsen".into()),
            Target::Video {
                id: "2868644730".into(),
                start_secs: None,
            },
            Target::Video {
                id: "2868644730".into(),
                start_secs: Some(45),
            },
            Target::Video {
                id: "2868644730".into(),
                start_secs: Some(3723),
            },
        ];
        for target in targets {
            assert_eq!(parse(&link(&target)), Some(target.clone()), "{target:?}");
        }
    }

    /// A start at zero is the start, so it is written as no time at all.
    #[test]
    fn a_link_at_the_start_carries_no_time() {
        let at_zero = Target::Video {
            id: "5".into(),
            start_secs: Some(0),
        };
        let written = link(&at_zero);
        assert!(!written.contains("t="), "{written}");
        assert_eq!(parse(&written), video("5", None));
    }

    /// A moment is a whole second, counted from the first one: the time a
    /// player reports a fraction into a second is the second it is in, and
    /// anything short of the first is the start.
    #[test]
    fn a_moment_starts_at_the_first_whole_second() {
        assert_eq!(moment(0.0), None);
        assert_eq!(moment(0.99), None);
        assert_eq!(moment(-3.0), None, "a position before the start");
        assert_eq!(moment(1.0), Some(1));
        assert_eq!(moment(3723.7), Some(3723));
    }

    /// perch's own link to a channel reads back as the channel, however
    /// whatever handed it over wrote the slashes and the case.
    #[test]
    fn an_app_link_reads_back_as_the_channel_it_names() {
        assert_eq!(app_link("#Forsen"), "perch://watch/forsen");
        assert_eq!(parse_app_link(&app_link("xqc")), channel("xqc"));
        assert_eq!(parse_app_link("perch:watch/xqc"), channel("xqc"));
        assert_eq!(parse_app_link("PERCH://Watch/XQC/"), channel("xqc"));
        assert!(is_app_link("Perch:anything"));
        assert!(!is_app_link("perch"));
        assert!(!is_app_link("perchance"));
        assert!(!is_app_link("forsen"));
    }

    /// A link in perch's scheme that names anything but a channel to watch
    /// opens nothing.
    #[test]
    fn an_app_link_to_anything_else_is_nothing() {
        for text in [
            "perch:",
            "perch://",
            "perch://watch",
            "perch://watch/",
            "perch://watch/two words",
            "perch://watch/#forsen",
            "perch://watch/forsen/extra",
            "perch://open/forsen",
            "perch://watch/forsen?x=1",
            "https://www.twitch.tv/forsen",
        ] {
            assert_eq!(parse_app_link(text), None, "{text}");
        }
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
