//! A reply's context: the dim line above it naming the message it answers,
//! and the rule for the `@parent` Twitch starts its text with, which that
//! line already says and so is not drawn twice. The parsing is
//! `twitch_chat::Reply`'s and the wording `chat_words::reply_context`'s.

use emotes::Token;
use gpui::{div, prelude::*, px, SharedString};

use twitch_chat::Reply;

use super::ChatView;
use crate::chat_words;
use crate::theme;

/// Whether the first word of a message drawn from `tokens` is the `@parent`
/// of its `reply`, and so is not drawn (`message_line`).
///
/// Only the very first word, and only when no emote comes before it: an
/// `@parent` later in the text was written by the person, and a message
/// that opens with an emote did not open with Twitch's `@`. A word that
/// names somebody else is kept. The word is skipped when the line is drawn,
/// never cut from the text, because the `emotes` tag counts characters from
/// the text's start.
pub(super) fn skips_leading_mention(reply: Option<&Reply>, tokens: &[Token]) -> bool {
    let Some(reply) = reply else {
        return false;
    };
    for token in tokens {
        match token {
            Token::Text(text) => {
                if let Some(word) = text.split_whitespace().next() {
                    return reply.is_mention(word);
                }
            }
            Token::Emote(_) => return false,
        }
    }
    false
}

impl ChatView {
    /// The message a reply answers, as one dim line above it: `Replying to
    /// @name: what they said`, cut short with an ellipsis where it does not
    /// fit (`chat_words::reply_context`). In the meta size whatever the
    /// chat's text size, like the time, since it is the row's supporting
    /// information; one line always, so a reply row is its message and one
    /// short line taller, never more.
    ///
    /// `text_ellipsis` and `line_clamp(1)` on a full-width child of the
    /// column, not `truncate`: gpui only ellipsises with a definite width,
    /// which a flex column's child gets only on the wrapping path (HANDOFF,
    /// "Do not use `.truncate()`").
    pub(super) fn reply_line(reply: &Reply) -> gpui::Div {
        div()
            .w_full()
            .text_size(px(theme::TEXT_META))
            .line_height(px(theme::LINE_TIGHT))
            .text_color(theme::text_dim())
            .text_ellipsis()
            .line_clamp(1)
            .child(SharedString::from(chat_words::reply_context(reply)))
    }
}

#[cfg(test)]
mod tests {
    use emotes::{Emote, Token};
    use twitch_chat::Reply;

    use super::skips_leading_mention;

    fn parent() -> Reply {
        Reply {
            login: "someone".into(),
            display_name: "Some_One".into(),
            body: "is this the boss?".into(),
        }
    }

    fn text(words: &str) -> Token {
        Token::Text(words.into())
    }

    fn emote() -> Token {
        Token::Emote(Emote {
            name: "Kappa".into(),
            url: "https://example.invalid/kappa".into(),
        })
    }

    /// The `@parent` that opens a reply is skipped, by login or display
    /// name, even after leading space.
    #[test]
    fn a_leading_parent_is_skipped() {
        let reply = parent();
        assert!(skips_leading_mention(
            Some(&reply),
            &[text("@someone yes it is")]
        ));
        assert!(skips_leading_mention(
            Some(&reply),
            &[text("  @Some_One hi")]
        ));
        assert!(skips_leading_mention(
            Some(&reply),
            &[text("@someone "), emote()]
        ));
    }

    /// A message that opens with an emote opened with no `@`, even if one
    /// follows; a blank text before the emote does not count as a word.
    #[test]
    fn an_emote_first_skips_nothing() {
        let reply = parent();
        assert!(!skips_leading_mention(
            Some(&reply),
            &[emote(), text(" @someone hi")]
        ));
        assert!(!skips_leading_mention(
            Some(&reply),
            &[text(" "), emote(), text(" @someone")]
        ));
    }

    /// An `@parent` the person wrote later in the text is theirs, and kept.
    #[test]
    fn a_parent_not_first_is_kept() {
        let reply = parent();
        assert!(!skips_leading_mention(
            Some(&reply),
            &[text("yes @someone")]
        ));
    }

    /// A first word naming somebody else is kept, and so is anything in a
    /// message that is not a reply.
    #[test]
    fn another_name_or_no_reply_is_kept() {
        let reply = parent();
        assert!(!skips_leading_mention(Some(&reply), &[text("@other hi")]));
        assert!(!skips_leading_mention(
            Some(&reply),
            &[text("@someone, hi")]
        ));
        assert!(!skips_leading_mention(None, &[text("@someone hi")]));
        assert!(!skips_leading_mention(Some(&reply), &[]));
    }
}
