//! What each pane plays, and when it restarts: the rendition a pane's stream
//! is resolved at, chosen against that pane's own height
//! ([`pane_height_for`](RootView::pane_height_for)); the re-pick when the
//! grid changes, which only ever moves upwards
//! ([`sync_quality`](RootView::sync_quality)); a pick from the pane's own
//! menu ([`request_quality`](RootView::request_quality)); and the restart
//! each of them ends in ([`restart_stream`](RootView::restart_stream)).
//!
//! Named for renditions rather than for quality: `streamlink::quality` is
//! the module every question here is put to, and goes by that name.

use gpui::{Context, Window};
use settings::QualityPreference;
use streamlink::quality;

use super::RootView;
use crate::layout;
use crate::watch::{Slot, StreamState};

impl RootView {
    /// Start pane `index`'s stream over, from where a recording has got to.
    /// For a quality change from anywhere: the pane's menu, the settings
    /// sheet, or the pane changing size.
    pub(super) fn restart_stream(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(slot) = self.slots.get_mut(index) else {
            return;
        };
        // A recording picks up where it was, not from the top. Read before
        // the state changes, since that is what drops the player.
        if let Some(position) = slot.video().map(|view| view.read(cx).position()) {
            slot.resume_at = position;
        }
        let key = slot.key.clone();
        self.set_slot_state(index, StreamState::Starting, cx);
        self.start_stream(key, window, cx);
    }

    /// What the settings would have a pane of `pane_height` play, from the
    /// renditions its stream offers. The one answer to that question, for
    /// the re-pick after a resize and for a pane handed back to the default.
    fn settings_pick(&self, available: &[String], pane_height: u32) -> Option<quality::Quality> {
        match &self.settings.quality {
            QualityPreference::Auto => quality::select(available, pane_height),
            QualityPreference::Fixed(name) => quality::select_named(available, name, pane_height),
        }
    }

    /// A quality chosen from pane `index`'s own menu: a rendition, which holds
    /// until the pane closes, or `None`, which hands the pane back to the
    /// settings.
    ///
    /// The stream restarts only when that changes what plays — a restart is
    /// seconds of black, and pinning the rendition already playing, or going
    /// back to a default that picks the same one, should cost nothing. Going
    /// back re-picks for the pane as it is now, down as well as up: that is
    /// an answer somebody asked for, where the re-pick after a resize is not.
    pub(super) fn request_quality(
        &mut self,
        index: usize,
        name: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.slots.get(index).and_then(Slot::video).cloned() else {
            return;
        };
        let (playing, available) = {
            let view = view.read(cx);
            (view.quality().to_string(), view.available().to_vec())
        };
        let target = match &name {
            Some(name) => Some(name.clone()),
            None => {
                let pane_height = self.pane_height_for(&self.slots[index].key, window);
                self.settings_pick(&available, pane_height)
                    .map(|pick| pick.name)
            }
        };
        let picked = name.is_some();
        self.slots[index].quality_override = name;
        if target.as_deref() == Some(playing.as_str()) {
            // Nothing to restart, so the menu the pane already has says it.
            view.update(cx, |view, cx| view.set_picked(picked, cx));
        } else {
            self.restart_stream(index, window, cx);
        }
        cx.notify();
    }

    /// How tall the pane `key` names is right now, in physical pixels: what
    /// its quality is chosen against, when its stream starts and again in
    /// [`sync_quality`](Self::sync_quality). With several panes the window is
    /// shared, so each one asks for proportionally less.
    ///
    /// Its cell of the watch grid, the one grid ([`grid`](Self::grid)), on
    /// whichever page is up: a mini-player tile is not one of the sizes a
    /// quality is chosen for. The cell's share of the body's height, seams
    /// included (`Grid::share_height`, which says why). Asked by the pane's
    /// key although every pane is one cell of that grid today — a pane in a
    /// window of its own included, since it keeps its cell — so that a pane
    /// measured by room of its own is asked the same question, and every
    /// caller already says which pane it means.
    pub(super) fn pane_height_for(&self, _key: &str, window: &Window) -> u32 {
        layout::quality_height(self.grid(window).share_height * window.scale_factor())
    }

    /// Choose each pane's quality again, for the size it is now — and move
    /// only upwards.
    ///
    /// A quality is picked when a stream opens, against the pane it will
    /// render into — and the pane changes size every time another opens or
    /// closes, the rail folds, or the window does. A pane opened as one of
    /// four stayed at the small rendition after the other three closed,
    /// which was a soft picture in a large pane for as long as nobody touched
    /// the menu. So the choice is made again whenever the grid changes, and
    /// a *sharper* answer restarts the stream: a restart is a few seconds of
    /// black while streamlink resolves again, worth it for the picture. A
    /// pane that has shrunk keeps what it has — the smaller pane hides
    /// nothing, the CPU it costs is what it cost when it opened, and a
    /// restart there would trade a visible interruption for a saving. A
    /// quality picked by hand from the pane's own menu is left alone; that
    /// choice was about this pane, whatever its size. So, for now, is a pane
    /// in a window of its own, whose window is not the grid's cell this
    /// measures.
    ///
    /// Each pane is measured on its own ([`pane_height_for`](Self::pane_height_for)),
    /// and its log line says the height it was measured at.
    pub(super) fn sync_quality(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let restart: Vec<(usize, u32)> = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.quality_override.is_none())
            .filter(|(_, slot)| !self.stage.is_popped(&slot.key))
            .filter_map(|(index, slot)| {
                let pane_height = self.pane_height_for(&slot.key, window);
                let view = slot.video()?.read(cx);
                let wanted = self.settings_pick(view.available(), pane_height)?;
                let playing = quality::parse_quality(view.quality()).map_or(0, |q| q.height);
                (wanted.height > playing).then_some((index, pane_height))
            })
            .collect();
        for (index, pane_height) in &restart {
            eprintln!(
                "video: {} re-choosing quality for a {pane_height}px pane",
                self.slots[*index].key
            );
            self.restart_stream(*index, window, cx);
        }
        if !restart.is_empty() {
            cx.notify();
        }
    }
}
