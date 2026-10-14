//! Cheers, as the root runs them: hearing a live chat's room, handing it the
//! cheermotes known for it, asking the worker for them otherwise, and handing
//! the answer to the live chats in that room. What to ask and what an answer
//! comes to are worked out purely in `crate::chat_cheers`; this is where that
//! meets the chats and the sign-in.

use gpui::{Context, Entity};
use twitch_api::cheermotes::Cheermote;

use super::RootView;
use crate::browse::SignIn;
use crate::chat::ChatView;
use crate::twitch::Request;

impl RootView {
    /// A chat said its room (`ChatViewEvent::Room`): if it is a live chat,
    /// hand it the cheermotes known for that room now, and ask for them if
    /// nobody has. A replay is left out: its lines never say they spent
    /// Bits, so it has nothing to draw them with. Said at every
    /// `ROOMSTATE`, which `Library::wants` keeps to one ask per room.
    pub(super) fn cheers_for_room(
        &mut self,
        chat: &Entity<ChatView>,
        room: &str,
        cx: &mut Context<Self>,
    ) {
        if chat.read(cx).live_channel().is_none() {
            return;
        }
        let cheers = self.cheers.for_room(room);
        chat.update(cx, |chat, cx| chat.set_cheers(cheers, cx));
        self.ask_cheermotes(room.to_string());
    }

    /// Ask the worker for one room's cheermotes, if they are wanted. Only
    /// signed in, as the badges are and for their reasons; a room that
    /// cannot be asked about yet is noted as met and asked about when
    /// sign-in completes (`ask_met_cheermotes`). Signed out it never is,
    /// so a room first met signed out has every cheer as the word typed;
    /// one answered before a sign-out keeps its book (`Library::forget`
    /// drops asks, not answers).
    fn ask_cheermotes(&mut self, room: String) {
        if !self.cheers.wants(&room) {
            return;
        }
        if matches!(self.sign_in, SignIn::SignedIn(_))
            && self
                .twitch
                .request(Request::Cheermotes { room: room.clone() })
        {
            self.cheers.asked(room);
        } else {
            self.cheers.met(room);
        }
    }

    /// Ask for every room a live chat met while nothing could be asked, for
    /// sign-in completing, beside `ask_met_badges`.
    pub(super) fn ask_met_cheermotes(&mut self) {
        for room in self.cheers.take_met() {
            self.ask_cheermotes(room);
        }
    }

    /// The worker's answer for one room. A failure is a line in the log; an
    /// answer goes to every live chat in that room, which mirrors it
    /// (`ChatView::set_cheers`).
    pub(super) fn on_cheermotes(
        &mut self,
        room: String,
        result: Result<Vec<Cheermote>, String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(reason) = self.cheers.answered(room.clone(), result) {
            eprintln!("chat cheers: {reason}");
            return;
        }
        let chats: Vec<_> = self
            .slots
            .iter()
            .filter_map(|slot| slot.chat.clone())
            .filter(|chat| {
                let chat = chat.read(cx);
                chat.live_channel().is_some() && chat.room_id() == Some(room.as_str())
            })
            .collect();
        for chat in chats {
            let cheers = self.cheers.for_room(&room);
            chat.update(cx, |chat, cx| chat.set_cheers(cheers, cx));
        }
    }
}
