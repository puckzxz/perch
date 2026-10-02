//! Sending chat, as the root runs it: what each live chat's composer is
//! told about the user, a message handed to the worker, and its answer
//! handed back to the chat it was typed into. Whether a composer is open,
//! what a message may be and what an answer comes to are worked out purely
//! in `chat::composer`; this is where that meets the sign-in, the follows
//! and the worker.

use gpui::{Context, Entity, WeakEntity, Window};
use twitch_api::chat::SendOutcome;

use super::RootView;
use crate::browse::SignIn;
use crate::chat::composer::{self, Access, Account, Step};
use crate::chat::ChatView;
use crate::twitch::{Request, SendFailure};

impl RootView {
    /// What a live chat on `channel` is told about the user: the sign-in
    /// with whether its token may send, whether this is their own channel,
    /// and whether they follow it, once a follows list has said.
    fn chat_access(&self, channel: &str) -> Access {
        let own_channel = matches!(
            &self.sign_in,
            SignIn::SignedIn(login) if login.eq_ignore_ascii_case(channel)
        );
        let found = self
            .follows
            .iter()
            .any(|stream| stream.user_login.eq_ignore_ascii_case(channel))
            || self
                .offline
                .iter()
                .any(|followed| followed.login.eq_ignore_ascii_case(channel));
        let follows = composer::follows(
            matches!(self.sign_in, SignIn::SignedIn(_)) && self.follows_loaded,
            self.follows_complete,
            found,
        );
        Access {
            account: Account::of(&self.sign_in, self.chat_scope),
            own_channel,
            follows,
        }
    }

    /// Tell `chat` what it should know about the user, if it is a live
    /// chat: for a chat just made (`watch_chat`).
    pub(super) fn tell_chat_access(&self, chat: &Entity<ChatView>, cx: &mut Context<Self>) {
        let Some(channel) = chat.read(cx).live_channel().map(str::to_string) else {
            return;
        };
        let access = self.chat_access(&channel);
        chat.update(cx, |chat, cx| chat.set_access(access, cx));
    }

    /// Tell every live chat what it should know about the user. Run after
    /// every event from the worker (`spawn_twitch`), since any of them can
    /// change the sign-in, the scope or the follows, and after the user
    /// signs out or in; a chat told what it already knew does nothing.
    pub(super) fn sync_chat_access(&mut self, cx: &mut Context<Self>) {
        let chats: Vec<_> = self
            .slots
            .iter()
            .filter_map(|slot| slot.chat.clone())
            .collect();
        for chat in chats {
            self.tell_chat_access(&chat, cx);
        }
    }

    /// A composer's `Enter`: ask the worker to send `message` to the room
    /// `room_id` names, and remember which chat asked, by a number of the
    /// root's own, so the answer finds it even if the panes have moved
    /// since. A chat that has closed by the time the answer comes is simply
    /// not told. With no worker to take it, the chat is told at once.
    pub(super) fn send_chat(
        &mut self,
        chat: &Entity<ChatView>,
        room_id: String,
        message: String,
        cx: &mut Context<Self>,
    ) {
        if !matches!(self.sign_in, SignIn::SignedIn(_)) {
            chat.update(cx, |chat, cx| {
                chat.sent(Some("Message not sent: not signed in".into()), cx)
            });
            return;
        }
        self.chat_send_seq += 1;
        let id = self.chat_send_seq;
        if self.twitch.request(Request::SendChat {
            id,
            broadcaster_id: room_id,
            message,
        }) {
            self.chat_sends.insert(id, chat.downgrade());
        } else {
            chat.update(cx, |chat, cx| {
                chat.sent(Some("Message not sent: the sign-in stopped".into()), cx)
            });
        }
    }

    /// The worker's answer for send `id`, handed to the chat that sent it.
    /// A token without the scope says so here as well as in the notice:
    /// every composer closes with `Sign in again to chat`, which a token
    /// validated before the scope was asked for would otherwise not learn
    /// for an hour.
    pub(super) fn on_chat_sent(
        &mut self,
        id: u64,
        result: Result<SendOutcome, SendFailure>,
        cx: &mut Context<Self>,
    ) {
        if matches!(result, Ok(SendOutcome::MissingScope)) {
            self.chat_scope = Some(false);
        }
        let Some(chat) = self.chat_sends.remove(&id).and_then(|chat| chat.upgrade()) else {
            return;
        };
        let notice = match &result {
            Ok(outcome) => composer::outcome_notice(outcome),
            Err(failure) => Some(composer::failure_notice(failure)),
        };
        chat.update(cx, |chat, cx| chat.sent(notice, cx));
    }

    /// Every send still waiting on an answer that will now never come — its
    /// worker has stopped, or returned — told to its chat as `notice`.
    pub(super) fn drop_chat_sends(&mut self, notice: &str, cx: &mut Context<Self>) {
        let waiting: Vec<WeakEntity<ChatView>> =
            self.chat_sends.drain().map(|(_, chat)| chat).collect();
        for chat in waiting.iter().filter_map(WeakEntity::upgrade) {
            chat.update(cx, |chat, cx| chat.sent(Some(notice.to_string()), cx));
        }
    }

    /// The button beside a closed composer's reason.
    pub(super) fn on_composer_step(
        &mut self,
        step: &Step,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match step {
            Step::SignIn => self.start_sign_in(window, cx),
            Step::SignInAgain => self.sign_in_again(window, cx),
            Step::OpenSettings => {
                if self.settings_panel.is_none() {
                    self.toggle_settings(window, cx);
                }
            }
            // The chat opens the page itself; it never asks the root.
            Step::OpenActivate(_) => {}
        }
    }
}
