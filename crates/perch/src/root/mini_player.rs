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
//!
//! Where the pop-out is offered, each tile can pop its pane out into a
//! window of its own, and the bar can pop out every pane it shows, each into
//! its own window — the whole mini player, out on top of whatever else is
//! on screen, one pane per window. A pane out is not shown here (it is on
//! screen already), and the player goes once none is left.

use gpui::{div, prelude::*, px, Context, ElementId, IntoElement, SharedString, Window};

use super::{pop_out, Page, RootView};
use crate::assets::Icon;
use crate::controls::{self, Variant};
use crate::layout::{self, MiniLayout};
use crate::stage::Stage;
use crate::watch::{Showing, Slot};
use crate::{loudness, motion, theme};

/// The group a tile's controls watch for the pointer, so they show only
/// while the pointer is on that tile.
const TILE_GROUP: &str = "mini-tile";

impl RootView {
    /// Whether the mini player is up: on the browse page, with the miniplayer
    /// on, and something playing here rather than in a window of its own.
    ///
    /// Whenever the main window holds a pane at home — any of
    /// [`mini_slots`](Self::mini_slots) — this is exactly the complement of
    /// the watch page, so a player is never drawn twice in one frame, nor
    /// left with nothing drawing it: browsing with the mini player off, no
    /// pane stays at home (`RootView::retire_homeless`). One answer, read by
    /// the player itself and by the room the browse lists leave for it.
    pub(super) fn mini_player_shows(&self) -> bool {
        self.page == Page::Browse && self.settings.miniplayer && !self.mini_slots().is_empty()
    }

    /// The panes the mini player shows, in their order: every one but those
    /// in windows of their own, which are on screen already. What its tiles,
    /// its shape, its label and the room the browse lists leave for it are
    /// all counted from.
    pub(super) fn mini_slots(&self) -> Vec<&Slot> {
        at_home(&self.slots, |slot| &slot.key, &self.stage)
    }

    /// Whether the mini player has anything to pop out: it is up, the
    /// pop-out is offered here, and a pane it shows has a player to move
    /// (`RootView::pop_out`). A pane it shows starting, off, ended or failed
    /// has none, and stays at home.
    ///
    /// One answer for its bar's `Pop out`, which shows only while this
    /// holds, and for `P` on the browse page, which pops them out while it
    /// holds and otherwise brings every popped pane back
    /// (`on_toggle_pop_out`). Asked apart, the key went by whether the
    /// player was up at all, and with only a pane that had ended left in
    /// it, `P` there popped nothing out and brought nothing back.
    pub(super) fn mini_pop_out_offered(&self) -> bool {
        pop_out::offered_here()
            && self.mini_player_shows()
            && self
                .mini_slots()
                .iter()
                .any(|slot| self.video_in_main(slot).is_some())
    }

