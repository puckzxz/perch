//! One right-click menu wherever a channel is named: a stream card, an
//! offline name, a rail row, a name in chat, and a pane — whose header's ⋯
//! opens it too, for the pointer that does not reach for the right button.
//!
//! The same words everywhere, so a channel reads the same whichever list it
//! is in: `Watch` and `Add beside` while it is live, `Pin to the rail` or
//! `Unpin from the rail`, `Past broadcasts`, `Open on twitch.tv` and `Copy
//! link`. A click still does the one thing it always did — a card or a live
//! row plays in place of the pane you are on, a name opens its page or its
//! card — and the menu is where the rest of what can be done with the
//! channel is, one right-click away rather than spread over hover pills.
//!
//! On a pane it is also where what only two panes or more need lives —
//! `Only this one`, giving the pane the window, moving it earlier or later
//! — beside its pop-out and its close, so the bar over a picture keeps to
//! what one stream needs.
//!
//! What it offers is [`entries`], pure and tested, from what the root knows
//! as it is drawn ([`Facts`]), so a row is never offered from a stale
//! answer; the root carries out the pick (`RootView::channel_menu_pick`).
//! Drawn as the chat's copy menu is ([`view`]): deferred and anchored at the
//! pointer, or under the ⋯, its rows acting on the press, put away by a
//! press anywhere outside it, `Esc`, or the pick.

use gpui::{
    anchored, deferred, div, prelude::*, px, Animation, AnimationExt, AnyElement, Context, Corner,
    MouseButton, MouseDownEvent, Pixels, Point, Window,
};

use crate::loudness::HearOnly;
use crate::stage::MaximizeButton;
use crate::{sidebar, theme};

/// What the menu is about: a channel, by its login, with the name it writes
/// itself as and its Helix id where the list it came from had one, which
/// spares its page a lookup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel {
    pub login: String,
    pub display_name: String,
    pub user_id: Option<String>,
}

/// Where a pane's picture is, for its menu's pop-out row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopOut {
    /// Not offered: not on this platform, or no player to move.
    Hidden,
    /// At home, with a player: out into a window of its own.
    Out,
    /// In a window of its own: back.
    Back,
}

/// A pane's own part of its menu, from the panes as they are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneFacts {
    /// What hearing one pane alone offers now (`loudness::hear_only`).
    pub hear_only: HearOnly,
    /// What its maximize offers (`Stage::maximize_button`).
    pub maximize: MaximizeButton,
    /// Whether it can move one place earlier, or later, in the order `1`–`4`
    /// count: with two panes or more on the page, and not past an end.
    pub earlier: bool,
    pub later: bool,
    pub pop_out: PopOut,
}

/// What the root knows of the channel as the menu is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Facts {
    /// On now, by a list the root holds: what `Watch` and `Add beside` need.
    pub live: bool,
    /// A pane plays it already, which `Watch` then chooses, and so `Add
    /// beside` is not offered.
    pub open: bool,
    /// Another pane fits beside the ones open, and there is one to be
    /// beside (`RootView::can_add`).
    pub can_add: bool,
    pub pinned: bool,
    /// Signed in: the channel's page asks Helix for its lists with the
    /// token, as a viewer card's `Channel page` is offered only so.
    pub signed_in: bool,
    /// Opened on a name in chat, where it stands in for the copy menu that
    /// the rest of the line opens, and so keeps its `Copy name`.
    pub from_chat: bool,
    /// A pane's own menu, with what is the pane's; `None` for a channel
    /// named anywhere else.
    pub pane: Option<PaneFacts>,
}

/// What a row does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// Play it in place of the pane you are on (`Opening::InPlace`).
    Watch,
    /// Open it beside the panes (`Opening::Beside`).
    Add,
    /// Pin it, or take the pin off: says which, as every pin control does,
    /// so a row drawn a frame stale cannot do the opposite.
    Pin(bool),
    /// Its page in Perch: past broadcasts and schedule.
    ChannelPage,
    OpenOnTwitch,
    CopyLink,
    CopyName,
    /// The pane's: `Only this one`, or `Hear all again`.
    HearOnly,
    /// The pane's: the window to itself, or every pane again.
    Maximize,
    MoveEarlier,
    MoveLater,
    PopOut,
    PopIn,
    Close,
}

/// One row: its id within the menu, its words, what a press does, and
/// whether a rule sets it off from the row before, between groups.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: &'static str,
    pub label: &'static str,
    pub pick: Pick,
    pub rule: bool,
}

impl Row {
    fn new(id: &'static str, label: &'static str, pick: Pick) -> Self {
        Self {
            id,
            label,
            pick,
            rule: false,
        }
    }
}

