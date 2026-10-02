//! Where each pane is drawn, applied: the one funnel every change of a
//! pane's state, of which panes there are, of the page, of what is popped
//! out and of which pane is maximized goes through
//! ([`restage`](RootView::restage)), and the one way the main window reaches
//! a pane's player ([`video_in_main`](RootView::video_in_main)).
//!
//! The answer itself is `crate::stage`'s. This is the root holding it to
//! what the slots are now: a popped pane that has closed or stopped comes
//! home, a maximize ends once its pane has gone or a popped pane comes home
//! to a cell it would hide, and every player is told where it is drawn,
//! which window's focus takes its keys back and what its maximize control
//! offers — and no player is told by anything else (`VideoView::set_place`,
//! `VideoView::set_maximize`), so the two cannot disagree.
//!
//! And the maximize itself, as the root drives it: giving a pane the watch
//! page and showing every pane again
//! ([`toggle_maximize`](RootView::toggle_maximize),
//! [`show_all_panes`](RootView::show_all_panes)), and choosing a pane
//! ([`choose`](RootView::choose)), which takes the maximize with it. And two
//! panes swapping places in the order ([`move_pane`](RootView::move_pane)),
//! which is the slots' alone and changes where nothing is drawn.

use gpui::{App, Context, Entity, FocusHandle, Window};

use super::{Page, RootView};
use crate::motion;
use crate::stage::{MaximizeButton, Place};
use crate::video_view::VideoView;
use crate::watch::{self, Showing, Slot, StreamState};

/// Every pane's key, in the panes' order: what `crate::stage` is asked about
/// the panes there are. A free function over the slots rather than a method,
/// so it borrows them alone and the stage beside them can still be changed.
pub(super) fn pane_keys(slots: &[Slot]) -> Vec<&str> {
    slots.iter().map(|slot| slot.key.as_str()).collect()
}

impl RootView {
    /// Where the pane `key` names is drawn: its own window if it is popped,
    /// and otherwise the watch grid or a mini-player tile, by the page — or
    /// nowhere, on the watch page while another pane is maximized.
    pub(super) fn place_of(&self, key: &str) -> Place {
        self.stage.place(self.page == Page::Watch, key)
    }

    /// What the maximize control of the pane `key` names offers; see
    /// `stage::MaximizeButton`. For a player being made (`Start::maximize`),
    /// which `restage` then keeps told.
    pub(super) fn maximize_button_of(&self, key: &str) -> MaximizeButton {
        self.stage.maximize_button(key, &pane_keys(&self.slots))
    }

    /// The focus of the window the pane `key` names is drawn in: its
    /// pop-out's own, or the root's in the main window.
    pub(super) fn focus_of(&self, key: &str) -> FocusHandle {
        self.stage
            .popped(key)
            .map_or_else(|| self.focus.clone(), |popped| popped.focus.clone())
    }

