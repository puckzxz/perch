//! How every chat is drawn: how big its text is, and whether each message
//! carries its own time. The two options in the chat options menu on a pane's
//! header (`watch::chat_menu`), kept in [`settings::Settings`] and handed to
//! every chat as one [`ChatDisplay`].
//!
//! The sizes are names on disk (`settings::ChatTextSize`) and pixels here,
//! so what a step comes to can change without a settings file having to.
//! Everything a row's height is made of follows the text size from one
//! place, [`Metrics::of`], so a larger step cannot grow the words without
//! growing the line they sit on, the emotes in it and the padding those
//! hang into.

use settings::{ChatTextSize, Settings};

use crate::{chat_text, theme};

/// What every chat is drawn with: the root's settings, mirrored onto each
/// `ChatView` when it is made and again whenever the menu changes them
/// (`RootView::set_chat_display`), the one place they change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChatDisplay {
    pub size: ChatTextSize,
    /// Every message starts with its time, and the once-a-minute breaks go.
    pub times: bool,
}

impl ChatDisplay {
    /// What `settings` say.
    pub fn of(settings: &Settings) -> Self {
        Self {
            size: settings.chat_text_size,
            times: settings.chat_message_times,
        }
    }

    /// Put this into `settings`, and say whether that changed them, so a
    /// press on what is already chosen does not rewrite the file.
    pub fn store(self, settings: &mut Settings) -> bool {
        if Self::of(settings) == self {
            return false;
        }
        settings.chat_text_size = self.size;
        settings.chat_message_times = self.times;
        true
    }
}

/// What the menu calls each step.
pub fn size_label(size: ChatTextSize) -> &'static str {
    match size {
        ChatTextSize::Small => "Small",
        ChatTextSize::Default => "Default",
        ChatTextSize::Large => "Large",
        ChatTextSize::Larger => "Larger",
    }
}

/// The body text size of a step, in pixels. `Default` is
/// [`theme::TEXT_BODY`] itself, so a chat nobody has changed is drawn exactly
/// as it always was. Small is a pixel below it; Large and Larger go up from
/// it a pixel and a half at a time.
pub fn text_size(size: ChatTextSize) -> f32 {
    match size {
        ChatTextSize::Small => 12.0,
        ChatTextSize::Default => theme::TEXT_BODY,
        ChatTextSize::Large => 14.5,
        ChatTextSize::Larger => 16.0,
    }
}

/// Rendered emote height at body size. Twitch's 2.0 assets are around 56px,
/// so this halves them and keeps them crisp on a HiDPI display. At other
/// sizes it is scaled with the text ([`Metrics::of`]).
///
/// An emote overhangs its line rather than growing it — see the wrapper in
/// `ChatView::message_line` — so at this height it comes within about half a
/// pixel of the hairline above and below. That is deliberate: shrinking the
/// emote to buy clearance costs more than the crowding does.
const EMOTE_HEIGHT: f32 = 28.0;

/// Everything a chat row's height is made of, at one text size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// The body text size.
    pub text: f32,
    /// Its leading: [`theme::LINE_BODY`] in the ratio it has to
    /// [`theme::TEXT_BODY`], so a larger step reads as the same chat, bigger,
    /// rather than as the same lines squeezed together.
    pub line: f32,
    /// How tall an emote is drawn: [`EMOTE_HEIGHT`] in proportion.
    pub emote: f32,
    /// How far an emote hangs past its line, top and bottom.
    ///
    /// At the top and bottom of a row this is absorbed by `row_pad_y`.
    /// *Between* two wrapped lines of the same message there is no padding
    /// at all — the lines sit exactly `line` apart — so an emote on the
    /// second line paints over the descenders of the first, and an emote on
    /// the first is painted over in turn. A message with an emote in it is
    /// given a gap of twice this between its lines, which gives the overhang
    /// somewhere to go.
    pub overhang: f32,
    /// A row's padding above and below: [`theme::ROW_PAD_Y`], grown with the
    /// text above body size and never shrunk below it. Everything else here
    /// scales, so the overhang does too, and at the larger steps it would
    /// otherwise reach past the padding into the row next door. Scaled
    /// rather than set to the overhang, so it keeps the half-pixel clearance
    /// the body size has, scaled as well.
    pub row_pad_y: f32,
    /// The most characters a piece of a long word is drawn as
    /// ([`chat_text::piece_chars`]): fewer as the glyphs get wider, so a
    /// piece still fits the narrowest chat on a line of its own.
    pub piece_chars: usize,
}

