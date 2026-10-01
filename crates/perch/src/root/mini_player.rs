//! The mini player: what is playing, in the bottom-right corner of the browse
//! page, with its sound, while you pick the next thing.
//!
//! It replaced a now-playing bar docked along the bottom of the page, which
//! showed each stream as a 96px muted thumbnail. That bar existed because an
//! earlier floating strip covered two cards at 1000px and let clicks through
//! to them. Floating again is safe for two reasons that strip lacked: the
//! player blocks the pointer from what is under it, and every browse list
//! leaves room at its foot for it (`layout::mini_reserve`), so the last row
//! can always be scrolled out from under it.
//!
//! It is drawn by the root, under the title bar and above the page, rather
//! than by the browse page — so moving between tabs, categories and channels
//! never rebuilds it, and the toasts, the settings sheet and the palette,
//! drawn after it, cover it. It is placed against the page's own column, so
//! it never reaches over the rail, and in from the column's right edge by
//! more than the list's scrollbar, so the whole track stays the list's.

use gpui::{div, prelude::*, px, Context, ElementId, IntoElement, SharedString};
use gpui_component::tooltip::Tooltip;

use super::{Page, RootView};
use crate::assets::Icon;
use crate::controls::{self, Variant};
use crate::layout::{self, MiniLayout};
use crate::watch::{self, Showing, Slot};
use crate::{loudness, motion, theme};

/// The group a tile's close watches for the pointer, so it shows only while
/// the pointer is on that tile.
const TILE_GROUP: &str = "mini-tile";

impl RootView {
    /// Whether the mini player is up: on the browse page, with the miniplayer
    /// on, and something playing.
    ///
    /// Whenever there are panes this is exactly the complement of the watch
    /// page, so a player is never drawn twice in one frame. One answer, read
    /// by the player itself and by the room the browse lists leave for it.
    pub(super) fn mini_player_shows(&self) -> bool {
        self.page == Page::Browse && self.settings.miniplayer && !self.slots.is_empty()
    }

    /// The player, bottom-right of the page, or nothing; see
    /// [`mini_player_shows`](Self::mini_player_shows).
    pub(super) fn mini_player(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if !self.mini_player_shows() {
            return None;
        }
        let mini = layout::mini_player(self.slots.len());

        let mut tiles = div().flex().flex_col().gap(px(theme::PANE_GAP));
        for row in self.slots.chunks(mini.cols) {
            let mut line = div().flex().flex_row().gap(px(theme::PANE_GAP));
            for slot in row {
                line = line.child(self.mini_tile(slot, mini, cx));
            }
            tiles = tiles.child(line);
        }

        let playing: Vec<(String, bool)> = self
            .slots
            .iter()
            .map(|slot| {
                let paused = slot.video().is_some_and(|view| view.read(cx).is_paused());
                (self.display_name(slot), paused)
            })
            .collect();
        let unmute = loudness::unmute_all_offered(self.slots.iter().map(|slot| slot.quiet));

        // What is playing is said here, in the bar, and never over the
        // pictures: static text on a moving image is the thing you end up
        // staring past.
        let bar = div()
            .flex_none()
            .h(px(theme::MINI_BAR_HEIGHT))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::GAP_WORD))
            .pl(px(theme::GAP_TIGHT))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_ellipsis()
                    .line_clamp(1)
                    .text_size(px(theme::TEXT_META))
                    .line_height(px(theme::LINE_TIGHT))
                    .text_color(theme::text())
                    .child(SharedString::from(mini_label(&playing))),
            )
            .child(self.mini_control(
                "mini-quiet",
                if unmute {
                    Icon::VolumeOff
                } else {
                    Icon::Volume
                },
                Variant::Pill,
                if unmute { "Unmute all" } else { "Mute all" },
                cx,
                move |this, cx| this.set_quiet_all(!unmute, cx),
            ))
            .child(self.mini_control(
                "mini-expand",
                Icon::Expand,
                Variant::Pill,
                "Back to watching",
                cx,
                |this, cx| this.go_watch(cx),
            ))
            .child(self.mini_control(
                "mini-stop",
                Icon::Close,
                Variant::Destructive,
                "Stop all",
                cx,
                |this, cx| this.stop_all(cx),
            ));

        Some(
            div()
                .absolute()
                // Clear of the list's scrollbar, which runs down the page's
                // right edge; see `layout::mini_player_right`.
                .right(px(layout::mini_player_right()))
                .bottom(px(theme::GAP))
                // On the player's own box and nothing larger: the wheel still
                // reaches the list underneath, and a click on a tile or a
                // control never also opens the card beneath it.
                .block_mouse_except_scroll()
                .p(px(theme::MINI_PLAYER_INSET))
                .rounded(px(theme::RADIUS_LG))
                .bg(theme::surface_raised())
                .border_1()
                .border_color(theme::border())
                .shadow_lg()
                .child(
                    div()
                        .w(px(theme::MINI_PLAYER_WIDTH))
                        .flex()
                        .flex_col()
                        .child(tiles)
                        .child(bar),
                ),
        )
    }

    /// One stream in the player: its picture, or a word on why there is
    /// none. A click goes back to watching with this pane the one the keys
    /// talk to (`go_watch_pane`), and a close under the pointer shuts just
    /// this pane — the
    /// docked bar had a close per stream, and the player keeps it.
    fn mini_tile(&self, slot: &Slot, mini: MiniLayout, cx: &mut Context<Self>) -> impl IntoElement {
        let key = slot.key.clone();
        let close_key = slot.key.clone();
        let name = self.display_name(slot);
        let showing = watch::showing(slot);

        div()
            // By key, never by position: closing a tile moves the ones after
            // it, and a position-keyed id would hand the survivor the closed
            // tile's hover. See `watch::pane_id`.
            .id(ElementId::Name(format!("mini-tile-{key}").into()))
            .group(TILE_GROUP)
            .relative()
            .flex_none()
            .w(px(mini.tile_w))
            .h(px(mini.tile_h))
            // A flex container, so the word, when there is one, sits in the
            // middle, and the player is a flex item filling a box of definite
            // size — never a block child whose height could fall back to its
            // frame's aspect (the `img` trap in the handoff).
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .bg(theme::player_bg())
            .cursor_pointer()
            .children(slot.video().cloned())
            // The pane's own reading of itself, said in a word: a tile is a
            // few words wide, and the pane says the whole of it a click away.
            .when_some(showing.word(), |tile, word| {
                let word = div()
                    .text_size(px(theme::TEXT_META))
                    .text_color(theme::text_dim())
                    .child(word);
                tile.child(if matches!(showing, Showing::Starting { .. }) {
                    // Breathing like the pane's own starting state: a still
                    // word reads as a hang.
                    motion::waiting(ElementId::Name(format!("mini-starting-{key}").into()), word)
                        .into_any_element()
                } else {
                    word.into_any_element()
                })
            })
            .child(
                controls::icon_button(
                    ElementId::Name(format!("mini-close-{key}").into()),
                    Icon::Close,
                    // Filled, because it sits on a picture that could be any
                    // colour; the card's overlays are drawn the same way.
                    Variant::Pill,
                )
                .absolute()
                .top(px(theme::GAP_WORD))
                .right(px(theme::GAP_WORD))
                // Hidden rather than transparent, like a card's overlays: at
                // zero opacity a control still takes the click.
                .invisible()
                .group_hover(TILE_GROUP, |style| style.visible())
                .tooltip(move |window, cx| Tooltip::new(format!("Close {name}")).build(window, cx))
                .on_click(cx.listener(move |this, _event, window, cx| {
                    // Or the tile under it goes back to watching too.
                    cx.stop_propagation();
                    this.focus.focus(window);
                    // Looked up now, by key: the pane may have moved since
                    // this was drawn, and the index is what `close_slot` takes.
                    if let Some(index) = this.slot_index(&close_key) {
                        this.close_slot(index, window, cx);
                    }
                })),
            )
            .on_click(cx.listener(move |this, _event, window, cx| {
                this.focus.focus(window);
                this.go_watch_pane(key.clone(), cx);
            }))
    }

    /// One of the bar's three controls: an icon, what it does in words for
    /// the pointer that rests on it, and the method it calls.
    ///
    /// Each takes focus back for the root first. The player blocks the pointer
    /// from the root, so the root's own mouse-down never hears these presses
    /// and never takes the keys back from a text box that had them.
    fn mini_control(
        &self,
        id: &'static str,
        icon: Icon,
        variant: Variant,
        tooltip: &'static str,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        controls::icon_button(id, icon, variant)
            .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
            .on_click(cx.listener(move |this, _event, window, cx| {
                this.focus.focus(window);
                on_click(this, cx);
            }))
    }
}

