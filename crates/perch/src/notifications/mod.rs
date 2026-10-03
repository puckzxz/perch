//! A desktop notification when a followed channel goes live, for while
//! perch is not the window in use: minimised, behind other windows, or left
//! on one monitor while you work in another app.
//!
//! The follows poll already says who went live, in a toast inside the window
//! (`root::follows`). That toast is only seen by somebody looking at perch,
//! and the point of being told somebody went live is usually that you were
//! not. So when the window is not the one in front, the same news goes to
//! the operating system's notifications as well — on Windows only; elsewhere
//! [`show`] and [`forget`] do nothing.
//!
//! Who is told is the settings sheet's Desktop notifications
//! (`settings::DesktopNotifications`): nobody, the channels pinned to the top
//! of the rail, or everyone followed. And only about a broadcast perch has
//! not seen before ([`Seen`]): never the first poll's list, which is
//! everyone already live when perch started, and never a broadcast that
//! dropped out of one poll and came back in the next under the same id.
//! [`who`] decides and [`batch`] words it, pure and tested; the platform half
//! only shows what it is handed.
//!
//! Clicked, a Windows notification opens perch's own link to the channel
//! (`target::app_link`), which Windows hands to perch as a launch — and a
//! launch while perch runs is handed to the running window (`instance`),
//! which comes forward and opens the channel beside what is playing. See
//! the Windows half for why it is a link rather than a click perch hears
//! itself, and for what it writes to the registry to make either work.

#[cfg(windows)]
mod windows;

use std::collections::{HashMap, HashSet};

use settings::DesktopNotifications;
use twitch_api::LiveStream;

/// The broadcast each followed channel was last seen live with, so that a
/// channel newly in the live list is told apart from one whose broadcast
/// blinked out of a poll and came back: the same broadcast is no news.
#[derive(Default)]
pub struct Seen(HashMap<String, String>);

impl Seen {
    /// Note `stream`'s broadcast as seen, and say whether it is one not
    /// seen before for its channel.
    fn note(&mut self, stream: &LiveStream) -> bool {
        let broadcast = broadcast(stream);
        self.0.insert(stream.user_login.clone(), broadcast.clone()) != Some(broadcast)
    }
}

/// What tells one of a channel's broadcasts from another: Helix's id for
/// it, or when it began where Helix left the id out.
fn broadcast(stream: &LiveStream) -> String {
    if stream.id.is_empty() {
        stream.started_at.clone()
    } else {
        stream.id.clone()
    }
}

/// Who among a poll's `streams` gets a desktop notification, most watched
/// first, noting every broadcast in the poll as seen whoever is told.
///
/// A `first_poll` — the first since the worker started — seeds what is
/// known in silence, the rule the in-app toast keeps too: on launch everyone
/// is "newly" live. The caller says so rather than it being read off an
/// empty `was_live`, because a poll after one where nobody was live is no
/// first: the first stream of the morning, after a night with everyone
/// offline, is the very one worth telling. With the window `focused`, the
/// in-app toast has already said it to somebody looking. Otherwise a channel
/// not in `was_live`, who the poll before listed, is told about if `asked`
/// covers it — every channel, or those `pinned` says are — and its broadcast
/// is one not seen before.
pub fn who<'a>(
    streams: &'a [LiveStream],
    was_live: &HashSet<String>,
    first_poll: bool,
    asked: DesktopNotifications,
    pinned: impl Fn(&str) -> bool,
    focused: bool,
    seen: &mut Seen,
) -> Vec<&'a LiveStream> {
    let mut told: Vec<&LiveStream> = streams
        .iter()
        .filter(|stream| {
            // Noted first, for everyone: a broadcast seen while nobody was
            // told is still one seen.
            let new_broadcast = seen.note(stream);
            let covered = match asked {
                DesktopNotifications::Off => false,
                DesktopNotifications::Pinned => pinned(&stream.user_login),
                DesktopNotifications::All => true,
            };
            new_broadcast
                && covered
                && !first_poll
                && !focused
                && !was_live.contains(&stream.user_login)
        })
        .collect();
    told.sort_by_key(|stream| std::cmp::Reverse(stream.viewer_count));
    told
}

/// The first line of the news that `stream` went live: the heading of the
/// desktop notification, and the start of the in-app toast.
pub fn went_live(stream: &LiveStream) -> String {
    format!("{} went live", name(stream))
}

/// The channel's name as Twitch writes it, or its login where that is blank.
fn name(stream: &LiveStream) -> &str {
    if stream.display_name.trim().is_empty() {
        &stream.user_login
    } else {
        &stream.display_name
    }
}

/// The most notifications one poll shows. After a sleep, or an outage that
/// kept the polls failing, the first good poll can find a dozen channels
/// started in the meantime, and a dozen toasts each with its sound is a
/// burst Windows only queues. So past this many, the last says the rest.
const BATCH: usize = 3;