impl Metrics {
    /// The metrics of `size`. At `Default` they are exactly the body size's
    /// constants, which is what this replaced.
    pub fn of(size: ChatTextSize) -> Self {
        let text = text_size(size);
        let scale = text / theme::TEXT_BODY;
        let line = theme::LINE_BODY * scale;
        let emote = EMOTE_HEIGHT * scale;
        Self {
            text,
            line,
            emote,
            overhang: (emote - line) / 2.0,
            row_pad_y: theme::ROW_PAD_Y * scale.max(1.0),
            piece_chars: chat_text::piece_chars(text),
        }
    }
}

/// Whether the row stamped `stamp` is drawn under a break with its time, the
/// row above it stamped `previous` (`None` for the first row): when it says
/// something the row above did not — and never while every message carries
/// its own time, which says it on every row instead.
pub fn draws_time_break(previous: Option<&str>, stamp: &str, times: bool) -> bool {
    !times && previous != Some(stamp)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nobody who never opens the menu sees a pixel change: the default step
    /// is the body size, its line, its emotes and its padding exactly.
    #[test]
    fn the_default_step_is_chat_as_it_was() {
        assert_eq!(ChatTextSize::default(), ChatTextSize::Default);
        let metrics = Metrics::of(ChatTextSize::Default);
        assert_eq!(metrics.text, theme::TEXT_BODY);
        assert_eq!(metrics.line, theme::LINE_BODY);
        assert_eq!(metrics.emote, 28.0);
        assert_eq!(metrics.overhang, 4.5);
        assert_eq!(metrics.row_pad_y, theme::ROW_PAD_Y);
        assert_eq!(metrics.piece_chars, chat_text::PIECE_CHARS);
    }

    /// The steps go up in order, and every one keeps the body size's ratio
    /// of line to text and of emote to text.
    #[test]
    fn every_step_keeps_the_body_sizes_proportions() {
        let ratio = theme::LINE_BODY / theme::TEXT_BODY;
        let mut last = 0.0;
        for size in ChatTextSize::ALL {
            let metrics = Metrics::of(size);
            assert!(
                metrics.text > last,
                "{size:?} is not larger than the step before it"
            );
            last = metrics.text;
            assert!(
                (metrics.line / metrics.text - ratio).abs() < 1e-4,
                "{size:?}"
            );
            assert!((metrics.emote / metrics.text - 28.0 / theme::TEXT_BODY).abs() < 1e-4);
        }
    }

    /// An emote's overhang always has padding to sit in, with at least the
    /// half pixel to spare the body size has, and the padding never shrinks
    /// below the body size's.
    #[test]
    fn an_emote_never_reaches_past_its_rows_padding() {
        for size in ChatTextSize::ALL {
            let metrics = Metrics::of(size);
            assert!(metrics.row_pad_y >= theme::ROW_PAD_Y, "{size:?}");
            assert!(
                metrics.row_pad_y - metrics.overhang >= 0.5 - 1e-4,
                "{size:?}: {metrics:?}"
            );
        }
    }

    /// With the times off a break is drawn above each new minute, the first
    /// row's included; with them on, never.
    #[test]
    fn a_time_break_is_drawn_only_without_times_on_rows() {
        assert!(draws_time_break(None, "15:27", false));
        assert!(draws_time_break(Some("15:26"), "15:27", false));
        assert!(!draws_time_break(Some("15:27"), "15:27", false));
        for previous in [None, Some("15:26"), Some("15:27")] {
            assert!(!draws_time_break(previous, "15:27", true));
        }
    }

    /// Storing what the settings already say changes nothing; anything else
    /// is written and reported.
    #[test]
    fn storing_reports_a_change_only() {
        let mut settings = Settings::default();
        let display = ChatDisplay::of(&settings);
        assert!(!display.store(&mut settings));
        let larger = ChatDisplay {
            size: ChatTextSize::Larger,
            times: true,
        };
        assert!(larger.store(&mut settings));
        assert_eq!(ChatDisplay::of(&settings), larger);
        assert!(!larger.store(&mut settings));
    }
}
