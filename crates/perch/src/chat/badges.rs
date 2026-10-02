//! The chat badges a chat draws: the books it is handed by the root
//! (`root::badges`, `chat_badges::Library`) and the group of pictures before
//! a speaker's name. Which badge has which picture and what its tooltip says
//! is `chat_badges`'s, pure and tested; this is where the chat view keeps
//! the books and lays the pictures out.

use gpui::{div, img, prelude::*, px, Context, SharedString};

use twitch_chat::ChatMessage;

use super::{ChatView, Row, RowKind};
use crate::chat_badges::{self, ChatBadges};
use crate::chat_display::Metrics;
use crate::controls;
use crate::theme;

impl ChatView {
    /// Draw badges with `badges` from now on: the root's books for this
    /// chat's room, handed over when the room becomes known and whenever an
    /// answer for it arrives (`root::badges`). Nothing for the books this
    /// chat already has.
    ///
    /// A row whose badges appear is wider on its first line and may wrap
    /// differently, and the list goes on trusting its old measurement, so if
    /// any row here wears a badge every row goes back unmeasured
    /// ([`remeasure`](Self::remeasure)), as for a Shared Chat label. Books
    /// arrive once each a session, so this is rare.
    pub fn set_badges(&mut self, badges: ChatBadges, cx: &mut Context<Self>) {
        if self.badges.same_as(&badges) {
            return;
        }
        self.badges = badges;
        let badged = |message: &ChatMessage| !message.badges.is_empty();
        let any = self.rows.iter().any(|row| match &row.kind {
            RowKind::Message(message) => badged(message),
            RowKind::Event(notice) => notice.body.as_ref().is_some_and(badged),
            RowKind::Notice(_) => false,
        });
        if any {
            self.remeasure();
        }
        cx.notify();
    }

    /// The room's numeric id, once known: what `root::badges` hands this
    /// chat the badges of.
    pub fn room_id(&self) -> Option<&str> {
        self.room_id.as_deref()
    }

    /// The badges a speaker wears, before their name, in Twitch's order, as
    /// one group that wraps as one: each the chat's badge size
    /// ([`Metrics::badge`]), which is under its line at every text size, in
    /// a box the line's height, so a row with badges is no taller than one
    /// without. Each says what it is under the pointer while the pointer is
    /// in the window (`chat_badges::tooltip`, `Subscriber, 14 months`), for
    /// the reason the Shared Chat tag does ([`source_tag`](Self::source_tag)).
    ///
    /// Only the badges this chat has art for (`ChatBadges::art`): none at
    /// all signed out, and then no group and no gap. A picture still on its
    /// way keeps its square, so the line does not move when it lands; the
    /// pictures come through the emote cache and are decoded into this
    /// pane's own image cache, like the emotes.
    pub(super) fn badge_group(
        &self,
        row: &Row,
        message: &ChatMessage,
        metrics: &Metrics,
    ) -> Option<gpui::Div> {
        let drawn: Vec<_> = message
            .badges
            .iter()
            .filter_map(|badge| self.badges.art(badge).map(|art| (badge, art)))
            .collect();
        if drawn.is_empty() {
            return None;
        }
        let size = px(metrics.badge);
        Some(
            div()
                .flex_none()
                .h(px(metrics.line))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::CHAT_BADGE_GAP))
                .children(drawn.into_iter().enumerate().map(|(index, (badge, art))| {
                    let picture = self.cache.get_or_request(&art.url);
                    let tip = chat_badges::tooltip(badge, art);
                    div()
                        .id(SharedString::from(format!(
                            "chat-badge-{}-{index}",
                            row.seq
                        )))
                        .flex_none()
                        .size(size)
                        .when_some(picture, |slot, path| {
                            slot.child(img(path).image_cache(&self.emote_images).size(size))
                        })
                        .when(self.window_hovered, |slot| slot.tooltip(controls::tip(tip)))
                })),
        )
    }
}
