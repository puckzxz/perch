//! The channel menu (`crate::channel_menu`) as the root runs it: opening it
//! where a channel is named — a card, an offline name or a rail row
//! (`browse::Action::ChannelMenu`), a name in chat
//! (`ChatViewEvent::ChannelMenu`), a pane (`PaneAction::Menu`) — what it
//! knows of the channel each time it is drawn, and carrying out a pick.
//!
//! One menu at a time across the window: opening it closes every other menu
//! (`close_menus`), and it closes with them, on `Esc` and whenever a modal
//! comes up (`close_copy_menus`), since like the copy menu it is drawn
//! deferred, over everything, the modals included.

use gpui::{AnyElement, Context, Pixels, Point, Window};

use super::panes::pane_keys;
use super::streams::Opening;
use super::{pop_out, RootView};
use crate::browse::SignIn;
use crate::channel_menu::{self, Channel, Facts, PaneFacts, Pick, PopOut};
use crate::stage;
use crate::target::{self, Target};
use crate::watch::PaneAction;

/// The menu open now: about which channel, opened from where — a pane, by
/// its key, a name in chat, or anywhere else — and where in the window.
pub(super) struct OpenChannelMenu {
    channel: Channel,
    /// Whether what it opened on was live: a stream card or a live rail
    /// row, rather than an offline name. A name in chat and a pane are
    /// asked of the lists the root holds.
    live: bool,
    from_chat: bool,
    pane: Option<String>,
    position: Point<Pixels>,
    /// Which opening this is, keying its arrival (`channel_menu::view`).
    opened: u64,
}

impl RootView {
    /// Open the menu about `channel` at `position`, in place of any other
    /// menu.
    pub(super) fn open_channel_menu(
        &mut self,
        channel: Channel,
        live: bool,
        from_chat: bool,
        pane: Option<String>,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.close_menus(cx);
        self.channel_menus_opened += 1;
        self.channel_menu = Some(OpenChannelMenu {
            channel,
            live,
            from_chat,
            pane,
            position,
            opened: self.channel_menus_opened,
        });
        cx.notify();
    }

    /// A right-click on the pane `key` names, on its picture or its header:
    /// its menu, about the channel it plays, with the pane's own rows. The
    /// pane becomes the one the keys talk to, as a press on it would have
    /// made it.
    pub(super) fn open_pane_menu(
        &mut self,
        key: &str,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.slot_index(key) else {
            return;
        };
        let slot = &self.slots[index];
        let login = slot.channel.clone();
        let user_id = slot
            .recording()
            .map(|video| video.user_id.clone())
            .filter(|id| !id.is_empty())
            .or_else(|| self.known_user_id(&login));
        let channel = Channel {
            display_name: self.display_name(slot),
            login,
            user_id,
        };
        let live = slot.is_live();
        self.active = Some(key.to_string());
        self.open_channel_menu(channel, live, false, Some(key.to_string()), position, cx);
    }

    /// A right-click on a name in chat: the menu about that person's
    /// channel, live if a list the root holds has them on.
    pub(super) fn open_chat_name_menu(
        &mut self,
        login: String,
        display_name: String,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let live = self.stream_info(&login).is_some();
        let user_id = self.known_user_id(&login);
        let channel = Channel {
            login,
            display_name,
            user_id,
        };
        self.open_channel_menu(channel, live, true, None, position, cx);
    }

    /// Close the menu, if it is open. Returns whether it was.
    pub(super) fn close_channel_menu(&mut self, cx: &mut Context<Self>) -> bool {
        let was = self.channel_menu.take().is_some();
        if was {
            cx.notify();
        }
        was
    }

    /// What the menu says, from what the root knows now.
    fn channel_menu_facts(&self, menu: &OpenChannelMenu) -> Facts {
        let login = &menu.channel.login;
        Facts {
            live: menu.live,
            open: self.slot_index(login).is_some(),
            can_add: self.can_add(),
            pinned: self.settings.is_pinned(login),
            signed_in: matches!(self.sign_in, SignIn::SignedIn(_)),
            from_chat: menu.from_chat,
            pane: menu
                .pane
                .as_deref()
                .and_then(|key| self.pane_menu_facts(key)),
        }
    }

