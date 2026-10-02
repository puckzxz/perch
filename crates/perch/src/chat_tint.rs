//! Which wash a chat row wears, if any. Pure and tested; the colours are
//! `theme`'s, and `ChatView::row_frame` lays the one chosen behind the row.
//!
//! A wash rather than anything that changes a row's geometry, for the reason
//! events are washed (HANDOFF, Chat): the row has to keep its place in the
//! rhythm you scan down. Events keep their two intensities, except that an
//! announcement wears the colour its sender picked, mapped onto a few washes
//! of the app's own rather than Twitch's saturated swatches; a message wears
//! one only when Twitch marks it: Highlight My Message, or someone's first
//! message in the channel.

use gpui::Hsla;
use twitch_chat::{ChatMessage, ChatNotice, NoticeKind};

use crate::theme;

/// Every wash a row can wear, named for why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wash {
    /// A sub, a gift, anything Twitch invents.
    Event,
    /// A raid, or an announcement in the channel's own colour.
    EventLoud,
    /// An announcement in one of the colours a sender can pick.
    AnnouncementBlue,
    AnnouncementGreen,
    AnnouncementOrange,
    AnnouncementPurple,
    /// Sent with Highlight My Message.
    Highlight,
    /// The speaker's first message in the channel.
    FirstMessage,
}

impl Wash {
    /// The colour, from the theme.
    pub fn color(self) -> Hsla {
        match self {
            Wash::Event => theme::event_wash(),
            Wash::EventLoud => theme::event_wash_loud(),
            Wash::AnnouncementBlue => theme::announcement_wash_blue(),
            Wash::AnnouncementGreen => theme::announcement_wash_green(),
            Wash::AnnouncementOrange => theme::announcement_wash_orange(),
            Wash::AnnouncementPurple => theme::announcement_wash_purple(),
            Wash::Highlight => theme::highlight_wash(),
            Wash::FirstMessage => theme::first_message_wash(),
        }
    }
}

/// The wash behind a message, if it has one. A highlighted message was paid
/// for to be seen and wins over a first one; Twitch would not send both on
/// one line in practice, but a row wears one wash and this says which.
pub fn message_wash(message: &ChatMessage) -> Option<Wash> {
    if message.highlighted {
        Some(Wash::Highlight)
    } else if message.first {
        Some(Wash::FirstMessage)
    } else {
        None
    }
}

/// The wash behind an event. An announcement in a colour its sender picked
/// wears that colour's wash; `PRIMARY`, a colour nobody recognises, or none,
/// keeps the loud one, as a raid does.
pub fn event_wash(notice: &ChatNotice) -> Wash {
    match notice.kind {
        NoticeKind::Announcement => match notice.announcement_color.as_deref() {
            Some("BLUE") => Wash::AnnouncementBlue,
            Some("GREEN") => Wash::AnnouncementGreen,
            Some("ORANGE") => Wash::AnnouncementOrange,
            Some("PURPLE") => Wash::AnnouncementPurple,
            _ => Wash::EventLoud,
        },
        NoticeKind::Raid => Wash::EventLoud,
        NoticeKind::Subscription | NoticeKind::Other => Wash::Event,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use twitch_chat::message::parse_line;

    fn message(tags: &str) -> ChatMessage {
        let line = format!("@{tags} :a!a@a PRIVMSG #a :hi");
        ChatMessage::from_irc(&parse_line(&line).unwrap()).unwrap()
    }

    fn notice(tags: &str) -> ChatNotice {
        let line = format!("@{tags};system-msg=x :tmi.twitch.tv USERNOTICE #a :hi");
        ChatNotice::from_irc(&parse_line(&line).unwrap()).unwrap()
    }

    #[test]
    fn a_message_is_washed_only_when_twitch_marks_it() {
        assert_eq!(message_wash(&message("color=#FF0000")), None);
        assert_eq!(
            message_wash(&message("first-msg=1")),
            Some(Wash::FirstMessage)
        );
        assert_eq!(
            message_wash(&message("msg-id=highlighted-message")),
            Some(Wash::Highlight)
        );
        assert_eq!(
            message_wash(&message("first-msg=1;msg-id=highlighted-message")),
            Some(Wash::Highlight)
        );
    }

    #[test]
    fn an_announcement_wears_its_colour() {
        let announce = |color: &str| {
            event_wash(&notice(&format!(
                "msg-id=announcement;login=m;msg-param-color={color}"
            )))
        };
        assert_eq!(announce("BLUE"), Wash::AnnouncementBlue);
        assert_eq!(announce("GREEN"), Wash::AnnouncementGreen);
        assert_eq!(announce("ORANGE"), Wash::AnnouncementOrange);
        assert_eq!(announce("PURPLE"), Wash::AnnouncementPurple);
        assert_eq!(announce("PRIMARY"), Wash::EventLoud);
        assert_eq!(announce("TEAL"), Wash::EventLoud);
        assert_eq!(
            event_wash(&notice("msg-id=announcement;login=m")),
            Wash::EventLoud
        );
    }

    #[test]
    fn other_events_keep_their_two_intensities() {
        assert_eq!(event_wash(&notice("msg-id=raid;login=r")), Wash::EventLoud);
        assert_eq!(event_wash(&notice("msg-id=resub;login=r")), Wash::Event);
        assert_eq!(
            event_wash(&notice("msg-id=somethingnew;login=r")),
            Wash::Event
        );
    }
}