    /// The player the main window draws for `slot`: none while the pane
    /// plays in a window of its own. The only way anything the main window
    /// draws — the watch page's panes, the mini player's tiles, the band's
    /// hover — reaches a player.
    ///
    /// None rather than the player for a popped pane for two reasons. A view
    /// drawn by two windows freezes in one of them (`crate::stage`); and a
    /// window that so much as reads an entity while drawing is redrawn every
    /// time it changes, which for a player is every frame — the main window
    /// would redraw at the video's rate for a picture it does not show.
    pub(super) fn video_in_main<'a>(&self, slot: &'a Slot) -> Option<&'a Entity<VideoView>> {
        if self.stage.is_popped(&slot.key) {
            return None;
        }
        slot.video()
    }

    /// What `slot` is showing in the main window: `Elsewhere` while its
    /// picture is in a window of its own, and otherwise its own reading,
    /// with the picture counted as covering the pane only once its first
    /// frame has faded all the way in (`VideoView::covers`). Until then the
    /// pane is still starting, and what it was waiting for stays drawn under
    /// the fading picture — so for what goes over the top of the pane it is
    /// a pane with nothing to cover, like one stopped.
    pub(super) fn showing_in_main<'a>(&self, slot: &'a Slot, cx: &App) -> Showing<'a> {
        let covered = self
            .video_in_main(slot)
            .is_some_and(|view| view.read(cx).covers());
        watch::showing(slot, covered, self.stage.is_popped(&slot.key))
    }

    /// Move the pane at `index` to `state`, and see to everything that
    /// follows from it. The only way a slot's state is written: a pane that
    /// stalls while popped has to come home, and a new player has to be
    /// told where it is drawn, and both are [`restage`](Self::restage)'s.
    /// A test holds the root to this.
    pub(super) fn set_slot_state(
        &mut self,
        index: usize,
        state: StreamState,
        cx: &mut Context<Self>,
    ) {
        self.slots[index].set_state(state);
        self.restage(cx);
    }

    /// Hold the stage to the slots, then tell every player where it is drawn.
    ///
    /// A popped pane that has closed, or stopped — offline, ended or failed
    /// — comes home: its window closes, deferred, and its main cell is where
    /// what it offers next is drawn. A maximize ends once its pane has gone,
    /// a lone pane is left, or a popped pane comes home (`Stage::retain`).
    /// Then each player hears its place, its window's focus and its maximize
    /// control, and each pane off the screen starts its header over, hidden,
    /// and lets its chat go. Every change of which panes there are, of their
    /// states, of the page, of what is popped or of what is maximized ends
    /// here.
    ///
    /// And so does a pane coming home to a main window that draws none
    /// there: off the watch page with the mini player off. It is stopped,
    /// as it would have been had it been home when the page was left
    /// ([`retire_homeless`](Self::retire_homeless)) — kept, a player would
    /// play its sound with no window drawing it, and a stopped live pane
    /// could start again by itself where nobody sees it. Deferred, because
    /// whoever called this may go on to use the index of the pane it moved,
    /// and stopping a pane takes it out of `slots`. Bring back never gets
    /// here that way: it brings the watch page up first (`pop_in`).
    ///
    /// A maximize that ends here rather than through `maximize_changed`
    /// brings back panes `sync_quality` left alone while they were put away,
    /// whose cells may have grown meanwhile — a resize, a pane closed — so
    /// each pane's quality is chosen again once this is done, through the
    /// main window, which this is called without; after any pane stopped
    /// above, so none is re-picked on its way out. A caller that chooses
    /// again itself (`close_slot`) costs nothing more: a second look finds
    /// the rendition already resolving (`wants_swap`).
    ///
    /// A header being dragged whose pane has gone is let go of here too, so
    /// no pane goes on offering itself to a drag of nothing.
    pub(super) fn restage(&mut self, cx: &mut Context<Self>) {
        if self
            .pane_move
            .as_deref()
            .is_some_and(|key| self.slot_index(key).is_none())
        {
            self.pane_move = None;
        }
        let was_maximized = self.stage.maximized().is_some();
        let slots = &self.slots;
        let gone = self.stage.retain(&pane_keys(slots), |key| {
            slots
                .iter()
                .any(|slot| slot.key == key && !stalled(&slot.state))
        });
        for (_, popped) in gone {
            self.let_pop_out_go(popped, cx);
        }
        if self.homeless() {
            let root = cx.entity().downgrade();
            cx.defer(move |cx| {
                let _ = root.update(cx, |this, cx| this.retire_homeless(cx));
            });
        }
        if was_maximized && self.stage.maximized().is_none() {
            let root = cx.entity().downgrade();
            let main = self.main_window;
            cx.defer(move |cx| {
                let _ = main.update(cx, |_, window, cx| {
                    let _ = root.update(cx, |this, cx| this.sync_quality(window, cx));
                });
            });
        }
        self.sync_presentation(cx);
        cx.notify();
    }

    /// Whether the main window draws the panes it holds at home, those not
    /// in windows of their own, on the page it is on: the watch page does,
    /// and the browse page does through the mini player, while that is on
    /// (`mini_player_shows`).
    pub(super) fn draws_home_panes(&self) -> bool {
        draws_home_panes(self.page == Page::Watch, self.settings.miniplayer)
    }

    /// Whether a pane is at home in the main window with nothing there to
    /// draw it; see [`draws_home_panes`](Self::draws_home_panes).
    fn homeless(&self) -> bool {
        !self.draws_home_panes()
            && self
                .slots
                .iter()
                .any(|slot| !self.stage.is_popped(&slot.key))
    }

    /// Stop every pane the main window holds at home with nothing there to
    /// draw it, and leave the rest: the one rule for leaving the watch page
    /// with the mini player off (`go_browse`), for turning it off while
    /// browsing (the settings sheet), and for a pane that comes home to
    /// either (`restage`), so the three cannot drift apart. A pane in a
    /// window of its own plays on: it is on screen, and was put there to be
    /// kept.
    pub(super) fn retire_homeless(&mut self, cx: &mut Context<Self>) {
        if !self.homeless() {
            return;
        }
        let popped: Vec<String> = self.stage.iter().map(|(key, _)| key.to_string()).collect();
        self.retire_slots(move |slot| popped.contains(&slot.key), cx);
    }

    /// Tell every player where it is drawn, in which window's focus, and
    /// what its maximize control offers; see `VideoView::set_place`, which
    /// does nothing for a player already told, and `VideoView::set_maximize`,
    /// the control's mirror, which nothing else writes after `Start`.
    ///
    /// Each pane whose cell is off the screen — every one off the watch
    /// page, and on it every one but the maximized pane's while one is — also
    /// has its header over the picture start over, hidden, as the player
    /// does its bar: nothing draws it there, and a fade that came back after
    /// frames without its element would replay its last flip on the way
    /// back (see `motion::Fade::apply`). A reveal still running goes with
    /// it. And its chat lets go of the rows it was holding back for a
    /// pointer over it (`ChatView::let_go_hold`), which only its drawing
    /// would otherwise do, so they cannot pile up for as long as it is
    /// away. A popped pane's cell is still drawn on the watch page, its
    /// header and chat with it, and is left as it is there while every pane
    /// is shown; with another pane maximized its cell is off the screen too,
    /// and goes the same way.
    ///
    /// And on the watch page, each pane whose cell is off the screen counts
    /// as pointed at already (`Slot::hovered`), as on the way back from a
    /// mini-player tile (`go_watch_pane`). Its probe is not drawn to keep
    /// that true or false, and the grid comes back under a pointer that has
    /// not moved however a maximize ends — `Z`, `Esc`, the maximized pane
    /// popped out or closed, a popped pane coming home — so the rising edge
    /// of whichever pane is now under it would take the keys from the pane
    /// they were talking to. Off the watch page it is left alone: the way
    /// back from a tile sets it for that.
    fn sync_presentation(&mut self, cx: &mut Context<Self>) {
        let watching = self.page == Page::Watch;
        let keys = pane_keys(&self.slots);
        let cells = self.stage.cells(&keys);
        let told: Vec<(Place, MaximizeButton, bool)> = keys
            .iter()
            .enumerate()
            .map(|(index, key)| {
                (
                    self.stage.place(watching, key),
                    self.stage.maximize_button(key, &keys),
                    watching && cells.contains(&index),
                )
            })
            .collect();
        for (slot, (place, button, on_screen)) in self.slots.iter_mut().zip(told) {
            if let Some(view) = slot.video() {
                let focus = self
                    .stage
                    .popped(&slot.key)
                    .map_or_else(|| self.focus.clone(), |popped| popped.focus.clone());
                view.update(cx, |video, cx| {
                    video.set_place(place, focus, cx);
                    video.set_maximize(button, cx);
                });
            }
            if !on_screen {
                if watching {
                    slot.hovered = true;
                }
                slot.header = motion::Fade::hidden();
                slot.revealed = false;
                if let Some(chat) = &slot.chat {
                    chat.update(cx, |chat, cx| chat.let_go_hold(cx));
                }
            }
        }
        // Which panes there are decides whether More offers hearing one
        // alone at all.
        self.sync_hear_only(cx);
    }

    /// Give the pane `key` names the whole watch page, chat and all, or, on
    /// the pane that has it, show every pane again: `Z` on the active pane,
    /// its bar's control or More's row, and the palette's `Maximize …`. What
    /// a press does is what the control offers (`Stage::toggle_maximize`),
    /// so nothing for a lone pane or one in a window of its own.
    ///
    /// The other panes are drawn as nothing while it lasts, and play on as
    /// they were, with their sound; see `crate::stage` for why nothing.
    pub(super) fn toggle_maximize(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.stage.toggle_maximize(key, &pane_keys(&self.slots)) {
            self.maximize_changed(window, cx);
        }
    }

    /// Show every pane again, if one has the watch page: `Esc`, on its way
    /// off the page, and the palette's `Show all panes`. Returns whether one
    /// had, so `Esc` stops there.
    pub(super) fn show_all_panes(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.stage.unmaximize() {
            return false;
        }
        self.maximize_changed(window, cx);
        true
    }

    /// Make the pane `key` names the one the keys talk to, by choice: its
    /// number, `Tab`, its mini-player tile, the palette's `Choose quality
    /// for …` row, bringing it back from its window, or opening what it
    /// already plays. While a pane is maximized the chosen one takes the
    /// maximize (`Stage::choose`), so `Space`, `M` and `Ctrl+W` always act
    /// on the pane on the page — or on a popped one, in its own window —
    /// never on one drawn nowhere. A pointer coming into a pane, or a press
    /// on one, is no choice to make here: only a pane on the page can be
    /// pointed at.
    pub(super) fn choose(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.active = Some(key.to_string());
        if self.stage.choose(key) {
            self.maximize_changed(window, cx);
        }
        cx.notify();
    }

    /// The pane with the watch page changed, or none has it any more: the
    /// keys follow the pane given it, and every player hears where it is
    /// drawn now (`restage`), each pane put away letting go of what it was
    /// doing. Each of those counts as pointed at already until it is drawn
    /// again (`sync_presentation`), so the grid coming back under a still
    /// pointer gives the keys to nobody.
    ///
    /// Then each pane's quality is chosen again: the maximized pane is
    /// measured by the whole body now (`pane_height_for`), and moves up in
    /// place if a sharper rendition fits; one shown in the grid again has
    /// shrunk, which never changes what plays, and the others come back to
    /// cells that may have grown while they were away.
    fn maximize_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = self.stage.maximized().map(str::to_string) {
            self.active = Some(key);
        }
        self.restage(cx);
        self.sync_quality(window, cx);
    }

    /// Swap the pane `from` names with the one `onto` names, in the order:
    /// `from`'s header dropped on `onto`, or `Shift+←`/`Shift+→` with `from`
    /// the active pane and `onto` its neighbour. A swap rather than an
    /// insert — the two trade places and every other pane stays where it
    /// was, which with four panes at most is the move people make. Whatever
    /// counts the order follows at once: the grid, `1`–`4` and `Tab`, the
    /// mini player and the palette all read `slots`. For the session only;
    /// nothing of it is saved.
    ///
    /// `from` is the active pane afterwards, the pane just handled, and its
    /// header comes up where it went (`reveal_header`). Every pane counts
    /// as pointed at already, as on the way back from a mini-player tile
    /// (`go_watch_pane`): another pane is under the pointer now without it
    /// having moved, and its rising edge would take the keys from the pane
    /// just moved.
    ///
    /// Nothing is restarted or told anything. Every cell in the grid is as
    /// tall as every other, and a pane's quality is chosen by its height
    /// (`pane_height_for`), so no pane's rendition changes with its place;
    /// nor does where it is drawn, nor its maximize control, which are
    /// `restage`'s and read no order. And nothing is remounted, since every
    /// id in a pane is its key's (`watch::page`).
    pub(super) fn move_pane(&mut self, from: &str, onto: &str, cx: &mut Context<Self>) {
        self.pane_move = None;
        let (Some(from_index), Some(onto_index)) = (self.slot_index(from), self.slot_index(onto))
        else {
            cx.notify();
            return;
        };
        self.slots.swap(from_index, onto_index);
        self.active = Some(from.to_string());
        for slot in &mut self.slots {
            slot.hovered = true;
        }
        self.reveal_header(from, cx);
        cx.notify();
    }
}

