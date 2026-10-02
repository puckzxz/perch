//! A chat room's modes, from `ROOMSTATE`: who may talk, and how often.
//!
//! Twitch sends `ROOMSTATE` once on joining with every mode in it, and again
//! whenever a moderator changes one, carrying only what changed. So a line is
//! read as a [`ModeUpdate`] of the tags it actually has, and merged over what
//! the room was ([`RoomModes::merged`]); [`RoomModes::changes`] says what a
//! merge changed, for the pane to say so. Words for any of it are the chat
//! view's (`perch::chat_words`).
//!
//! Kept here, beside the parser, because which tags mean what is IRC's
//! business. Nothing here knows about a socket.

use crate::message::IrcMessage;

/// Every mode a room can be in, as last heard. `Default` is a room with
/// none on, which is what a room is before anything has been said about it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RoomModes {
    /// Only emotes may be sent (`emote-only`).
    pub emote_only: bool,
    /// Only followers may talk, and only once they have followed for this
    /// many minutes: `Some(0)` is any follower, `None` is anyone at all
    /// (`followers-only`, where Twitch writes off as `-1`).
    pub followers_only: Option<u32>,
    /// Every message has to differ from the speaker's last (`r9k`, which
    /// Twitch's own chat calls unique chat).
    pub unique: bool,
    /// Seconds a speaker waits between messages; zero is off (`slow`).
    pub slow: u32,
    /// Only subscribers, and the channel's moderators, may talk
    /// (`subs-only`).
    pub subs_only: bool,
}

/// The modes one `ROOMSTATE` line speaks of: `None` for a mode it does not
/// mention, which keeps whatever it was.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModeUpdate {
    pub emote_only: Option<bool>,
    pub followers_only: Option<Option<u32>>,
    pub unique: Option<bool>,
    pub slow: Option<u32>,
    pub subs_only: Option<bool>,
}

/// One mode that a [`ModeUpdate`] changed, and what it is now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeChange {
    EmoteOnly(bool),
    FollowersOnly(Option<u32>),
    Unique(bool),
    Slow(u32),
    SubsOnly(bool),
}

impl ModeUpdate {
    /// What a `ROOMSTATE` line says, read from its tags. A tag whose value is
    /// not a number Twitch would send is read as not being there, rather
    /// than as a mode switched off.
    pub fn from_irc(message: &IrcMessage) -> Self {
        let flag = |key: &str| match message.tag(key) {
            Some("1") => Some(true),
            Some("0") => Some(false),
            _ => None,
        };
        let followers_only = message
            .tag("followers-only")
            .and_then(|raw| raw.parse::<i64>().ok())
            .map(|minutes| u32::try_from(minutes).ok());
        Self {
            emote_only: flag("emote-only"),
            followers_only,
            unique: flag("r9k"),
            slow: message.tag("slow").and_then(|raw| raw.parse().ok()),
            subs_only: flag("subs-only"),
        }
    }

    /// Whether this speaks of no mode at all: a recording's room, which only
    /// ever announces its id, or a line with none of the tags.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Whether this speaks of every mode: the whole line Twitch sends on
    /// joining, and again on every rejoin after a dropped connection, as
    /// against the one-mode line a moderator's change sends. The chat
    /// reloads its third-party emotes on one of these, which is what retries
    /// a provider that failed the last time.
    pub fn is_full(&self) -> bool {
        self.emote_only.is_some()
            && self.followers_only.is_some()
            && self.unique.is_some()
            && self.slow.is_some()
            && self.subs_only.is_some()
    }
}

impl RoomModes {
    /// These modes with `update` laid over them: what it mentions, as it
    /// says; everything else, as it was.
    pub fn merged(self, update: ModeUpdate) -> Self {
        Self {
            emote_only: update.emote_only.unwrap_or(self.emote_only),
            followers_only: update.followers_only.unwrap_or(self.followers_only),
            unique: update.unique.unwrap_or(self.unique),
            slow: update.slow.unwrap_or(self.slow),
            subs_only: update.subs_only.unwrap_or(self.subs_only),
        }
    }

