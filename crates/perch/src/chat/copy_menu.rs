//! A chat row's right-click menu: what can be copied from it.
//!
//! Chat is mostly read, and the one thing a reader wants out of a line is
//! the line — a link someone posted, a name to look up, what an
//! emote is called to use it elsewhere. Drag-selecting text is not offered
//! by gpui's text, and a message here is a row of separate words and
//! pictures anyway (see the module above), so the menu copies the pieces
//! whole: `Copy message`, the text as it was sent, emote names written out
//! where the pictures are; `Copy name`; `Copy link`, while the row has one,
//! the one under the pointer or else its first; and `Copy emote name`,
//! when the pointer was on an emote. What each copies is [`entries`]'s,
//! pure and tested.
//!
//! Opened by the right button anywhere on a row ([`ChatView::row_menu`],
//! which gpui delivers as `MouseButton::Right` on Windows, from
//! `WM_RBUTTONDOWN`, as on the other platforms), at the pointer, drawn
//! deferred and anchored so it may reach past the chat's edge and is
//! never clipped by the list, and turned back from the window's edges by
//! `anchored`. A link or an emote says it was the one pressed
//! ([`ChatView::press_target`]) before the row hears the press, which goes
//! on to the root's own mouse-down like any other, so the keys come back
//! to the root as they do for a press anywhere on the page.
//!
//! Dismissed as the app's other menus are: a press anywhere outside it,
//! heard in the capture phase (`on_mouse_down_out`), so that press still
//! reaches whatever it landed on — a right-click on another row opens that
//! row's menu in its place — and `Esc`, through `RootView::close_menus`.
//! Its rows act on the press, for the reason `video_view::menu::menu_row`
//! gives, and a test below holds this file to it; the clipboard is written
//! by the root (`RootView::copy`), which says so in a toast and swallows
//! the rest of the press's run (`RootView::run_guard`), so a double-click
//! on a row cannot open the link the closed menu leaves under the pointer.
//!
//! While it is open the chat holds its rows as it does under the pointer
//! (`ChatView::sync_hold`), wherever the pointer goes, so the row it is
//! about does not scroll away from under it; when it closes the hold is the
//! pointer's again, and a pointer still over the chat goes on holding it —
//! `Chat paused` stays up rather than the rows landing on the close.

use gpui::{
    anchored, deferred, div, prelude::*, px, Animation, AnimationExt, AnyElement, Context,
    MouseButton, MouseDownEvent, Pixels, Point, SharedString,
};

use super::{ChatView, ChatViewEvent, RowKind};
use crate::chat_text::{self, Kind};
use crate::theme;

/// What under the pointer a right-click on a row landed on, as far as the
/// menu cares: a link, by where it points, an emote, by its name, or the
/// row itself, anywhere else on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Target {
    Row,
    Link(String),
    Emote(String),
}

/// One row of the menu: its words, what it puts on the clipboard, and the
/// toast that says it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Entry {
    pub label: &'static str,
    pub text: String,
    pub toast: &'static str,
}

/// An open menu: where it was opened, in the window, and what it offers,
/// worked out from the row when it opened, so a row trimmed off the top of
/// the backlog or greyed by a moderator meanwhile still copies what was
/// there. `opened` keys its arrival, so a menu opened in place of another
/// fades in as itself.
pub(super) struct CopyMenu {
    position: Point<Pixels>,
    entries: Vec<Entry>,
    opened: u64,
}

