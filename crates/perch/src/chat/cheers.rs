//! The cheers a chat draws: the book it is handed by the root
//! (`root::cheers`, `chat_cheers::Library`) and a cheer in a line, its
//! cheermote's picture with the Bits it spent after it. Which words are
//! cheers is `emotes::apply_cheers`'s and `chat_cheers`'s, pure and tested;
//! this is where the chat view keeps the book and lays the cheer out.
//!
//! A cheer is drawn as an emote is (`emote`): its picture at the emote
//! height in a box a line high, hanging past the line above and below, so a
//! row with a cheer is no taller than one without; animated under an id of
//! its own; its tooltip in the emote's look (`EmoteTooltip`), only while the
//! pointer is in the window; and, until the picture is on disk, the word's
//! prefix in its place, dim, as an emote's name stands in for an emote. The
//! amount follows it in bold, in the tier's colour, lifted as far as it must
//! be to read on the row's own paint (`theme::readable_on`,
//! `ChatView::row_ground`). The two are one piece of the line, so they never
//! wrap apart. Nothing in it says what a right press landed on, so the press
//! opens the row's copy menu as on the row's words: a cheer has no name to
//! copy that the message's text does not already carry.

use emotes::Cheer;
use gpui::{div, img, prelude::*, px, AnyElement, Context, Hsla, SharedString};

use twitch_chat::ChatMessage;

use super::emote::EmoteTooltip;
use super::{ChatView, RowKind};
use crate::chat_cheers::ChatCheers;
use crate::chat_display::Metrics;
use crate::chat_words;
use crate::theme;

impl ChatView {
    /// Draw cheers with `cheers` from now on: the root's book for this live
    /// chat's room, handed over when the room becomes known and when its
    /// answer arrives (`root::cheers`). Nothing for the book this chat
    /// already has.
    ///
    /// A word that becomes a picture and an amount changes its line's width
    /// and may wrap it differently, and the list goes on trusting its old
    /// measurement, so if any row here spent Bits every row goes back
    /// unmeasured ([`remeasure`](Self::remeasure)), as for badges. A book
    /// arrives once a room a session, so this is rare.
    pub fn set_cheers(&mut self, cheers: ChatCheers, cx: &mut Context<Self>) {
        if self.cheers.same_as(&cheers) {
            return;
        }
        self.cheers = cheers;
        let cheered = |message: &ChatMessage| message.bits.is_some();
        let any = self.rows.iter().any(|row| match &row.kind {
            RowKind::Message(message) => cheered(message),
            RowKind::Event(notice) => notice.body.as_ref().is_some_and(cheered),
            RowKind::Notice(_) => false,
        });
        if any {
            self.remeasure();
        }
        cx.notify();
    }

    /// One cheer of a message line, the `index`-th picture in it (emotes
    /// and cheers count together, so no two share an id in a row), with its
    /// amount in `amount_color`.
    ///
    /// The id is the picture's, as an emote's is, and the word's too, behind
    /// a `cheer ` no emote's id starts with: `Cheer100` and `Cheer150` are
    /// one picture, and the tooltip keeps the words it came up with for as
    /// long as its id lasts, which across the pane's rows would be the other
    /// word's.
    ///
    /// Until the picture's file is on disk, the word's prefix as typed
    /// stands in for it, dim, as an emote's name does, so the line still
    /// reads as sent: on its way, and for as long as the image cache cannot
    /// fetch it, which it tries again every half minute and never gives up
    /// on. The amount moves once, when the picture lands, as the words
    /// after an emote do. From then the box keeps an emote's width while
    /// gpui decodes the file, which an `img` with no frames yet has none of.
    pub(super) fn cheer_piece(
        &self,
        cheer: Cheer,
        index: usize,
        amount_color: Hsla,
        metrics: &Metrics,
    ) -> AnyElement {
        let amount = cheer.amount.to_string();
        let first = match self.cache.get_or_request(&cheer.url) {
            Some(path) => {
                let id = SharedString::from(format!("cheer {} {}", cheer.name, cheer.url));
                let picture = img(path)
                    .image_cache(&self.emote_images)
                    .id((id, index))
                    .h(px(metrics.emote))
                    .mt(px(-metrics.overhang))
                    .when(self.window_hovered, move |picture| {
                        picture.tooltip(move |_window, cx| {
                            cx.new(|_| EmoteTooltip::new(chat_words::cheer_tip(&cheer)))
                                .into()
                        })
                    });
                div()
                    .flex_none()
                    .h(px(metrics.line))
                    .min_w(px(metrics.emote + 2.0 * theme::EMOTE_PAD_X))
                    .px(px(theme::EMOTE_PAD_X))
                    .child(picture)
            }
            None => {
                // The amount is the word's digits after the prefix, with no
                // leading zero (`ChatCheers::find`), so it is the word's end.
                let prefix = cheer.name.strip_suffix(&amount).unwrap_or(&cheer.name);
                div()
                    .flex_none()
                    .text_color(theme::text_dim())
                    .child(SharedString::from(prefix.to_string()))
            }
        };
        div()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .child(first)
            .child(
                div()
                    .flex_none()
                    .font_weight(theme::weight_title())
                    .text_color(amount_color)
                    .child(SharedString::from(amount)),
            )
            .into_any_element()
    }
}