    /// What differs between `self` and `after`, one change per mode, in the
    /// order the chat's line of modes reads.
    pub fn changes(&self, after: &Self) -> Vec<ModeChange> {
        let mut changes = Vec::new();
        if self.slow != after.slow {
            changes.push(ModeChange::Slow(after.slow));
        }
        if self.followers_only != after.followers_only {
            changes.push(ModeChange::FollowersOnly(after.followers_only));
        }
        if self.subs_only != after.subs_only {
            changes.push(ModeChange::SubsOnly(after.subs_only));
        }
        if self.emote_only != after.emote_only {
            changes.push(ModeChange::EmoteOnly(after.emote_only));
        }
        if self.unique != after.unique {
            changes.push(ModeChange::Unique(after.unique));
        }
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::parse_line;

    fn update(line: &str) -> ModeUpdate {
        ModeUpdate::from_irc(&parse_line(line).unwrap())
    }

    /// The line sent on joining, verbatim from Twitch's IRC guide: every
    /// mode, all off.
    const JOINED: &str =
        "@emote-only=0;followers-only=-1;r9k=0;room-id=12345678;slow=0;subs-only=0 :tmi.twitch.tv ROOMSTATE #bar";

    #[test]
    fn the_join_line_names_every_mode() {
        let full = update(JOINED);
        assert_eq!(full.followers_only, Some(None));
        assert_eq!(full.slow, Some(0));
        assert_eq!(RoomModes::default().merged(full), RoomModes::default());
    }

    /// A later line carries only what changed, and the rest stays as it was.
    #[test]
    fn a_partial_update_merges_over_the_full_one() {
        let joined = RoomModes::default().merged(update(
            "@emote-only=0;followers-only=10;r9k=1;room-id=1;slow=0;subs-only=0 :tmi.twitch.tv ROOMSTATE #bar",
        ));
        assert_eq!(joined.followers_only, Some(10));
        assert!(joined.unique);

        let slowed = joined.merged(update("@room-id=1;slow=30 :tmi.twitch.tv ROOMSTATE #bar"));
        assert_eq!(slowed, RoomModes { slow: 30, ..joined });

        let anyone = slowed.merged(update(
            "@followers-only=-1;room-id=1 :tmi.twitch.tv ROOMSTATE #bar",
        ));
        assert_eq!(anyone.followers_only, None);
        assert_eq!(anyone.slow, 30);

        // Any follower at all is zero minutes, not off.
        let followers = anyone.merged(update(
            "@followers-only=0;room-id=1 :tmi.twitch.tv ROOMSTATE #bar",
        ));
        assert_eq!(followers.followers_only, Some(0));
    }

    /// A line with nothing but the room's id changes nothing, and says so.
    #[test]
    fn an_id_alone_is_no_update() {
        let bare = update("@room-id=1 :tmi.twitch.tv ROOMSTATE #bar");
        assert!(bare.is_empty());
        assert!(!update(JOINED).is_empty());
        // Nonsense is read as absence, not as off.
        assert!(update("@slow=fast;subs-only=yes :tmi.twitch.tv ROOMSTATE #bar").is_empty());
    }

    /// Only a line naming all five modes is a whole one, as joining sends;
    /// a moderator's change, or a recording's bare id, is not.
    #[test]
    fn only_the_join_line_is_full() {
        assert!(update(JOINED).is_full());
        assert!(!update("@room-id=1;slow=30 :tmi.twitch.tv ROOMSTATE #bar").is_full());
        assert!(!update("@room-id=1 :tmi.twitch.tv ROOMSTATE #bar").is_full());
        assert!(!update(
            "@emote-only=0;followers-only=-1;r9k=0;room-id=1;slow=0 :tmi.twitch.tv ROOMSTATE #bar"
        )
        .is_full());
    }

    #[test]
    fn changes_are_each_mode_that_moved_in_reading_order() {
        let before = RoomModes {
            slow: 30,
            ..RoomModes::default()
        };
        let after = RoomModes {
            slow: 0,
            subs_only: true,
            emote_only: true,
            ..RoomModes::default()
        };
        assert_eq!(
            before.changes(&after),
            [
                ModeChange::Slow(0),
                ModeChange::SubsOnly(true),
                ModeChange::EmoteOnly(true),
            ]
        );
        assert!(after.changes(&after).is_empty());
    }
}