/// What the menu offers for a row of `kind`, opened on `target`, in the
/// order it lists them. A message's text is as sent: Twitch's own emotes
/// and everyone else's are words in it, which is what the pictures were
/// drawn from, and a reply keeps the `@name` it started with. An event
/// copies the note attached to it, or Twitch's sentence when it has none,
/// and its sender's name only when a note says who; the app's own notices
/// are the words alone.
pub(super) fn entries(kind: &RowKind, target: &Target) -> Vec<Entry> {
    let (text, name, links) = match kind {
        RowKind::Message(message) => (
            message.text.clone(),
            Some(message.display_name.clone()),
            links(&message.text),
        ),
        RowKind::Event(notice) => match &notice.body {
            Some(body) => (
                body.text.clone(),
                Some(body.display_name.clone()),
                links(&body.text),
            ),
            None => (notice.system.clone(), None, Vec::new()),
        },
        RowKind::Notice(text) => (text.to_string(), None, Vec::new()),
    };

    let mut entries = Vec::new();
    if !text.is_empty() {
        entries.push(Entry {
            label: "Copy message",
            text,
            toast: "Message copied",
        });
    }
    if let Some(name) = name.filter(|name| !name.is_empty()) {
        entries.push(Entry {
            label: "Copy name",
            text: name,
            toast: "Name copied",
        });
    }
    // The link pressed, if a link was; otherwise the row's first.
    let link = match target {
        Target::Link(url) => Some(url.clone()),
        _ => links.into_iter().next(),
    };
    if let Some(link) = link {
        entries.push(Entry {
            label: "Copy link",
            text: link,
            toast: "Link copied",
        });
    }
    if let Target::Emote(name) = target {
        entries.push(Entry {
            label: "Copy emote name",
            text: name.clone(),
            toast: "Emote name copied",
        });
    }
    entries
}

/// Every link in `text`, in order, as the chat opens them: the words
/// `chat_text` reads as links, without the punctuation around them, a bare
/// host given its `https://` (`chat_text::Word::url`).
fn links(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(chat_text::classify)
        .filter(|word| word.kind == Kind::Link)
        .map(|word| word.url())
        .collect()
}

impl ChatView {
    /// The listener a link or an emote hangs on the right button: it says
    /// it was what the press landed on, for its row's own listener, which
    /// hears the press next, to open the menu with.
    pub(super) fn press_target(
        target: Target,
        cx: &mut Context<Self>,
    ) -> impl Fn(&MouseDownEvent, &mut gpui::Window, &mut gpui::App) + 'static {
        // By hand rather than through `cx.listener`, whose returned closure
        // borrows `cx` for as long as it is held.
        let view = cx.entity().downgrade();
        move |_: &MouseDownEvent, _window: &mut gpui::Window, cx: &mut gpui::App| {
            view.update(cx, |this, _cx| this.pressed = Some(target.clone()))
                .ok();
        }
    }

    /// The right button's listener on the row `seq`: open its menu at the
    /// pointer, about whatever a link or an emote in it said it was, or
    /// the row. The menu already open closed in the capture phase, if the
    /// press was outside it; this opens the new one in its place.
    pub(super) fn row_menu(
        seq: u64,
        cx: &mut Context<Self>,
    ) -> impl Fn(&MouseDownEvent, &mut gpui::Window, &mut gpui::App) + 'static {
        let view = cx.entity().downgrade();
        move |event: &MouseDownEvent, _window: &mut gpui::Window, cx: &mut gpui::App| {
            view.update(cx, |this, cx| {
                let target = this.pressed.take().unwrap_or(Target::Row);
                this.open_menu(seq, &target, event.position, cx);
            })
            .ok();
        }
    }

    /// Open the menu for the row `seq` at `position`, if the row is still
    /// here and has anything to copy.
    fn open_menu(
        &mut self,
        seq: u64,
        target: &Target,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self.rows.iter().find(|row| row.seq == seq) else {
            return;
        };
        let entries = entries(&row.kind, target);
        if entries.is_empty() {
            return;
        }
        self.menus_opened += 1;
        self.menu = Some(CopyMenu {
            position,
            entries,
            opened: self.menus_opened,
        });
        cx.notify();
    }

    /// Close the menu, if it is open. Returns whether it was, so `Esc` can
    /// take back the menu before it takes you off the page.
    pub fn close_menu(&mut self, cx: &mut Context<Self>) -> bool {
        self.pressed = None;
        if self.menu.take().is_none() {
            return false;
        }
        cx.notify();
        true
    }

    /// The row at `index` of the open menu, pressed: close the menu and
    /// hand what it copies to the root.
    fn pick(&mut self, index: usize, cx: &mut Context<Self>) {
        let entry = self
            .menu
            .as_ref()
            .and_then(|menu| menu.entries.get(index).cloned());
        self.close_menu(cx);
        if let Some(entry) = entry {
            cx.emit(ChatViewEvent::Copy {
                text: entry.text,
                toast: entry.toast,
            });
        }
    }

    /// The open menu, if there is one, for the chat's own element: a layer
    /// drawn after everything else in the window, at the pointer where it
    /// was opened, styled as the app's other menus are.
    pub(super) fn copy_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.menu.as_ref()?;
        let rows: Vec<_> = menu
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| menu_row(index, entry.label.into(), cx))
            .collect();
        let menu_box = div()
            .flex()
            .flex_col()
            .min_w(px(theme::MENU_MIN_WIDTH))
            .rounded(px(theme::RADIUS_LG))
            .overflow_hidden()
            .bg(theme::surface_raised())
            .border_1()
            .border_color(theme::border())
            // Over the chat and maybe a pane beside it, whose rows, links
            // and picture would otherwise hear a press on a row too.
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                this.close_menu(cx);
            }))
            // Drawn from inside the chat, it would inherit the chat's text
            // size and leading (`chat_display`), which a menu does not
            // follow: it is the app's, as every other menu is. The rows set
            // their size; this puts back gpui's own leading under them.
            .line_height(gpui::phi())
            .children(rows)
            // Mounted only while open, so a one-shot is enough, as the bar's
            // menus are; it opens at the pointer rather than out of a
            // button, so it fades in without a rise.
            .with_animation(
                ("chat-copy-menu", menu.opened as usize),
                Animation::new(theme::MOTION_ENTER).with_easing(theme::ease_enter()),
                |menu, delta| menu.opacity(delta),
            );
        Some(deferred(anchored().position(menu.position).child(menu_box)).into_any_element())
    }
}