    /// The player, bottom-right of the page, or nothing; see
    /// [`mini_player_shows`](Self::mini_player_shows).
    pub(super) fn mini_player(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if !self.mini_player_shows() {
            return None;
        }
        let shown = self.mini_slots();
        let mini = layout::mini_player(shown.len());

        let mut tiles = div().flex().flex_col().gap(px(theme::PANE_GAP));
        for row in shown.chunks(mini.cols) {
            let mut line = div().flex().flex_row().gap(px(theme::PANE_GAP));
            for slot in row {
                line = line.child(self.mini_tile(slot, mini, cx));
            }
            tiles = tiles.child(line);
        }

        let playing: Vec<(String, bool)> = shown
            .iter()
            .map(|slot| {
                let paused = self
                    .video_in_main(slot)
                    .is_some_and(|view| view.read(cx).is_paused());
                (self.display_name(slot), paused)
            })
            .collect();
        let unmute = loudness::unmute_all_offered(self.slots.iter().map(|slot| slot.quiet));
        // Its words follow what a press would do, so its id does too: a
        // tooltip comes up with the words of its moment and keeps them, and
        // without this one that came up over Mute all went on saying so after
        // the press, until the pointer left. See `controls::tip`.
        let (quiet_id, quiet_icon, quiet_words) = if unmute {
            ("mini-unmute-all", Icon::VolumeOff, "Unmute all")
        } else {
            ("mini-mute-all", Icon::Volume, "Mute all")
        };

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
                quiet_id,
                quiet_icon,
                Variant::Pill,
                quiet_words,
                cx,
                move |this, _window, cx| this.set_quiet_all(!unmute, cx),
            ))
            // Every pane here out into windows of their own, where that is
            // offered and a pane here has a player to move.
            .when(self.mini_pop_out_offered(), |bar| {
                bar.child(self.mini_control(
                    "mini-pop-out",
                    Icon::PopOut,
                    Variant::Pill,
                    "Pop out",
                    cx,
                    |this, window, cx| {
                        // The player goes, or shrinks, from under the pointer
                        // with them, and a card or a tile there would take a
                        // double-click's second press; see `run_guard`.
                        this.take_rest_of_run();
                        this.pop_out_all_shown(window, cx);
                    },
                ))
            })
            .child(self.mini_control(
                "mini-expand",
                Icon::Expand,
                Variant::Pill,
                "Back to watching",
                cx,
                |this, _window, cx| this.go_watch(cx),
            ))
            .child(self.mini_control(
                "mini-stop",
                Icon::Close,
                Variant::Destructive,
                "Stop all",
                cx,
                |this, _window, cx| this.stop_all(cx),
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
    /// this pane — the docked bar had a close per stream, and the player
    /// keeps it. Beside the close, where the pop-out is offered and the pane
    /// has a player to move, the way out into a window of its own.
    ///
    /// The word is the pane's own reading of itself (`watch::Showing`), so a
    /// player whose first frame has not faded in yet says `Starting…`, as the
    /// pane does, where it used to say a word of its own.
    fn mini_tile(&self, slot: &Slot, mini: MiniLayout, cx: &mut Context<Self>) -> impl IntoElement {
        let key = slot.key.clone();
        let close_key = slot.key.clone();
        let name = self.display_name(slot);
        let showing = self.showing_in_main(slot, cx);
        let pop = (pop_out::offered_here() && self.video_in_main(slot).is_some()).then(|| {
            let pop_key = slot.key.clone();
            controls::icon_button(
                ElementId::Name(format!("mini-pop-out-{key}").into()),
                Icon::PopOut,
                Variant::Pill,
            )
            .tooltip(controls::tip(format!("Pop out {name}")))
            .on_click(cx.listener(move |this, _event, window, cx| {
                // Or the tile under it goes back to watching too.
                cx.stop_propagation();
                this.focus.focus(window);
                // The tile goes from under the pointer, and the tile or the
                // card that comes there would take a double-click's second
                // press; see `run_guard`.
                this.take_rest_of_run();
                this.pop_out(&pop_key, window, cx);
            }))
        });

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
            // A flex container, so the player is a flex item filling a box of
            // definite size — never a block child whose height could fall
            // back to its frame's aspect (the `img` trap in the handoff).
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .bg(theme::player_bg())
            .cursor_pointer()
            // The pane's own reading of itself, said in a word: a tile is a
            // few words wide, and the pane says the whole of it a click away.
            // Under the player, which draws nothing until its first frame
            // and then fades that in over the word; and out of the flow, so
            // the two never share the tile's width.
            .when_some(showing.word(), |tile, word| {
                let word = div()
                    .text_size(px(theme::TEXT_META))
                    .text_color(theme::text_dim())
                    .child(word);
                let word = if matches!(showing, Showing::Starting { .. }) {
                    // Breathing like the pane's own starting state: a still
                    // word reads as a hang.
                    motion::waiting(ElementId::Name(format!("mini-starting-{key}").into()), word)
                        .into_any_element()
                } else {
                    word.into_any_element()
                };
                tile.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(word),
                )
            })
            .children(self.video_in_main(slot).cloned())
            .child(
                // The tile's controls, top-right: out into a window of its
                // own, then the close. Filled, because they sit on a picture
                // that could be any colour; the card's overlays are drawn the
                // same way.
                div()
                    .absolute()
                    .top(px(theme::GAP_WORD))
                    .right(px(theme::GAP_WORD))
                    .flex()
                    .flex_row()
                    .gap(px(theme::GAP_TIGHT))
                    // Hidden rather than transparent, like a card's overlays:
                    // at zero opacity a control still takes the click. A
                    // hidden element paints none of its children, nor their
                    // listeners, so this hides both.
                    .invisible()
                    .group_hover(TILE_GROUP, |style| style.visible())
                    .children(pop)
                    .child(
                        controls::icon_button(
                            ElementId::Name(format!("mini-close-{key}").into()),
                            Icon::Close,
                            Variant::Pill,
                        )
                        .tooltip(controls::tip(format!("Close {name}")))
                        .on_click(cx.listener(
                            move |this, _event, window, cx| {
                                // Or the tile under it goes back to watching too.
                                cx.stop_propagation();
                                this.focus.focus(window);
                                // Looked up now, by key: the pane may have
                                // moved since this was drawn, and the index
                                // is what `close_slot` takes.
                                if let Some(index) = this.slot_index(&close_key) {
                                    this.close_slot(index, window, cx);
                                }
                            },
                        )),
                    ),
            )
            .on_click(cx.listener(move |this, _event, window, cx| {
                this.focus.focus(window);
                this.go_watch_pane(key.clone(), cx);
            }))
    }

    /// One of the bar's controls: an icon, what it does in words for the
    /// pointer that rests on it, and the method it calls, with the main
    /// window, which `Pop out` opens its windows against. A control whose
    /// words change with what it would do passes an `id` that changes with
    /// them; see `controls::tip`.
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
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        controls::icon_button(id, icon, variant)
            .tooltip(controls::tip(tooltip))
            .on_click(cx.listener(move |this, _event, window, cx| {
                this.focus.focus(window);
                on_click(this, window, cx);
            }))
    }
}

/// The panes the mini player shows, from `panes` in their order, each named
/// by its `key`: every one the `stage` keeps at home, since a pane in a
/// window of its own is on screen already. What `RootView::mini_slots`
/// answers, and so what the tiles, the player's shape, its label and the
/// room the browse lists leave for it are counted from.
fn at_home<'a, T, P>(panes: &'a [T], key: impl Fn(&T) -> &str, stage: &Stage<P>) -> Vec<&'a T> {
    panes
        .iter()
        .filter(|pane| !stage.is_popped(key(pane)))
        .collect()
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

    /// A pane popped out into a window of its own is not in the mini player,
    /// so the label neither names it nor counts it as paused: it says what
    /// is still here.
    #[test]
    fn the_label_counts_only_panes_still_here() {
        let panes = [
            ("Asmongold", false),
            ("Nubzombie", true),
            ("Kaicenat", false),
        ];
        let mut stage = Stage::default();
        assert!(stage.pop_out("Nubzombie", ()));
        let here = at_home(&panes, |pane| pane.0, &stage);
        assert_eq!(here, [&panes[0], &panes[2]]);
        let label = mini_label(&here.into_iter().copied().collect::<Vec<_>>());
        assert_eq!(label, "Asmongold and Kaicenat");

        assert!(stage.pop_out("Asmongold", ()));
        assert!(stage.pop_out("Kaicenat", ()));
        assert!(
            at_home(&panes, |pane| pane.0, &stage).is_empty(),
            "nothing left for the player to show"
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
