//! What a chat says about itself in words: a timeout or a ban, the line
//! above a reply, and the room's modes, both as the quiet line at the foot
//! of the pane and as a notice when one changes. Pure and tested; the chat
//! view (`chat/`) puts the words where they go.
//!
//! The modes are `twitch_chat::RoomModes`, merged there from `ROOMSTATE`.
//! Sentence case, as the chat's own notices are, and lengths in whole words
//! (`10 minutes`) where they are read in a sentence and in short units
//! (`10m`) on the line of modes, which has to stay one line.

use twitch_chat::{ModeChange, Reply, RoomModes};

/// Units a length is said in, largest first.
const UNITS: [(u64, &str, &str); 4] = [
    (86_400, "day", "d"),
    (3_600, "hour", "h"),
    (60, "minute", "m"),
    (1, "second", "s"),
];

/// The two largest units `secs` has any of, largest first, as
/// `(count, unit)`: `5400` is an hour and thirty minutes. Two at most, so a
/// length reads at a glance (`1 day 3 hours`, not down to the second);
/// Twitch's own presets are whole units anyway.
fn parts(secs: u64) -> Vec<(u64, usize)> {
    let mut left = secs;
    let mut parts = Vec::new();
    for (index, (size, _, _)) in UNITS.iter().enumerate() {
        if left >= *size && parts.len() < 2 {
            parts.push((left / size, index));
            left %= size;
        } else if !parts.is_empty() {
            // Only adjacent units: `1 day 0 hours 5 minutes` stops at the day.
            break;
        }
    }
    parts
}