    /// A pane's own part of its menu, by the answers its other controls
    /// read: hearing it alone (`hear_only_now`), its maximize
    /// (`Stage::maximize_button`), moving it (`move_active`'s rule: two
    /// panes or more on the page, none being dragged, and not past an end,
    /// `stage::moved`), and its pop-out (`pop_out::offered_here`, and a
    /// player to move).
    fn pane_menu_facts(&self, key: &str) -> Option<PaneFacts> {
        let index = self.slot_index(key)?;
        let keys = pane_keys(&self.slots);
        let movable = self.cells().len() >= 2 && self.pane_move.is_none();
        let moves = |delta| movable && stage::moved(index, delta, self.slots.len()).is_some();
        let pop_out = if self.stage.is_popped(key) {
            PopOut::Back
        } else if pop_out::offered_here() && self.slots[index].video().is_some() {
            PopOut::Out
        } else {
            PopOut::Hidden
        };
        Some(PaneFacts {
            hear_only: self.hear_only_now(),
            maximize: self.stage.maximize_button(key, &keys),
            earlier: moves(-1),
            later: moves(1),
            pop_out,
        })
    }

    /// The menu, while it is open; nothing, and closed, once the pane it is
    /// about has gone.
    pub(super) fn channel_menu_view(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.channel_menu.as_ref()?;
        if menu
            .pane
            .as_deref()
            .is_some_and(|key| self.slot_index(key).is_none())
        {
            self.channel_menu = None;
            return None;
        }
        let rows = channel_menu::entries(self.channel_menu_facts(menu));
        let (position, opened) = (menu.position, menu.opened);
        Some(channel_menu::view(
            &rows,
            position,
            opened,
            |this: &mut RootView, pick, window, cx| this.channel_menu_pick(pick, window, cx),
            |this: &mut RootView, _window, cx| {
                this.close_channel_menu(cx);
            },
            cx,
        ))
    }

    /// A row pressed: the menu closes, the keys come back to the root, and
    /// the rest of the press's run is heard by nothing (`run_guard`), since
    /// whatever the pick does moves what is under the pointer. A pane's
    /// rows go by its key, the way its own controls do (`on_pane_action`),
    /// so they do what its More and its header do; a channel's elsewhere go
    /// the way a card's press and a rail row's go.
    fn channel_menu_pick(&mut self, pick: Pick, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.channel_menu.take() else {
            return;
        };
        self.take_rest_of_run();
        self.focus.focus(window);
        if let Some(key) = menu.pane.as_deref() {
            self.pane_menu_pick(key, pick, window, cx);
            cx.notify();
            return;
        }
        let Channel {
            login,
            display_name,
            user_id,
        } = menu.channel;
        match pick {
            Pick::Watch => self.open_channel(login, Opening::InPlace, window, cx),
            Pick::Add => self.open_channel(login, Opening::Beside, window, cx),
            Pick::Pin(pinned) => self.set_pinned(login, pinned, cx),
            Pick::ChannelPage => {
                let user_id = user_id.or_else(|| self.known_user_id(&login));
                self.open_channel_page(login, display_name, user_id, window, cx);
            }
            Pick::OpenOnTwitch => cx.open_url(&target::link(&Target::Channel(login))),
            Pick::CopyLink => self.copy(target::link(&Target::Channel(login)), "Link copied", cx),
            Pick::CopyName => self.copy(display_name, "Name copied", cx),
            // A pane's own, never offered anywhere else.
            Pick::HearOnly
            | Pick::Maximize
            | Pick::MoveEarlier
            | Pick::MoveLater
            | Pick::PopOut
            | Pick::PopIn
            | Pick::Close => {}
        }
        cx.notify();
    }

    /// A row of a pane's menu, for the pane `key` names.
    fn pane_menu_pick(
        &mut self,
        key: &str,
        pick: Pick,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let action = match pick {
            Pick::Pin(pinned) => PaneAction::Pin(pinned),
            Pick::ChannelPage => PaneAction::ChannelPage,
            Pick::OpenOnTwitch => PaneAction::OpenOnTwitch,
            Pick::CopyLink => PaneAction::CopyLink,
            Pick::HearOnly => PaneAction::HearOnly,
            Pick::Maximize => PaneAction::Maximize,
            Pick::PopOut => PaneAction::PopOut,
            Pick::PopIn => PaneAction::PopIn,
            Pick::Close => PaneAction::Close,
            // `Shift+←` and `Shift+→`, for this pane.
            Pick::MoveEarlier | Pick::MoveLater => {
                self.active = Some(key.to_string());
                let delta = if pick == Pick::MoveEarlier { -1 } else { 1 };
                self.move_active(delta, cx);
                return;
            }
            // Never offered on a pane: it plays already.
            Pick::Watch | Pick::Add | Pick::CopyName => return,
        };
        self.on_pane_action(key, action, window, cx);
    }
}