/// What the menu offers, top to bottom, in up to three groups: what to
/// watch, a channel named anywhere but a pane, while it is live; the
/// channel's; and a pane's own. A row's id follows its words, as every
/// control whose words follow a state does.
pub fn entries(facts: Facts) -> Vec<Row> {
    let mut rows = Vec::new();

    if facts.pane.is_none() && facts.live {
        rows.push(Row::new("watch", "Watch", Pick::Watch));
        if facts.can_add && !facts.open {
            rows.push(Row::new("add", "Add beside", Pick::Add));
        }
    }

    let channel = rows.len();
    rows.push(Row::new(
        if facts.pinned { "unpin" } else { "pin" },
        sidebar::pin_words(facts.pinned),
        Pick::Pin(!facts.pinned),
    ));
    if facts.signed_in {
        rows.push(Row::new("videos", "Past broadcasts", Pick::ChannelPage));
    }
    rows.push(Row::new("twitch", "Open on twitch.tv", Pick::OpenOnTwitch));
    rows.push(Row::new("copy-link", "Copy link", Pick::CopyLink));
    if facts.from_chat {
        rows.push(Row::new("copy-name", "Copy name", Pick::CopyName));
    }
    if channel > 0 {
        rows[channel].rule = true;
    }

    if let Some(pane) = facts.pane {
        let own = rows.len();
        if let Some((id, label)) = pane.hear_only.row() {
            rows.push(Row::new(id, label, Pick::HearOnly));
        }
        match pane.maximize {
            MaximizeButton::Maximize => rows.push(Row::new("maximize", "Maximize", Pick::Maximize)),
            MaximizeButton::Restore => {
                rows.push(Row::new("show-all", "Show all panes", Pick::Maximize))
            }
            MaximizeButton::Hidden => {}
        }
        if pane.earlier {
            rows.push(Row::new("move-earlier", "Move earlier", Pick::MoveEarlier));
        }
        if pane.later {
            rows.push(Row::new("move-later", "Move later", Pick::MoveLater));
        }
        match pane.pop_out {
            PopOut::Out => rows.push(Row::new("pop-out", "Pop out", Pick::PopOut)),
            PopOut::Back => rows.push(Row::new("pop-in", "Bring back", Pick::PopIn)),
            PopOut::Hidden => {}
        }
        rows.push(Row::new("close", "Close", Pick::Close));
        rows[own].rule = true;
    }

    rows
}