/// What the bar says is playing: one name, two joined, or the first and a
/// count of the rest — then whether any of it is paused, which the tiles
/// cannot say for a picture that simply is not moving. `paused` alone when
/// all of it is, and how many when only some.
///
/// Each entry is a pane's name, as `RootView::display_name` gives it, and
/// whether it is paused.
fn mini_label(playing: &[(impl AsRef<str>, bool)]) -> String {
    let names = match playing {
        [] => String::new(),
        [(one, _)] => one.as_ref().to_string(),
        [(first, _), (second, _)] => format!("{} and {}", first.as_ref(), second.as_ref()),
        [(first, _), rest @ ..] => format!("{} and {} more", first.as_ref(), rest.len()),
    };
    let paused = playing.iter().filter(|(_, paused)| *paused).count();
    match paused {
        0 => names,
        all if all == playing.len() => format!("{names} · paused"),
        some => format!("{names} · {some} paused"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_label_names_one_and_counts_the_rest() {
        assert_eq!(mini_label(&[("Asmongold", false)]), "Asmongold");
        assert_eq!(
            mini_label(&[("Asmongold", false), ("Nubzombie", false)]),
            "Asmongold and Nubzombie"
        );
        assert_eq!(
            mini_label(&[
                ("Asmongold", false),
                ("Nubzombie", false),
                ("Kaicenat", false)
            ]),
            "Asmongold and 2 more"
        );
        assert_eq!(
            mini_label(&[("A", false), ("B", false), ("C", false), ("D", false)]),
            "A and 3 more"
        );
    }

    #[test]
    fn the_label_says_when_panes_are_paused() {
        assert_eq!(mini_label(&[("Asmongold", true)]), "Asmongold · paused");
        assert_eq!(
            mini_label(&[("Asmongold", true), ("Nubzombie", true)]),
            "Asmongold and Nubzombie · paused"
        );
        assert_eq!(
            mini_label(&[("Asmongold", false), ("Nubzombie", true)]),
            "Asmongold and Nubzombie · 1 paused"
        );
        assert_eq!(
            mini_label(&[("A", true), ("B", false), ("C", true)]),
            "A and 2 more · 2 paused"
        );
    }
}
