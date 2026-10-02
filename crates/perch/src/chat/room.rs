//! What the room itself says, as the chat view hears it: its `ROOMSTATE`
//! (the room's id and its modes), a person timed out or banned, and the
//! quiet line of modes at the foot of the pane. The words are
//! `chat_words`'s and the merge is `twitch_chat::RoomModes`'s; this is where
//! the chat view keeps them and puts them on screen.

use gpui::{div, prelude::*, px, Context, SharedString};

use twitch_chat::{ChatMessage, ModeUpdate};

use super::{ChatView, ChatViewEvent, RowKind};
use crate::chat_words;
use crate::theme;

/// What a person said in a row, if a person said anything in it: a
/// message, or the note attached to an event. The app's own notices are
/// nobody's.
fn said(kind: &RowKind) -> Option<&ChatMessage> {
    match kind {
        RowKind::Message(message) => Some(message.as_ref()),
        RowKind::Event(notice) => notice.body.as_ref(),
        RowKind::Notice(_) => None,
    }
}

impl ChatView {
    /// A `ROOMSTATE`: the room's id, for the third-party emotes and the
    /// badges, and whatever modes it speaks of.
    ///
    /// The emote sets load for a new room, and again on a full line, the one
    /// Twitch sends on joining and on every rejoin after a dropped
    /// connection (`ModeUpdate::is_full`): that reload is what retries a
    /// provider whose fetch failed or timed out, so a chat that lost one gets
    /// it back at its next reconnect. Not at every line: a live room sends
    /// one naming a single mode each time a moderator changes it, and the
    /// sets do not change with it. The root hears of the room every time
    /// ([`ChatViewEvent::Room`]).
    ///
    /// The modes are merged over what the room was. The first line sets
    /// them without a word, since joining is not a change anybody made;
    /// every line after says what it changed as a notice row, a reconnect's
    /// full line included, which says only what moved while the chat was
    /// away. A line that speaks of no mode (a replay's) leaves them alone.
    pub(super) fn room_state(
        &mut self,
        room_id: String,
        update: ModeUpdate,
        cx: &mut Context<Self>,
    ) {
        if self.room_id.as_deref() != Some(room_id.as_str()) || update.is_full() {
            self.emote_loader.load_channel(room_id.clone());
            self.room_id = Some(room_id.clone());
        }
        cx.emit(ChatViewEvent::Room(room_id));
        if update.is_empty() {
            return;
        }
        let after = self.modes.unwrap_or_default().merged(update);
        if let Some(before) = self.modes {
            for change in before.changes(&after) {
                self.append(
                    RowKind::Notice(chat_words::mode_notice(change).into()),
                    None,
                );
            }
        }
        self.modes = Some(after);
    }

    /// One person timed out or banned: said in a notice that names them as
    /// their messages do, and every row of theirs already shown greyed where
    /// it stands, the way a `CLEARMSG` greys one (`Row::deleted`). Rows of
    /// theirs still waiting on the pointer were never shown, and are not
    /// shown now, as for a `CLEARMSG`.
    ///
    /// Theirs means a message of theirs, or an event carrying a note they
    /// wrote (a resub's message): the note is greyed with the `deleted` tag
    /// as a message is (`message_line`), and Twitch's sentence above it is
    /// left as it is, since that is Twitch speaking, not them.
    pub(super) fn clear_person(&mut self, login: &str, ban_seconds: Option<u64>) {
        let theirs =
            |kind: &RowKind| said(kind).is_some_and(|m| m.login.eq_ignore_ascii_case(login));
        // Their display name, from the newest thing of theirs to hand,
        // since `CLEARCHAT` carries only the login.
        let name = self
            .held
            .iter()
            .rev()
            .map(|(kind, _)| kind)
            .chain(self.rows.iter().rev().map(|row| &row.kind))
            .filter(|kind| theirs(kind))
            .find_map(|kind| said(kind).map(|message| message.display_name.clone()))
            .unwrap_or_else(|| login.to_string());
        for row in self.rows.iter_mut().filter(|row| theirs(&row.kind)) {
            row.deleted = true;
        }
        self.held.retain(|(kind, _)| !theirs(kind));
        self.append(
            RowKind::Notice(chat_words::ban_notice(&name, ban_seconds).into()),
            None,
        );
    }

    /// The room's modes, while any is on, as one quiet line at the foot of
    /// the pane under the list (`Slow mode 30s · Sub-only`,
    /// `chat_words::modes_line`): the dim meta style the notices use, over a
    /// hairline, and one line always, ellipsised in a narrow chat. Not a row
    /// in the list, which scrolls: a mode is the state of the room now, and
    /// a change to one is the notice row.
    pub(super) fn modes_line(modes: String) -> gpui::Div {
        div()
            .flex_none()
            .w_full()
            .px(px(theme::ROW_PAD_X))
            .py(px(theme::GAP_WORD))
            .border_t_1()
            .border_color(theme::divider())
            .text_size(px(theme::TEXT_META))
            .line_height(px(theme::LINE_TIGHT))
            .text_color(theme::text_dim())
            .text_ellipsis()
            .line_clamp(1)
            .child(SharedString::from(modes))
    }
}
