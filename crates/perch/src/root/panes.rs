//! Where each pane is drawn, applied: the one funnel every change of a
//! pane's state, of which panes there are, of the page and of what is popped
//! out goes through ([`restage`](RootView::restage)), and the one way the
//! main window reaches a pane's player
//! ([`video_in_main`](RootView::video_in_main)).
//!
//! The answer itself is `crate::stage`'s. This is the root holding it to
//! what the slots are now: a popped pane that has closed or stopped comes
//! home, and every player is told where it is drawn and which window's focus
//! takes its keys back — and no player is told by anything else
//! (`VideoView::set_place`), so the two cannot disagree.

use gpui::{App, Context, Entity, FocusHandle};

use super::{Page, RootView};
use crate::motion;
use crate::stage::Place;
use crate::video_view::VideoView;
use crate::watch::{self, Showing, Slot, StreamState};

impl RootView {
    /// Where the pane `key` names is drawn: its own window if it is popped,
    /// and otherwise the watch grid or a mini-player tile, by the page.
    pub(super) fn place_of(&self, key: &str) -> Place {
        self.stage.place(self.page == Page::Watch, key)
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
    /// what it offers next is drawn. Then each player hears its place and
    /// its window's focus, and each pane off the screen starts its header
    /// over, hidden. Every change of which panes there are, of their states,
    /// of the page or of what is popped ends here.
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
    pub(super) fn restage(&mut self, cx: &mut Context<Self>) {
        let slots = &self.slots;
        let gone = self.stage.retain(|key| {
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

    /// Tell every player where it is drawn, and in which window's focus; see
    /// `VideoView::set_place`, which does nothing for a player already told.
    ///
    /// Off the watch page, each pane's header over the picture also starts
    /// over, hidden, as the player does its bar: the browse page never draws
    /// it, and a fade that came back after frames without its element would
    /// replay its last flip on the way back (see `motion::Fade::apply`). A
    /// reveal still running goes with it. Where the pointer was is left
    /// alone: it decides which pane takes the keys on the way back, and
    /// `go_watch_pane` sets it for that. A popped pane's header is still
    /// drawn on the watch page, over its cell, and is left as it is there.
    fn sync_presentation(&mut self, cx: &mut Context<Self>) {
        let watching = self.page == Page::Watch;
        for slot in &mut self.slots {
            let place = self.stage.place(watching, &slot.key);
            if let Some(view) = slot.video() {
                let focus = self
                    .stage
                    .popped(&slot.key)
                    .map_or_else(|| self.focus.clone(), |popped| popped.focus.clone());
                view.update(cx, |video, cx| video.set_place(place, focus, cx));
            }
            if !watching {
                slot.header = motion::Fade::hidden();
                slot.revealed = false;
            }
        }
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
