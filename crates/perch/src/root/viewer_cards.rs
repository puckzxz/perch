//! Viewer cards, as the root runs them: hearing a chat open a person's card,
//! asking the worker who they are, telling every open card what is known,
//! and carrying out the card's `Watch` and `Channel page`. What to ask and
//! what an answer comes to are worked out purely in `crate::chat_profiles`;
//! the card itself is `chat::viewer_card`. This is where that meets the
//! chats, the sign-in and the panes.

use gpui::{Context, Entity, Window};
use twitch_api::users::User;

use super::RootView;
use crate::browse::{Action, SignIn};
use crate::chat::viewer_card::CardFacts;
use crate::chat::ChatView;
use crate::chat_profiles::Lookup;
use crate::twitch::Request;

impl RootView {
    /// Tell `chat`'s open card, if it has one, what the root knows of its
    /// person, asking the worker about their account first if nobody has.
    /// Only signed in, for the reason the badges are (`ask_badges`); a
    /// failed answer is not wanted again until the card is opened afresh
    /// (`chat_profiles::Profiles::reopened`), so this can run after every
    /// worker event without asking in a loop.
    pub(super) fn tell_card(&mut self, chat: &Entity<ChatView>, cx: &mut Context<Self>) {
        let Some(login) = chat.read(cx).card_login().map(str::to_string) else {
            return;
        };
        let signed_in = matches!(self.sign_in, SignIn::SignedIn(_));
        if signed_in
            && self.profiles.wants(&login)
            && self.twitch.request(Request::Profile {
                login: login.clone(),
            })
        {
            self.profiles.asked(&login);
        }
        let facts = CardFacts {
            profile: self.profiles.lookup(&login, signed_in),
            watchable: self.can_add()
                && self.slot_index(&login).is_none()
                && self.live_info(&login).is_some(),
            channel_page: signed_in,
        };
        chat.update(cx, |chat, cx| chat.set_card_facts(&login, facts, cx));
    }

    /// Tell every open card what it should know: after every event from the
    /// worker (`spawn_twitch`), since any can change the sign-in or who is
    /// live; when the user signs out or the worker starts over; and at
    /// every `restage`, since the panes there are decide whether `Watch` is
    /// offered. A card told what it already knows does nothing.
    pub(super) fn sync_viewer_cards(&mut self, cx: &mut Context<Self>) {
        let chats: Vec<_> = self
            .slots
            .iter()
            .filter_map(|slot| slot.chat.clone())
            .collect();
        for chat in chats {
            self.tell_card(&chat, cx);
        }
    }

    /// A chat opened `login`'s card: a failure about their account is
    /// forgotten, so this opening asks again, and the card is told.
    pub(super) fn on_card_opened(
        &mut self,
        chat: &Entity<ChatView>,
        login: &str,
        cx: &mut Context<Self>,
    ) {
        self.profiles.reopened(login);
        self.tell_card(chat, cx);
    }

    /// The worker's answer about `login`'s account. A failure is a line in
    /// the log; either way every open card hears what is known now.
    pub(super) fn on_profile(
        &mut self,
        login: String,
        result: Result<Option<User>, String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(reason) = self.profiles.answered(&login, result) {
            eprintln!("viewer card: {reason}");
        }
        self.sync_viewer_cards(cx);
    }

    /// Close every chat's card; returns whether one was open. For `Esc`
    /// with the menus, and for the guide coming up or going
    /// (`close_menus`).
    pub(super) fn close_viewer_cards(&mut self, cx: &mut Context<Self>) -> bool {
        let chats: Vec<_> = self
            .slots
            .iter()
            .filter_map(|slot| slot.chat.clone())
            .collect();
        let mut closed = false;
        for chat in chats {
            closed |= chat.update(cx, |chat, cx| chat.close_card(cx));
        }
        closed
    }

    /// A card's `Watch`: `login`'s stream beside the panes, as the guide's
    /// `+ Add` does (`open_channel`, and one step on the trail), on the
    /// watch page. The grid changes under the pointer, so the rest of the
    /// press's run is heard by nothing (`run_guard`).
    pub(super) fn watch_from_card(
        &mut self,
        login: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.take_rest_of_run();
        self.on_browse_action(Action::Add(login), window, cx);
    }

    /// A card's `Channel page`: Perch's page of `login`'s past broadcasts
    /// and schedule (`open_channel_page`), which leaves the watch page.
    /// Offered only signed in (`CardFacts::channel_page`), as the page's
    /// lists need the token.
    ///
    /// Leaving stops every pane at home while the mini player is off, and
    /// Back does not bring them back, so a first press that would stop any
    /// (`leaving_stops_panes`) goes nowhere: the card is asked to ask for a
    /// second press instead (`ChatView::confirm_leaving`), and only that one,
    /// `confirmed`, goes. Decided here, at the press, from what is true now.
    /// The account's id goes with it when it is known, which spares the
    /// page a lookup.
    pub(super) fn channel_page_from_card(
        &mut self,
        chat: &Entity<ChatView>,
        login: String,
        display_name: String,
        confirmed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !confirmed && self.leaving_stops_panes() {
            chat.update(cx, |chat, cx| chat.confirm_leaving(cx));
            return;
        }
        self.take_rest_of_run();
        let user_id = match self.profiles.lookup(&login, true) {
            Lookup::Known(user) => Some(user.id.clone()).filter(|id| !id.is_empty()),
            _ => None,
        };
        self.open_channel_page(login, display_name, user_id, window, cx);
    }
}