/// A length in words: `30 seconds`, `10 minutes`, `1 hour 30 minutes`,
/// `14 days`. Zero is `0 seconds`.
pub fn span(secs: u64) -> String {
    let parts = parts(secs);
    if parts.is_empty() {
        return "0 seconds".to_string();
    }
    parts
        .iter()
        .map(|(count, unit)| {
            let word = UNITS[*unit].1;
            if *count == 1 {
                format!("1 {word}")
            } else {
                format!("{count} {word}s")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A length in short units, for the line of modes: `30s`, `10m`, `1h 30m`.
pub fn short_span(secs: u64) -> String {
    let parts = parts(secs);
    if parts.is_empty() {
        return "0s".to_string();
    }
    parts
        .iter()
        .map(|(count, unit)| format!("{count}{}", UNITS[*unit].2))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The notice for one person cleared from the chat: `name was timed out for
/// 10 minutes` for a timeout of `seconds`, `name was banned` for a ban.
pub fn ban_notice(name: &str, seconds: Option<u64>) -> String {
    match seconds {
        Some(secs) => format!("{name} was timed out for {}", span(secs)),
        None => format!("{name} was banned"),
    }
}

/// The line above a reply: `Replying to @name: what they said`, with the
/// parent's text on one line (a line break in it would be a second line the
/// row has no room for). The chat view cuts it short where it does not fit.
/// A parent with no text to show (Twitch always sends some, but a tag can
/// be empty) is just `Replying to @name`.
pub fn reply_context(reply: &Reply) -> String {
    let body = reply.body.split_whitespace().collect::<Vec<_>>().join(" ");
    if body.is_empty() {
        format!("Replying to @{}", reply.display_name)
    } else {
        format!("Replying to @{}: {body}", reply.display_name)
    }
}

/// The room's modes as one quiet line for the foot of the chat, `Slow mode
/// 30s · Sub-only`, or `None` when none is on and there is nothing to say.
/// In the order `RoomModes::changes` reads them, the ones that hold a
/// speaker back longest first.
pub fn modes_line(modes: &RoomModes) -> Option<String> {
    let mut parts = Vec::new();
    if modes.slow > 0 {
        parts.push(format!("Slow mode {}", short_span(modes.slow.into())));
    }
    match modes.followers_only {
        Some(0) => parts.push("Followers-only".to_string()),
        Some(minutes) => parts.push(format!(
            "Followers-only {}",
            short_span(u64::from(minutes) * 60)
        )),
        None => {}
    }
    if modes.subs_only {
        parts.push("Sub-only".to_string());
    }
    if modes.emote_only {
        parts.push("Emote-only".to_string());
    }
    if modes.unique {
        parts.push("Unique chat".to_string());
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// The notice for one mode a moderator changed after the pane joined.
pub fn mode_notice(change: ModeChange) -> String {
    let switched = |name: &str, on: bool| format!("{name} is {}", if on { "on" } else { "off" });
    match change {
        ModeChange::Slow(0) => switched("Slow mode", false),
        ModeChange::Slow(secs) => {
            format!("Slow mode is on: one message every {}", span(secs.into()))
        }
        ModeChange::FollowersOnly(None) => switched("Followers-only mode", false),
        ModeChange::FollowersOnly(Some(0)) => {
            "Followers-only mode is on: any follower can chat".to_string()
        }
        ModeChange::FollowersOnly(Some(minutes)) => format!(
            "Followers-only mode is on: followers of {} or more",
            span(u64::from(minutes) * 60)
        ),
        ModeChange::SubsOnly(on) => switched("Sub-only mode", on),
        ModeChange::EmoteOnly(on) => switched("Emote-only mode", on),
        ModeChange::Unique(on) => switched("Unique chat", on),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_length_reads_in_its_largest_units() {
        assert_eq!(span(1), "1 second");
        assert_eq!(span(30), "30 seconds");
        assert_eq!(span(60), "1 minute");
        assert_eq!(span(600), "10 minutes");
        assert_eq!(span(90), "1 minute 30 seconds");
        assert_eq!(span(5_400), "1 hour 30 minutes");
        assert_eq!(span(86_400), "1 day");
        // Twitch's longest timeout.
        assert_eq!(span(1_209_600), "14 days");
        // Two adjacent units at most.
        assert_eq!(span(86_400 + 300), "1 day");
        assert_eq!(span(3_661), "1 hour 1 minute");
        assert_eq!(span(0), "0 seconds");

        assert_eq!(short_span(30), "30s");
        assert_eq!(short_span(600), "10m");
        assert_eq!(short_span(5_400), "1h 30m");
        assert_eq!(short_span(7 * 86_400), "7d");
    }

    #[test]
    fn a_timeout_says_how_long_and_a_ban_does_not() {
        assert_eq!(
            ban_notice("Ronni", Some(600)),
            "Ronni was timed out for 10 minutes"
        );
        assert_eq!(
            ban_notice("ronni", Some(1)),
            "ronni was timed out for 1 second"
        );
        assert_eq!(ban_notice("Ronni", None), "Ronni was banned");
    }

    #[test]
    fn a_reply_names_its_parent_on_one_line() {
        let reply = |body: &str| Reply {
            login: "someone".into(),
            display_name: "SomeOne".into(),
            body: body.into(),
        };
        assert_eq!(
            reply_context(&reply("is this the boss?")),
            "Replying to @SomeOne: is this the boss?"
        );
        assert_eq!(
            reply_context(&reply(
                "two
lines  and	spaces "
            )),
            "Replying to @SomeOne: two lines and spaces"
        );
        assert_eq!(reply_context(&reply("")), "Replying to @SomeOne");
    }

    #[test]
    fn the_modes_line_names_what_is_on() {
        assert_eq!(modes_line(&RoomModes::default()), None);
        let modes = RoomModes {
            slow: 30,
            subs_only: true,
            ..RoomModes::default()
        };
        assert_eq!(
            modes_line(&modes).as_deref(),
            Some("Slow mode 30s · Sub-only")
        );
        let all = RoomModes {
            emote_only: true,
            followers_only: Some(10),
            unique: true,
            slow: 120,
            subs_only: true,
        };
        assert_eq!(
            modes_line(&all).as_deref(),
            Some("Slow mode 2m · Followers-only 10m · Sub-only · Emote-only · Unique chat")
        );
        let anyone_following = RoomModes {
            followers_only: Some(0),
            ..RoomModes::default()
        };
        assert_eq!(
            modes_line(&anyone_following).as_deref(),
            Some("Followers-only")
        );
    }

    #[test]
    fn a_mode_change_is_a_sentence() {
        assert_eq!(
            mode_notice(ModeChange::Slow(30)),
            "Slow mode is on: one message every 30 seconds"
        );
        assert_eq!(mode_notice(ModeChange::Slow(0)), "Slow mode is off");
        assert_eq!(
            mode_notice(ModeChange::FollowersOnly(Some(10))),
            "Followers-only mode is on: followers of 10 minutes or more"
        );
        assert_eq!(
            mode_notice(ModeChange::FollowersOnly(Some(0))),
            "Followers-only mode is on: any follower can chat"
        );
        assert_eq!(
            mode_notice(ModeChange::FollowersOnly(None)),
            "Followers-only mode is off"
        );
        assert_eq!(
            mode_notice(ModeChange::SubsOnly(true)),
            "Sub-only mode is on"
        );
        assert_eq!(
            mode_notice(ModeChange::EmoteOnly(false)),
            "Emote-only mode is off"
        );
        assert_eq!(mode_notice(ModeChange::Unique(true)), "Unique chat is on");
    }
}