/// The notifications for one poll's `told`, most watched first: one each, up
/// to [`BATCH`]; past that, one each for all but the last of them, and a
/// last that names the rest — `<name> and <n> others went live`, the others'
/// names beneath — and opens the first of the rest, whom it names first.
/// Never a summary of one: that one is shown as itself.
pub fn batch(told: &[&LiveStream]) -> Vec<Notice> {
    if told.len() <= BATCH {
        return told.iter().map(|stream| Notice::of(stream)).collect();
    }
    let (each, rest) = told.split_at(BATCH - 1);
    let (first, others) = rest.split_first().expect("more than BATCH were told");
    let more = match others.len() {
        1 => "1 other".to_string(),
        n => format!("{n} others"),
    };
    let summary = Notice {
        login: first.user_login.clone(),
        heading: format!("{} and {more} went live", name(first)),
        lines: vec![others
            .iter()
            .map(|stream| name(stream))
            .collect::<Vec<_>>()
            .join(", ")],
    };
    each.iter()
        .map(|stream| Notice::of(stream))
        .chain(std::iter::once(summary))
        .collect()
}

/// One desktop notification: whose channel it opens, and what it says.
///
/// Made on every platform, from the same poll, and read only by the Windows
/// half: elsewhere [`show`] drops it unread, and the macOS leg's clippy, with
/// `-D warnings`, would otherwise call its fields dead.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct Notice {
    /// The channel it opens when clicked, and the tag that has a later
    /// notice of the same channel replace this one rather than pile up.
    pub login: String,
    /// `<name> went live`.
    pub heading: String,
    /// What is on, then what it is called — the category and the title —
    /// each only when the stream has one.
    pub lines: Vec<String>,
}

impl Notice {
    pub fn of(stream: &LiveStream) -> Self {
        Self {
            login: stream.user_login.clone(),
            heading: went_live(stream),
            lines: [&stream.game_name, &stream.title]
                .into_iter()
                .map(|line| line.trim())
                .filter(|line| !line.is_empty())
                .map(String::from)
                .collect(),
        }
    }
}

/// Show `notices`, off the UI thread: the first of a session registers perch
/// with Windows first, and every one is a round trip to the notification
/// platform. Failures go to the log, since the in-app toast has said it
/// anyway. Nothing, off Windows.
pub fn show(notices: Vec<Notice>) {
    if notices.is_empty() {
        return;
    }
    #[cfg(windows)]
    windows::show(notices);
}

