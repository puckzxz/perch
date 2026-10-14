//! Which wash a chat row wears, if any. Pure and tested; the colours are
//! `theme`'s, and `ChatView::row_frame` lays the one chosen behind the row.
//!
//! A wash rather than anything that changes a row's geometry, for the reason
//! events are washed (HANDOFF, Chat): the row has to keep its place in the
//! rhythm you scan down. Events keep their two intensities, except that an
//! announcement wears the colour its sender picked, mapped onto a few washes
//! of the app's own rather than Twitch's saturated swatches; a message wears
//! one when Twitch marks it — Highlight My Message, or someone's first
//! message in the channel — or when it is addressed to the signed-in user:
//! it names them, or answers one of their messages.

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
    /// Names the signed-in user, or replies to them.
    Mention,
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
            Wash::Mention => theme::mention_wash(),
            Wash::Highlight => theme::highlight_wash(),
            Wash::FirstMessage => theme::first_message_wash(),
        }
    }
}

/// The wash behind a message, if it has one, for a chat read by `me`, the
/// signed-in login, if anyone is signed in. A row wears one wash, and this
/// says which: a line addressed to the reader first, since it is the one
/// that wants an answer; then a highlighted one, paid for to be seen; then
/// a first message. Twitch would not send the last two on one line in
/// practice.
pub fn message_wash(message: &ChatMessage, me: Option<&str>) -> Option<Wash> {
    if me.is_some_and(|me| addressed_to(message, me)) {
        Some(Wash::Mention)
    } else if message.highlighted {
        Some(Wash::Highlight)
    } else if message.first {
        Some(Wash::FirstMessage)
    } else {
        None
    }
}

/// Whether `message` is addressed to `me`, a login: it answers one of
/// their messages, or names them as a word of its own, with an `@` or
/// without, in any case — `@Name`, `name,`, `NAME?` — which is what
/// Twitch, FrankerFaceZ and Chatterino all mark. Never their own line, and
/// never a word that only contains the name: `names` is not `name`.
pub fn addressed_to(message: &ChatMessage, me: &str) -> bool {
    if me.is_empty() || message.login.eq_ignore_ascii_case(me) {
        return false;
    }
    if message
        .reply
        .as_ref()
        .is_some_and(|parent| parent.login.eq_ignore_ascii_case(me))
    {
        return true;
    }
    message.text.split_whitespace().any(|word| {
        word.trim_matches(|c: char| !(c.is_alphanumeric() || c == '_'))
            .eq_ignore_ascii_case(me)
    })
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

    fn said(login: &str, tags: &str, text: &str) -> ChatMessage {
        let line = format!("@{tags} :{login}!{login}@{login} PRIVMSG #a :{text}");
        ChatMessage::from_irc(&parse_line(&line).unwrap()).unwrap()
    }

    #[test]
    fn a_message_is_washed_only_when_twitch_marks_it() {
        assert_eq!(message_wash(&message("color=#FF0000"), None), None);
        assert_eq!(
            message_wash(&message("first-msg=1"), None),
            Some(Wash::FirstMessage)
        );
        assert_eq!(
            message_wash(&message("msg-id=highlighted-message"), None),
            Some(Wash::Highlight)
        );
        assert_eq!(
            message_wash(&message("first-msg=1;msg-id=highlighted-message"), None),
            Some(Wash::Highlight)
        );
    }

    /// A line naming the reader, as a word of its own and in any case, or
    /// answering them, is theirs to see; their own lines and words that
    /// only contain the name are not.
    #[test]
    fn a_line_addressed_to_the_reader_is_marked() {
        let me = Some("viewer_1");
        for text in [
            "@viewer_1 hi",
            "hi @Viewer_1",
            "VIEWER_1, look",
            "is that you viewer_1?",
            "(viewer_1)",
        ] {
            assert_eq!(
                message_wash(&said("b", "", text), me),
                Some(Wash::Mention),
                "{text}"
            );
        }
        for text in ["viewer_12 hi", "@viewer hi", "xviewer_1", "nothing here"] {
            assert_eq!(message_wash(&said("b", "", text), me), None, "{text}");
        }
        // Their own line, naming themselves, is not news to them.
        assert_eq!(
            message_wash(&said("viewer_1", "", "viewer_1 here"), me),
            None
        );
        // Signed out, nobody is addressed.
        assert_eq!(message_wash(&said("b", "", "@viewer_1"), None), None);
        // A reply to one of theirs, whatever it says.
        let reply = said(
            "b",
            "reply-parent-user-login=viewer_1;reply-parent-display-name=Viewer_1;reply-parent-msg-body=hey",
            "agreed",
        );
        assert_eq!(message_wash(&reply, me), Some(Wash::Mention));
        // It outranks Twitch's own marks.
        assert_eq!(
            message_wash(&said("b", "msg-id=highlighted-message", "@viewer_1"), me),
            Some(Wash::Mention)
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
