//! Chat badges, as the root runs them: hearing a chat's room, handing it the
//! badges known for it, asking the worker for the rest, and handing every
//! answer to the chats it is for. What to ask and what an answer comes to are
//! worked out purely in `crate::chat_badges`; this is where that meets the
//! chats and the sign-in.

use gpui::{Context, Entity};
use twitch_api::badges::BadgeSet;

use super::RootView;
use crate::browse::SignIn;
use crate::chat::ChatView;
use crate::twitch::Request;

impl RootView {
    /// A chat said its room (`ChatViewEvent::Room`): hand it what is known
    /// for that room now, and ask for the global badges and the room's own
    /// if nobody has. Said at every `ROOMSTATE`, which `Library::wants`
    /// keeps to one ask per book.
    pub(super) fn on_chat_room(
        &mut self,
        chat: Entity<ChatView>,
        room: &str,
        cx: &mut Context<Self>,
    ) {
        let badges = self.badges.for_room(Some(room));
        chat.update(cx, |chat, cx| chat.set_badges(badges, cx));
        self.ask_badges(None);
        self.ask_badges(Some(room.to_string()));
    }

    /// Ask the worker for one badge book, the global one (`None`) or a
    /// room's, if it is wanted. Only signed in, for the reason
    /// `ask_channel_name` gives, and because Helix needs the token; a book
    /// that cannot be asked for yet is noted as met and asked for when
    /// sign-in completes (`ask_met_badges`). Signed out it never is, and no
    /// chat draws a badge.
    fn ask_badges(&mut self, book: Option<String>) {
        if !self.badges.wants(&book) {
            return;
        }
        if matches!(self.sign_in, SignIn::SignedIn(_))
            && self.twitch.request(Request::Badges {
                channel: book.clone(),
            })
        {
            self.badges.asked(book);
        } else {
            self.badges.met(book);
        }
    }

    /// Ask for every book a chat needed while nothing could be asked, for
    /// sign-in completing, beside `ask_met_channel_names`.
    pub(super) fn ask_met_badges(&mut self) {
        for book in self.badges.take_met() {
            self.ask_badges(book);
        }
    }

    /// The worker's answer for one book. A failure is a line in the log;
    /// an answer goes to every chat it is for: all of them for the global
    /// book, the chats in that room for a channel's. Each mirrors it
    /// (`ChatView::set_badges`).
    pub(super) fn on_badges(
        &mut self,
        channel: Option<String>,
        result: Result<Vec<BadgeSet>, String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(reason) = self.badges.answered(channel.clone(), result) {
            eprintln!("chat badges: {reason}");
            return;
        }
        let chats: Vec<_> = self
            .slots
            .iter()
            .filter_map(|slot| slot.chat.clone())
            .collect();
        for chat in chats {
            let room = chat.read(cx).room_id().map(str::to_string);
            let Some(room) = room else {
                continue;
            };
            if channel.as_ref().is_some_and(|id| *id != room) {
                continue;
            }
            let badges = self.badges.for_room(Some(&room));
            chat.update(cx, |chat, cx| chat.set_badges(badges, cx));
        }
    }
}
