//! Shared Chat labels, as the root runs them: hearing from a chat that it met
//! a partner it cannot name, asking the worker, and handing the answer to
//! every chat. Which ids to ask about, and what an answer comes to, are
//! worked out purely in `crate::shared_chat`; this is where that meets the
//! chats and the sign-in.

use gpui::{Context, Entity};
use twitch_api::Channel;

use super::RootView;
use crate::browse::SignIn;
use crate::chat::{ChatView, ChatViewEvent};
use crate::twitch::{RecommendError, Request};

impl RootView {
    /// Listen to a chat made for a pane, live or a replay, and hand it every
    /// Shared Chat name already known, so a partner met in another pane is
    /// named here from the first line. It says when it meets a partner it
    /// cannot name (only a live chat ever does) and what its room is, for
    /// its badges (`root::badges`), and asks for what a row of its copy menu
    /// copies to go on the clipboard (`copy`). The subscription lasts as
    /// long as the chat does.
    pub(super) fn watch_chat(&mut self, chat: &Entity<ChatView>, cx: &mut Context<Self>) {
        let known: Vec<_> = self
            .source_rooms
            .labels()
            .iter()
            .map(|(id, label)| (id.clone(), label.clone()))
            .collect();
        if !known.is_empty() {
            chat.update(cx, |chat, cx| chat.learn_rooms(&known, cx));
        }
        cx.subscribe(chat, |this: &mut RootView, chat, event, cx| match event {
            ChatViewEvent::UnknownRoom(id) => this.ask_channel_name(id),
            ChatViewEvent::Room(room) => this.on_chat_room(chat, room, cx),
            // A row of the menu acts on the press and has closed it; the
            // rest of the press's run would land on whatever the closed menu
            // left under the pointer, a link in chat among them (`run_guard`).
            ChatViewEvent::Copy { text, toast } => {
                this.take_rest_of_run();
                this.copy(text.clone(), toast, cx);
            }
        })
        .detach();
    }

    /// A chat met a line copied from a room it cannot name: ask the worker,
    /// if nobody has. Said by a chat at every such line, and cheap enough to
    /// be: `SourceRooms::wants` asks nothing about an id already asked
    /// about, named or given up on. Only signed in, as `ask_last_live` is
    /// and for its reason: the worker reads its requests only once sign-in
    /// has got it into its loop. An id that cannot be asked about yet, or
    /// whose ask the worker could not take, is noted as met
    /// (`SourceRooms::met`): the next line from that room tries again, and
    /// so does sign-in completing (`ask_met_channel_names`).
    fn ask_channel_name(&mut self, id: &str) {
        if !self.source_rooms.wants(id) {
            return;
        }
        if matches!(self.sign_in, SignIn::SignedIn(_))
            && self.twitch.request(Request::ChannelNames {
                ids: vec![id.to_string()],
            })
        {
            self.source_rooms.asked(id);
        } else {
            self.source_rooms.met(id);
        }
    }

    /// Ask about every partner a chat met while nothing could be asked: a
    /// pane opened during sign-in loads its history, copies and all, before
    /// the worker can take an ask, and a quiet partner might not speak again
    /// for a long time. For sign-in completing, beside `ask_missing`.
    pub(super) fn ask_met_channel_names(&mut self) {
        for id in self.source_rooms.take_met() {
            self.ask_channel_name(&id);
        }
    }

    /// The worker's answer for `ids`. Whatever names it brought go to every
    /// chat, each of which mirrors them (`ChatView::learn_rooms`); a
    /// failure, or a refusal, is one line in the log and nothing on screen.
    pub(super) fn on_channel_names(
        &mut self,
        ids: Vec<String>,
        result: Result<Vec<Channel>, RecommendError>,
        cx: &mut Context<Self>,
    ) {
        let (learned, log) = self.source_rooms.answered(&ids, result);
        if let Some(reason) = log {
            eprintln!("shared chat: {reason}");
        }
        if learned.is_empty() {
            return;
        }
        let chats: Vec<_> = self
            .slots
            .iter()
            .filter_map(|slot| slot.chat.clone())
            .collect();
        for chat in chats {
            chat.update(cx, |chat, cx| chat.learn_rooms(&learned, cx));
        }
    }
}