/// One row of the menu: its words, and on the press, what it copies. Only
/// the first press of a run, as the bar's rows: a later one is never a
/// row's to take, since whatever the run's first press landed on chose.
fn menu_row(
    index: usize,
    label: SharedString,
    cx: &mut Context<ChatView>,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(("chat-copy-row", index))
        .px(px(theme::PANEL_PAD))
        .py(px(theme::CONTROL_PAD_Y))
        .text_size(px(theme::TEXT_LABEL))
        .font_weight(theme::weight_label())
        .whitespace_nowrap()
        .cursor_pointer()
        .text_color(theme::text())
        .hover(|style| style.bg(theme::hover()))
        .child(label)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                if event.click_count > 1 {
                    return;
                }
                this.pick(index, cx);
            }),
        )
}

#[cfg(test)]
mod tests {
    use twitch_chat::{ChatMessage, ChatNotice};

    use super::*;

    fn message(text: &str) -> ChatMessage {
        ChatMessage {
            login: "fisher".into(),
            display_name: "Fisher".into(),
            color: 0,
            text: text.into(),
            is_action: false,
            emotes: None,
            sent_at: None,
            id: None,
            source_room: None,
            badges: Vec::new(),
            reply: None,
            first: false,
            highlighted: false,
        }
    }

    fn labels(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.label).collect()
    }

    fn copied<'a>(entries: &'a [Entry], label: &str) -> Option<&'a str> {
        entries
            .iter()
            .find(|entry| entry.label == label)
            .map(|entry| entry.text.as_str())
    }

    /// A plain message offers its text and its speaker, and no link.
    #[test]
    fn a_message_copies_its_text_and_name() {
        let row = RowKind::Message(Box::new(message("good morning Kappa")));
        let entries = entries(&row, &Target::Row);
        assert_eq!(labels(&entries), ["Copy message", "Copy name"]);
        // The emote is its name, as it was sent.
        assert_eq!(copied(&entries, "Copy message"), Some("good morning Kappa"));
        assert_eq!(copied(&entries, "Copy name"), Some("Fisher"));
    }

    /// With links, the row's first is offered from anywhere on the row, and
    /// the one pressed when a link was; a bare host is copied as the chat
    /// opens it, and its punctuation is left behind.
    #[test]
    fn a_link_copies_the_one_pressed_or_the_first() {
        let row = RowKind::Message(Box::new(message(
            "see (example.com/a), then https://example.org/b!",
        )));
        let anywhere = entries(&row, &Target::Row);
        assert_eq!(
            labels(&anywhere),
            ["Copy message", "Copy name", "Copy link"]
        );
        assert_eq!(
            copied(&anywhere, "Copy link"),
            Some("https://example.com/a")
        );

        let pressed = entries(&row, &Target::Link("https://example.org/b".into()));
        assert_eq!(copied(&pressed, "Copy link"), Some("https://example.org/b"));
        assert_eq!(
            links("see (example.com/a), then https://example.org/b!"),
            ["https://example.com/a", "https://example.org/b"]
        );
    }

    /// An emote pressed adds its name, last; the message is still offered
    /// whole.
    #[test]
    fn an_emote_copies_its_name() {
        let row = RowKind::Message(Box::new(message("nice catJAM")));
        let entries = entries(&row, &Target::Emote("catJAM".into()));
        assert_eq!(
            labels(&entries),
            ["Copy message", "Copy name", "Copy emote name"]
        );
        assert_eq!(copied(&entries, "Copy emote name"), Some("catJAM"));
        assert_eq!(copied(&entries, "Copy message"), Some("nice catJAM"));
    }

    /// An event copies the note attached to it and who wrote it; without a
    /// note, Twitch's sentence and no name. The app's own notices are their
    /// words.
    #[test]
    fn events_and_notices_copy_what_they_say() {
        let notice = |body: Option<ChatMessage>| ChatNotice {
            kind: twitch_chat::NoticeKind::Subscription,
            system: "Fisher subscribed for 3 months.".into(),
            body,
            sent_at: None,
            source_room: None,
            announcement_color: None,
        };

        let noted = RowKind::Event(Box::new(notice(Some(message("three months PogChamp")))));
        let entries_noted = entries(&noted, &Target::Row);
        assert_eq!(labels(&entries_noted), ["Copy message", "Copy name"]);
        assert_eq!(
            copied(&entries_noted, "Copy message"),
            Some("three months PogChamp")
        );

        let bare = RowKind::Event(Box::new(notice(None)));
        let entries_bare = entries(&bare, &Target::Row);
        assert_eq!(labels(&entries_bare), ["Copy message"]);
        assert_eq!(
            copied(&entries_bare, "Copy message"),
            Some("Fisher subscribed for 3 months.")
        );

        let ours = RowKind::Notice("Connected to fisher's chat".into());
        let entries_ours = entries(&ours, &Target::Row);
        assert_eq!(labels(&entries_ours), ["Copy message"]);
        assert_eq!(
            copied(&entries_ours, "Copy message"),
            Some("Connected to fisher's chat")
        );
    }

    /// Each entry says what it copied, in a sentence-case toast.
    #[test]
    fn every_entry_says_what_it_copied() {
        let row = RowKind::Message(Box::new(message("example.com Kappa")));
        let entries = entries(&row, &Target::Emote("Kappa".into()));
        let toasts: Vec<_> = entries.iter().map(|entry| entry.toast).collect();
        assert_eq!(
            toasts,
            [
                "Message copied",
                "Name copied",
                "Link copied",
                "Emote name copied"
            ]
        );
    }

    /// The bar's menu test, for this one: a row that waited for a click
    /// would never hear it, since the press closes the menu and takes the
    /// row away before the release. Held against the file's own source, as
    /// no test without a window can press one.
    #[test]
    fn menu_rows_act_on_the_press() {
        let source = include_str!("copy_menu.rs");
        let code = source
            .split("#[cfg(test)]")
            .next()
            .expect("the file has code above its tests");
        let code: String = code.split_whitespace().collect();
        assert!(
            !code.contains(".on_click("),
            "something in the chat's copy menu waits for a click, which never comes"
        );
        assert!(
            code.contains(".on_mouse_down(MouseButton::Left,"),
            "the chat's copy menu's rows no longer act on the press"
        );
    }
}