/// Whether the main window draws a pane at home, `watching` or not, with the
/// mini player on or off: only the browse page with the mini player off
/// draws none, so only there must a pane be in a window of its own or stop.
fn draws_home_panes(watching: bool, miniplayer: bool) -> bool {
    watching || miniplayer
}

/// Whether a pane in `state` has nothing playing and nothing on its way: what
/// brings a popped pane home.
fn stalled(state: &StreamState) -> bool {
    match state {
        StreamState::Offline | StreamState::Ended | StreamState::Failed(_) => true,
        StreamState::Starting | StreamState::Playing(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every write of a slot's state goes through `set_slot_state`, so none
    /// can leave a popped pane out after it stalled, or a new player not
    /// knowing where it is drawn. Read from the root's own sources, every
    /// file there is, so a new one is held to it without being listed.
    #[test]
    fn every_change_of_state_goes_through_set_slot_state() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/root");
        let mut writes = 0;
        for entry in std::fs::read_dir(&dir).expect("the root's sources are missing") {
            let path = entry.expect("an unreadable source").path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("an unreadable source");
            let code = source
                .split("#[cfg(test)]")
                .next()
                .expect("a file has code above its tests");
            // Spaces and line ends out, so formatting cannot hide a call.
            let code: String = code.split_whitespace().collect();
            let found = code.matches(".set_state(").count();
            let name = path.file_name().and_then(|name| name.to_str());
            if name == Some("panes.rs") {
                writes += found;
            } else {
                assert_eq!(
                    found,
                    0,
                    "{} writes a slot's state itself; use set_slot_state",
                    path.display()
                );
            }
        }
        assert_eq!(writes, 1, "set_slot_state is the one write");
    }

    /// What the main window draws reaches a player only through
    /// `video_in_main`, which answers none for a popped pane, so a drawer
    /// cannot read a player another window draws, which would redraw the
    /// main window at the video's rate. The mini player and the pages are
    /// the root's drawers; read from their sources, as the test above reads
    /// the root's.
    #[test]
    fn the_main_windows_drawers_reach_players_only_through_video_in_main() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/root");
        for name in ["mini_player.rs", "pages.rs"] {
            let source = std::fs::read_to_string(dir.join(name)).expect("an unreadable source");
            let code: String = source
                .split("#[cfg(test)]")
                .next()
                .expect("a file has code above its tests")
                .split_whitespace()
                .collect();
            assert!(
                !code.contains(".video()"),
                "{name} reaches a player through Slot::video; use video_in_main"
            );
        }
    }

    /// A pane at home is drawn by the watch page, or by the mini player
    /// while browsing; browsing with it off, nothing draws one.
    #[test]
    fn only_browsing_with_the_mini_player_off_draws_no_pane_at_home() {
        assert!(draws_home_panes(true, false));
        assert!(draws_home_panes(true, true));
        assert!(draws_home_panes(false, true));
        assert!(!draws_home_panes(false, false));
    }

    /// Leaving the page, turning the mini player off and a pane coming home
    /// all stop a pane by the one rule, `retire_homeless`: none of them
    /// closes panes by a rule of its own that could come to keep a pane the
    /// others would stop, or the other way round. Read from the sources, as
    /// the test above reads them.
    #[test]
    fn panes_with_nowhere_to_be_drawn_stop_by_one_rule() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/root");
        // Spaces and line ends out, as above.
        let code = |name: &str| {
            let source = std::fs::read_to_string(dir.join(name)).expect("an unreadable source");
            source
                .split("#[cfg(test)]")
                .next()
                .expect("a file has code above its tests")
                .split_whitespace()
                .collect::<String>()
        };
        let streams = code("streams.rs");
        let go_browse = streams
            .split("fngo_browse(")
            .nth(1)
            .and_then(|rest| rest.split("pub(super)fn").next())
            .expect("go_browse is in streams.rs");
        assert!(go_browse.contains("retire_homeless(cx)"));
        assert!(!go_browse.contains("retire_slots("));
        assert!(code("prefs.rs").contains("retire_homeless(cx)"));
    }

    /// A pane comes home when there is nothing to play, and only then: a
    /// restart passes through starting on its way back to playing.
    #[test]
    fn only_a_stopped_pane_comes_home() {
        assert!(stalled(&StreamState::Offline));
        assert!(stalled(&StreamState::Ended));
        assert!(stalled(&StreamState::Failed("gone".into())));
        assert!(!stalled(&StreamState::Starting));
    }
}