/// Take back what showing notifications registered with Windows, now that
/// Desktop notifications is Off. Nothing, off Windows.
pub fn forget() {
    #[cfg(windows)]
    windows::forget();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(login: &str, broadcast: &str, viewers: u64) -> LiveStream {
        LiveStream {
            id: broadcast.into(),
            user_login: login.into(),
            user_id: String::new(),
            display_name: login.to_uppercase(),
            title: String::new(),
            game_name: String::new(),
            viewer_count: viewers,
            thumbnail_url: String::new(),
            started_at: String::new(),
        }
    }

    fn live(logins: &[&str]) -> HashSet<String> {
        logins.iter().map(|login| login.to_string()).collect()
    }

    fn logins(told: &[&LiveStream]) -> Vec<String> {
        told.iter()
            .map(|stream| stream.user_login.clone())
            .collect()
    }

    /// Asked for everyone, in the background, past the first poll: whoever
    /// is new to the live list, most watched first, and nobody already on.
    #[test]
    fn everyone_new_is_told_in_the_background() {
        let mut seen = Seen::default();
        let streams = [
            stream("a", "1", 10),
            stream("b", "2", 500),
            stream("old", "3", 9),
        ];
        let told = who(
            &streams,
            &live(&["old"]),
            false,
            DesktopNotifications::All,
            |_| false,
            false,
            &mut seen,
        );
        assert_eq!(logins(&told), ["b", "a"]);
    }

    /// Pinned only tells about the pinned, and Off about nobody.
    #[test]
    fn the_setting_says_who_is_covered() {
        let streams = [stream("pinned", "1", 1), stream("other", "2", 1)];
        let was = live(&["someone"]);
        let pinned = |login: &str| login == "pinned";

        let told = who(
            &streams,
            &was,
            false,
            DesktopNotifications::Pinned,
            pinned,
            false,
            &mut Seen::default(),
        );
        assert_eq!(logins(&told), ["pinned"]);

        let told = who(
            &streams,
            &was,
            false,
            DesktopNotifications::Off,
            pinned,
            false,
            &mut Seen::default(),
        );
        assert!(told.is_empty());
    }

    /// With perch in front the in-app toast says it, and the first poll is
    /// everyone already live at launch: neither tells anybody.
    #[test]
    fn nobody_is_told_while_focused_or_at_the_first_poll() {
        let streams = [stream("a", "1", 1)];
        let focused = who(
            &streams,
            &live(&["someone"]),
            false,
            DesktopNotifications::All,
            |_| true,
            true,
            &mut Seen::default(),
        );
        assert!(focused.is_empty(), "told while focused");

        let seed = who(
            &streams,
            &HashSet::new(),
            true,
            DesktopNotifications::All,
            |_| true,
            false,
            &mut Seen::default(),
        );
        assert!(seed.is_empty(), "told at the first poll");
    }

    /// A poll after one where nobody was live is no first poll: the first
    /// stream of the morning, after a night with everyone offline, is told.
    #[test]
    fn the_first_stream_after_nobody_was_live_is_told() {
        let streams = [stream("a", "1", 1)];
        let told = who(
            &streams,
            &HashSet::new(),
            false,
            DesktopNotifications::All,
            |_| false,
            false,
            &mut Seen::default(),
        );
        assert_eq!(logins(&told), ["a"]);
    }

    /// One notification a broadcast: a stream that drops out of a poll and
    /// comes back under the same id is not news again, seen while told or
    /// while nobody was (focused, or at the first poll); a new broadcast is.
    #[test]
    fn a_broadcast_is_told_about_once() {
        let mut seen = Seen::default();
        let all = DesktopNotifications::All;
        let back = [stream("a", "1", 1)];
        let gone = live(&["someone"]);

        assert_eq!(
            logins(&who(&back, &gone, false, all, |_| true, false, &mut seen)),
            ["a"]
        );
        assert!(
            who(&back, &gone, false, all, |_| true, false, &mut seen).is_empty(),
            "told twice about one broadcast"
        );

        let restarted = [stream("a", "2", 1)];
        assert_eq!(
            logins(&who(
                &restarted,
                &gone,
                false,
                all,
                |_| true,
                false,
                &mut seen
            )),
            ["a"]
        );

        // Seen at the first poll, then back after a blink, unfocused.
        let mut seen = Seen::default();
        let started = [stream("b", "7", 1)];
        assert!(who(
            &started,
            &HashSet::new(),
            true,
            all,
            |_| true,
            false,
            &mut seen
        )
        .is_empty());
        assert!(
            who(&started, &gone, false, all, |_| true, false, &mut seen).is_empty(),
            "a broadcast already on at launch was news"
        );

        // Seen while focused, then back after a blink in the background.
        let mut seen = Seen::default();
        assert!(who(&started, &gone, false, all, |_| true, true, &mut seen).is_empty());
        assert!(who(&started, &gone, false, all, |_| true, false, &mut seen).is_empty());
    }

    /// Without Helix's id for a broadcast, when it began tells them apart.
    #[test]
    fn a_broadcast_without_an_id_goes_by_when_it_began() {
        let mut seen = Seen::default();
        let mut first = stream("a", "", 1);
        first.started_at = "2026-10-03T18:00:00Z".into();
        let gone = live(&["someone"]);
        let all = DesktopNotifications::All;
        assert_eq!(
            logins(&who(
                &[first.clone()],
                &gone,
                false,
                all,
                |_| true,
                false,
                &mut seen
            )),
            ["a"]
        );
        assert!(who(
            &[first.clone()],
            &gone,
            false,
            all,
            |_| true,
            false,
            &mut seen
        )
        .is_empty());
        first.started_at = "2026-10-03T21:00:00Z".into();
        assert_eq!(
            logins(&who(
                &[first],
                &gone,
                false,
                all,
                |_| true,
                false,
                &mut seen
            )),
            ["a"]
        );
    }

    /// Up to three a poll are shown each as itself; past three, the first
    /// two are, and a third names the rest and opens the first of them.
    #[test]
    fn a_burst_of_go_lives_is_cut_to_three() {
        let streams: Vec<LiveStream> = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|login| stream(login, login, 1))
            .collect();
        let told: Vec<&LiveStream> = streams.iter().collect();

        let each: Vec<Notice> = told[..3].iter().map(|s| Notice::of(s)).collect();
        assert_eq!(batch(&told[..3]), each);

        let five = batch(&told);
        assert_eq!(five.len(), 3);
        assert_eq!(five[..2], [Notice::of(told[0]), Notice::of(told[1])]);
        assert_eq!(
            five[2],
            Notice {
                login: "c".into(),
                heading: "C and 2 others went live".into(),
                lines: vec!["D, E".into()],
            }
        );

        let four = batch(&told[..4]);
        assert_eq!(four[2].heading, "C and 1 other went live");
        assert_eq!(four[2].lines, ["D"]);
    }

    /// The heading names the channel as Twitch writes it; the lines are what
    /// is on and what it is called, only those the stream has.
    #[test]
    fn a_notice_says_who_went_live_and_with_what() {
        let mut full = stream("xqc", "1", 1);
        full.display_name = "xQc".into();
        full.game_name = "Just Chatting".into();
        full.title = "  react andy  ".into();
        assert_eq!(
            Notice::of(&full),
            Notice {
                login: "xqc".into(),
                heading: "xQc went live".into(),
                lines: vec!["Just Chatting".into(), "react andy".into()],
            }
        );

        let mut bare = stream("forsen", "2", 1);
        bare.display_name = " ".into();
        bare.game_name = "   ".into();
        let notice = Notice::of(&bare);
        assert_eq!(notice.heading, "forsen went live", "a blank display name");
        assert!(notice.lines.is_empty(), "{:?}", notice.lines);
    }
}