/// The open menu, drawn after everything else in the window with its
/// `corner` at `position` — its top left at the pointer where a right-click
/// opened it, or its top right under the ⋯ that did — as the chat's copy
/// menu is: raised and ruled like the app's other menus, its rows acting on
/// the press — only the first of a run, as theirs do — and a press anywhere
/// outside it putting it away, heard in the capture phase so that press
/// still lands. `opened` keys its arrival, so a menu opened in place of
/// another fades in as itself.
pub fn view<V: 'static>(
    rows: &[Row],
    position: Point<Pixels>,
    corner: Corner,
    opened: u64,
    on_pick: impl Fn(&mut V, Pick, &mut Window, &mut Context<V>) + Clone + 'static,
    on_dismiss: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> AnyElement {
    // Under an id of its own, which the rows' ids sit in.
    let mut menu = div()
        .id("channel-menu")
        .flex()
        .flex_col()
        .min_w(px(theme::MENU_MIN_WIDTH))
        .py(px(theme::GAP_WORD))
        .rounded(px(theme::RADIUS_LG))
        .overflow_hidden()
        .bg(theme::surface_raised())
        .border_1()
        .border_color(theme::border())
        .shadow_lg()
        // Over a card, a row or a pane, whose own listeners would otherwise
        // hear a press on a row too.
        .occlude()
        .on_mouse_down_out(
            cx.listener(move |view, _: &MouseDownEvent, window, cx| on_dismiss(view, window, cx)),
        )
        // A menu drawn from inside a chat would take the chat's text size
        // and leading; this one is the app's, as every other menu is.
        .line_height(gpui::phi());
    for row in rows {
        let pick = row.pick;
        let on_pick = on_pick.clone();
        menu = menu
            .when(row.rule, |menu| {
                menu.child(div().my(px(theme::GAP_WORD)).h(px(1.0)).bg(theme::border()))
            })
            .child(
                div()
                    .id(row.id)
                    .px(px(theme::PANEL_PAD))
                    .py(px(theme::CONTROL_PAD_Y))
                    .text_size(px(theme::TEXT_LABEL))
                    .font_weight(theme::weight_label())
                    .whitespace_nowrap()
                    .cursor_pointer()
                    .text_color(theme::text())
                    .hover(|style| style.bg(theme::hover()))
                    .child(row.label)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                            if event.click_count > 1 {
                                return;
                            }
                            on_pick(view, pick, window, cx)
                        }),
                    ),
            );
    }
    let menu = menu.with_animation(
        ("channel-menu", opened as usize),
        Animation::new(theme::MOTION_ENTER).with_easing(theme::ease_enter()),
        |menu, delta| menu.opacity(delta),
    );
    deferred(anchored().anchor(corner).position(position).child(menu)).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Facts {
        Facts {
            live: true,
            open: false,
            can_add: true,
            pinned: false,
            signed_in: true,
            from_chat: false,
            pane: None,
        }
    }

    fn picks(rows: &[Row]) -> Vec<Pick> {
        rows.iter().map(|row| row.pick).collect()
    }

    /// A live channel named anywhere: watch it or add it, then the
    /// channel's own, ruled off from them.
    #[test]
    fn a_live_channel_offers_watching_first() {
        let rows = entries(facts());
        assert_eq!(
            picks(&rows),
            [
                Pick::Watch,
                Pick::Add,
                Pick::Pin(true),
                Pick::ChannelPage,
                Pick::OpenOnTwitch,
                Pick::CopyLink,
            ]
        );
        assert!(rows[2].rule, "the channel's own rows are not set off");
        assert!(!rows[0].rule, "a rule over the first row");
    }

    /// Off, there is nothing to watch: the channel's rows alone, with no
    /// rule over the first.
    #[test]
    fn an_offline_channel_offers_only_the_channel() {
        let rows = entries(Facts {
            live: false,
            ..facts()
        });
        assert_eq!(rows[0].pick, Pick::Pin(true));
        assert!(!rows[0].rule);
        assert!(!picks(&rows).contains(&Pick::Watch));
    }

    /// `Add beside` only with room, and not for a channel a pane plays,
    /// which `Watch` chooses; the pin says what it would do; the page only
    /// signed in; and a name in chat keeps `Copy name`.
    #[test]
    fn rows_follow_what_is_known() {
        let full = picks(&entries(Facts {
            can_add: false,
            ..facts()
        }));
        assert!(!full.contains(&Pick::Add));
        let open = picks(&entries(Facts {
            open: true,
            ..facts()
        }));
        assert!(open.contains(&Pick::Watch) && !open.contains(&Pick::Add));

        let pinned = entries(Facts {
            pinned: true,
            ..facts()
        });
        let pin = pinned
            .iter()
            .find(|row| matches!(row.pick, Pick::Pin(_)))
            .unwrap();
        assert_eq!(pin.pick, Pick::Pin(false));
        assert_eq!((pin.id, pin.label), ("unpin", "Unpin from the rail"));

        let signed_out = picks(&entries(Facts {
            signed_in: false,
            ..facts()
        }));
        assert!(!signed_out.contains(&Pick::ChannelPage));

        let chat = picks(&entries(Facts {
            from_chat: true,
            ..facts()
        }));
        assert_eq!(chat.last(), Some(&Pick::CopyName));
    }

    /// A pane's menu has no watching of its own, and adds what is the
    /// pane's — hearing it alone, the window, moving it, its pop-out — then
    /// its close, ruled off from the channel's rows.
    #[test]
    fn a_pane_adds_its_own_rows() {
        let pane = PaneFacts {
            hear_only: HearOnly::Offered,
            maximize: MaximizeButton::Maximize,
            earlier: true,
            later: false,
            pop_out: PopOut::Out,
        };
        let rows = entries(Facts {
            pane: Some(pane),
            ..facts()
        });
        assert_eq!(
            picks(&rows),
            [
                Pick::Pin(true),
                Pick::ChannelPage,
                Pick::OpenOnTwitch,
                Pick::CopyLink,
                Pick::HearOnly,
                Pick::Maximize,
                Pick::MoveEarlier,
                Pick::PopOut,
                Pick::Close,
            ]
        );
        let own = rows
            .iter()
            .position(|row| row.pick == Pick::HearOnly)
            .unwrap();
        assert!(rows[own].rule);

        // One pane alone: nothing but its pop-out and its close.
        let alone = picks(&entries(Facts {
            pane: Some(PaneFacts {
                hear_only: HearOnly::Hidden,
                maximize: MaximizeButton::Hidden,
                earlier: false,
                later: false,
                pop_out: PopOut::Back,
            }),
            ..facts()
        }));
        assert_eq!(&alone[alone.len() - 2..], [Pick::PopIn, Pick::Close]);
        assert!(!alone.contains(&Pick::HearOnly) && !alone.contains(&Pick::Maximize));
    }

    /// Ids are unique within a menu, since they key the rows' elements.
    #[test]
    fn ids_are_unique() {
        let rows = entries(Facts {
            from_chat: true,
            pane: Some(PaneFacts {
                hear_only: HearOnly::Held,
                maximize: MaximizeButton::Restore,
                earlier: true,
                later: true,
                pop_out: PopOut::Out,
            }),
            ..facts()
        });
        let mut ids: Vec<&str> = rows.iter().map(|row| row.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before, "{ids:?}");
    }
}
