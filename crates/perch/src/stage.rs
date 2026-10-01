//! Where each pane is drawn: in the watch grid, as a tile in the mini
//! player, in a window of its own — or nowhere, while another pane has the
//! watch page to itself.
//!
//! The one owner of that answer. A pane's player is one `VideoView`, and a
//! view drawn by two windows at once freezes in the second: it skips
//! uploading a frame it already holds, so the second window would go on
//! painting the first tile it was given. So every drawer asks here before it
//! draws a player — the main window through `RootView::video_in_main`, a
//! pop-out through `RootView::pop_out_video` — and both answers come from
//! the same [`Stage`], which cannot say yes to both.
//!
//! Which cells the watch grid draws is here too ([`Stage::cells`]): every
//! pane's, or the maximized pane's alone. The others are drawn as nothing
//! at all — no strip of names, no thumbnails — and play on as they were.
//! A strip would take height from the pane given the page, and so lower the
//! rendition it is picked; thumbnails would be the focus layout that chat
//! per pane rules out; and a pane drawn small asks its player for a small
//! frame, which would come back soft when the grid did.
//!
//! The order of the panes is not here: that is `RootView::slots`, and
//! nothing else. A popped pane keeps its place in it, its number and its
//! chat in the main window; only its picture moves. A maximized pane keeps
//! its place too, and the others theirs, so showing them all again puts
//! every pane back where it was.
//!
//! Generic over what a popped pane carries, and free of gpui, in the
//! `trail` pattern: the app keeps its window and focus there
//! (`root::pop_out::PoppedOut`), and the tests keep nothing.

/// Where a pane's player is drawn, and so what it draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// In the watch grid, with its bar, its menus and its double-click.
    Pane,
    /// On the watch page while another pane has it to itself: drawn by
    /// nothing. It plays on as it was, with its sound, rendering at the size
    /// it was last drawn at, and behaves as a tile does — no bar, no hover,
    /// no menu — until the grid comes back.
    Offstage,
    /// A tile in the mini player on the browse page: no bar, no hover, and a
    /// click that goes back to watching.
    Tile,
    /// In a window of its own, on top of everything: the bar with play,
    /// volume, Bring back and Close, and the picture to drag the window by.
    PopOut,
}

/// What a pane's maximize control offers, on its bar or folded into More,
/// and what the palette offers for it: the one answer to that question
/// ([`Stage::maximize_button`]), so the control, `Z` and the palette agree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaximizeButton {
    /// Nothing: fewer than two panes, so nothing to give the page over
    /// from, or a pane in a window of its own, which is not on the page.
    Hidden,
    /// Give this pane the watch page.
    Maximize,
    /// This pane has it: show every pane again.
    Restore,
}

/// Which panes are drawn in windows of their own, each with what its window
/// needs, `P`, and which pane has the watch page to itself, if one does.
/// Every other pane is drawn by the main window, as a pane or a tile by the
/// page, or by nothing while another is maximized.
#[derive(Debug)]
pub struct Stage<P> {
    /// The panes popped out, by key, oldest first.
    popped: Vec<(String, P)>,
    /// The pane given the whole watch page, chat and all, by key: the only
    /// cell the grid draws ([`cells`](Self::cells)). Never a popped pane,
    /// and never with fewer than two panes; see
    /// [`retain`](Self::retain). For the session only.
    maximized: Option<String>,
}

// Hand-written rather than derived: `derive(Default)` would demand
// `P: Default`, and an empty stage needs no such thing from its payloads.
impl<P> Default for Stage<P> {
    fn default() -> Self {
        Self {
            popped: Vec::new(),
            maximized: None,
        }
    }
}

impl<P> Stage<P> {
    /// Whether the pane `key` names is drawn in a window of its own.
    pub fn is_popped(&self, key: &str) -> bool {
        self.popped(key).is_some()
    }

    /// What the popped pane `key` names carries, or `None` for a pane the
    /// main window draws.
    pub fn popped(&self, key: &str) -> Option<&P> {
        self.popped
            .iter()
            .find(|(popped, _)| popped == key)
            .map(|(_, payload)| payload)
    }

    /// The same, to change: the window a pop-out was opened in arrives
    /// after the pane has moved there.
    pub fn popped_mut(&mut self, key: &str) -> Option<&mut P> {
        self.popped
            .iter_mut()
            .find(|(popped, _)| popped == key)
            .map(|(_, payload)| payload)
    }

    /// Every popped pane, oldest first, with what it carries.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &P)> {
        self.popped
            .iter()
            .map(|(key, payload)| (key.as_str(), payload))
    }

    /// Move the pane `key` names into a window of its own. Refused for a
    /// pane already in one, which keeps its window: two windows for one
    /// player is exactly what this type is here to prevent.
    ///
    /// A maximized pane popped out gives the watch page back: it is not on
    /// the page to fill it, and the page would otherwise draw nothing but
    /// its cell's `Playing in its own window`.
    pub fn pop_out(&mut self, key: &str, payload: P) -> bool {
        if self.is_popped(key) {
            return false;
        }
        if self.maximized.as_deref() == Some(key) {
            self.maximized = None;
        }
        self.popped.push((key.to_string(), payload));
        true
    }

    /// Bring the pane `key` names back to the main window, handing back what
    /// it carried so the caller can close its window. `None` for a pane
    /// that was not out.
    pub fn pop_in(&mut self, key: &str) -> Option<P> {
        let index = self.popped.iter().position(|(popped, _)| popped == key)?;
        Some(self.popped.remove(index).1)
    }

    /// Hold the stage to the panes there are now, `panes`, by key, in their
    /// order: bring home every popped pane that has gone, or that `keep`
    /// says no to — one that has stalled — and hand back what each carried,
    /// so the caller can close those windows. A pane that stops playing
    /// comes home to its main cell, where what it offers next is drawn.
    ///
    /// The maximize ends here too, once its pane has gone — closed, or
    /// replaced without a [`rename`](Self::rename) — or fewer than two panes
    /// are left: a lone pane has the page already. And when a popped pane
    /// comes home rather than going: its main cell is where what it offers
    /// next is drawn, and with another pane maximized that cell is drawn
    /// nowhere — its window would close with no sign of where it went. A
    /// popped pane closed takes nothing with it.
    pub fn retain(&mut self, panes: &[&str], keep: impl Fn(&str) -> bool) -> Vec<(String, P)> {
        let (kept, gone): (Vec<_>, Vec<_>) = std::mem::take(&mut self.popped)
            .into_iter()
            .partition(|(key, _)| panes.contains(&key.as_str()) && keep(key));
        self.popped = kept;
        let homecoming = gone.iter().any(|(key, _)| panes.contains(&key.as_str()));
        if homecoming
            || panes.len() < 2
            || self
                .maximized
                .as_deref()
                .is_some_and(|maximized| !panes.contains(&maximized))
        {
            self.maximized = None;
        }
        gone
    }

    /// Where the pane `key` names is drawn, with the watch page up or not:
    /// in its own window if it is popped, whichever page is up; otherwise a
    /// tile off the watch page; and on it, nowhere while another pane is
    /// maximized, and in the grid the rest of the time.
    pub fn place(&self, watching: bool, key: &str) -> Place {
        if self.is_popped(key) {
            Place::PopOut
        } else if !watching {
            Place::Tile
        } else if self
            .maximized
            .as_deref()
            .is_some_and(|maximized| maximized != key)
        {
            Place::Offstage
        } else {
            Place::Pane
        }
    }

    /// The cells the watch grid draws, as positions in `panes`, in order:
    /// the maximized pane alone, or every pane, a popped one's cell — its
    /// chat and its `Bring back` — included.
    pub fn cells(&self, panes: &[&str]) -> Vec<usize> {
        let maximized = self
            .maximized
            .as_deref()
            .and_then(|maximized| panes.iter().position(|pane| *pane == maximized));
        match maximized {
            Some(index) => vec![index],
            None => (0..panes.len()).collect(),
        }
    }

    /// The pane with the watch page to itself, if one has.
    pub fn maximized(&self) -> Option<&str> {
        self.maximized.as_deref()
    }

    /// What the maximize control of the pane `key` names offers, among
    /// `panes`: to show every pane again on the pane that has the page, to
    /// give it the page on any other pane on it, and nothing with fewer than
    /// two panes or for a pane in a window of its own.
    pub fn maximize_button(&self, key: &str, panes: &[&str]) -> MaximizeButton {
        if self.maximized.as_deref() == Some(key) {
            MaximizeButton::Restore
        } else if panes.len() < 2 || !panes.contains(&key) || self.is_popped(key) {
            MaximizeButton::Hidden
        } else {
            MaximizeButton::Maximize
        }
    }

    /// Do what the maximize control of the pane `key` names offers
    /// ([`maximize_button`](Self::maximize_button)): give it the page, or
    /// show every pane again. Returns whether anything changed; a hidden
    /// control changes nothing.
    pub fn toggle_maximize(&mut self, key: &str, panes: &[&str]) -> bool {
        match self.maximize_button(key, panes) {
            MaximizeButton::Hidden => false,
            MaximizeButton::Maximize => {
                self.maximized = Some(key.to_string());
                true
            }
            MaximizeButton::Restore => self.unmaximize(),
        }
    }

    /// Show every pane again. Returns whether one had the page.
    pub fn unmaximize(&mut self) -> bool {
        self.maximized.take().is_some()
    }

    /// The pane `key` names was chosen — by its number, `Tab`, a tile, the
    /// palette: while another pane is maximized, it takes the maximize, so
    /// the pane the keys talk to is always the one on the page. Not a popped
    /// pane, which is on screen already, in a window of its own. Returns
    /// whether the maximize moved.
    pub fn choose(&mut self, key: &str) -> bool {
        let moves = self
            .maximized
            .as_deref()
            .is_some_and(|maximized| maximized != key)
            && !self.is_popped(key);
        if moves {
            self.maximized = Some(key.to_string());
        }
        moves
    }

    /// The pane `old` named is `new` now: a recording swapped in for a live
    /// pane, in place (`RootView::replace_with_video`). A maximize goes with
    /// it, rather than ending when [`retain`](Self::retain) no longer finds
    /// `old`. A popped pane is not renamed: its window looks its player up by
    /// key, so it is brought home before it is replaced.
    pub fn rename(&mut self, old: &str, new: &str) {
        if self.maximized.as_deref() == Some(old) {
            self.maximized = Some(new.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stage with these panes popped, each carrying its own name.
    fn popped(keys: &[&str]) -> Stage<String> {
        let mut stage = Stage::default();
        for key in keys {
            assert!(stage.pop_out(key, format!("window of {key}")));
        }
        stage
    }

    /// Three panes, in their order.
    const THREE: [&str; 3] = ["forsen", "quin69", "xqc"];

    /// A stage with `key` maximized among [`THREE`].
    fn maximized(key: &str) -> Stage<()> {
        let mut stage = Stage::default();
        assert!(stage.toggle_maximize(key, &THREE));
        stage
    }

    #[test]
    fn a_popped_pane_is_drawn_in_its_own_window() {
        let stage = popped(&["forsen"]);
        assert!(stage.is_popped("forsen"));
        assert_eq!(
            stage.popped("forsen").map(String::as_str),
            Some("window of forsen")
        );
        assert!(!stage.is_popped("quin69"));
        assert_eq!(stage.popped("quin69"), None);
    }

    /// One window per pane: a second pop-out of the same pane is refused,
    /// and the first keeps what it carried.
    #[test]
    fn popping_out_twice_is_refused() {
        let mut stage = popped(&["forsen"]);
        assert!(!stage.pop_out("forsen", "a second window".to_string()));
        assert_eq!(
            stage.popped("forsen").map(String::as_str),
            Some("window of forsen")
        );
        assert_eq!(stage.iter().count(), 1);
    }

    #[test]
    fn popping_in_hands_back_its_window() {
        let mut stage = popped(&["forsen", "quin69"]);
        assert_eq!(stage.pop_in("forsen").as_deref(), Some("window of forsen"));
        assert!(!stage.is_popped("forsen"));
        assert!(
            stage.is_popped("quin69"),
            "only the one asked for came home"
        );
        assert_eq!(stage.pop_in("forsen"), None, "nothing is out twice");
    }

    /// A pane closed while popped hands back its window, so the caller can
    /// close it too.
    #[test]
    fn a_closed_pane_hands_back_its_window() {
        let mut stage = popped(&["forsen", "quin69"]);
        let gone = stage.retain(&["quin69"], |_| true);
        assert_eq!(
            gone,
            vec![("forsen".to_string(), "window of forsen".to_string())]
        );
        assert!(!stage.is_popped("forsen"));
        assert!(stage.is_popped("quin69"));
    }

    /// A pane whose stream stalls comes home the same way: the caller's
    /// `keep` says no, and the window comes back with it.
    #[test]
    fn a_stalled_pane_comes_home() {
        let mut stage = popped(&["forsen"]);
        let gone = stage.retain(&["forsen"], |_| false);
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].1, "window of forsen");
        assert_eq!(stage.place(true, "forsen"), Place::Pane);
    }

    /// The page decides between a pane and a tile; a popped pane is in its
    /// window on either page.
    #[test]
    fn places_follow_the_page_unless_popped() {
        let stage = popped(&["forsen"]);
        assert_eq!(stage.place(true, "quin69"), Place::Pane);
        assert_eq!(stage.place(false, "quin69"), Place::Tile);
        for watching in [true, false] {
            assert_eq!(stage.place(watching, "forsen"), Place::PopOut);
        }
    }

    /// A lone pane has the page already, so there is nothing to maximize
    /// it over; with two, either can be.
    #[test]
    fn maximize_needs_two_panes() {
        let mut stage: Stage<()> = Stage::default();
        assert!(!stage.toggle_maximize("forsen", &["forsen"]));
        assert_eq!(stage.maximized(), None);
        assert!(!stage.toggle_maximize("forsen", &[]), "no panes at all");
        assert!(stage.toggle_maximize("forsen", &["forsen", "quin69"]));
        assert_eq!(stage.maximized(), Some("forsen"));
    }

    /// The grid draws the maximized pane's cell and nothing else, and every
    /// cell again once it is shown all.
    #[test]
    fn a_maximized_pane_is_the_only_cell() {
        let mut stage = maximized("quin69");
        assert_eq!(stage.cells(&THREE), vec![1]);
        assert!(stage.toggle_maximize("quin69", &THREE), "a second press");
        assert_eq!(stage.cells(&THREE), vec![0, 1, 2]);
        assert!(stage.toggle_maximize("xqc", &THREE));
        assert!(stage.unmaximize());
        assert!(!stage.unmaximize(), "nothing left to show all of");
        assert_eq!(stage.cells(&THREE), vec![0, 1, 2]);
    }

    /// The others are on the watch page but drawn nowhere; off the page,
    /// every pane is a tile in the mini player, the maximized one included.
    #[test]
    fn the_others_wait_offstage() {
        let stage = maximized("quin69");
        assert_eq!(stage.place(true, "quin69"), Place::Pane);
        assert_eq!(stage.place(true, "forsen"), Place::Offstage);
        assert_eq!(stage.place(true, "xqc"), Place::Offstage);
        for key in THREE {
            assert_eq!(stage.place(false, key), Place::Tile, "{key} browsing");
        }
    }

    /// A pane chosen while another is maximized takes the maximize, so the
    /// pane the keys talk to is the one on the page. Choosing the pane that
    /// has it, or choosing with nothing maximized, moves nothing.
    #[test]
    fn choosing_a_pane_moves_the_maximize() {
        let mut stage = maximized("forsen");
        assert!(stage.choose("xqc"));
        assert_eq!(stage.maximized(), Some("xqc"));
        assert_eq!(stage.cells(&THREE), vec![2]);
        assert!(!stage.choose("xqc"), "it has it already");

        let mut stage: Stage<()> = Stage::default();
        assert!(!stage.choose("xqc"));
        assert_eq!(stage.maximized(), None, "choosing never maximizes");
    }

    /// A pane in a window of its own is not on the page to fill it: it is
    /// not given the page, and choosing it leaves the maximize where it is.
    #[test]
    fn a_popped_pane_cannot_be_maximized() {
        let mut stage: Stage<()> = Stage::default();
        assert!(stage.pop_out("quin69", ()));
        assert_eq!(
            stage.maximize_button("quin69", &THREE),
            MaximizeButton::Hidden
        );
        assert!(!stage.toggle_maximize("quin69", &THREE));
        assert!(stage.toggle_maximize("forsen", &THREE));
        assert!(!stage.choose("quin69"));
        assert_eq!(stage.maximized(), Some("forsen"));
    }

    /// Popping out the maximized pane gives the page back to every pane.
    #[test]
    fn popping_out_the_maximized_pane_ends_it() {
        let mut stage = maximized("quin69");
        assert!(stage.pop_out("quin69", ()));
        assert_eq!(stage.maximized(), None);
        assert_eq!(stage.cells(&THREE), vec![0, 1, 2]);
        assert_eq!(stage.place(true, "forsen"), Place::Pane);

        let mut stage = maximized("quin69");
        assert!(stage.pop_out("forsen", ()));
        assert_eq!(
            stage.maximized(),
            Some("quin69"),
            "another pane popped out leaves the maximize"
        );
    }

    /// The maximized pane closing shows the rest; another closing does not.
    #[test]
    fn closing_the_maximized_pane_shows_the_rest() {
        let mut stage = maximized("forsen");
        stage.retain(&["forsen", "quin69"], |_| true);
        assert_eq!(stage.maximized(), Some("forsen"), "xqc closed");
        let left = ["quin69", "xqc"];
        stage.retain(&left, |_| true);
        assert_eq!(stage.maximized(), None);
        assert_eq!(stage.cells(&left), vec![0, 1]);
    }

    /// A popped pane whose stream stalls while another is maximized comes
    /// home to every pane shown, so its cell, and what it offers next, is on
    /// the page; one closed while popped leaves the maximize where it is.
    #[test]
    fn a_stalled_pane_coming_home_shows_every_pane() {
        let mut stage: Stage<String> = Stage::default();
        assert!(stage.pop_out("xqc", "window of xqc".to_string()));
        assert!(stage.toggle_maximize("forsen", &THREE));
        let gone = stage.retain(&THREE, |key| key != "xqc");
        assert_eq!(gone.len(), 1, "its window comes back to be closed");
        assert_eq!(stage.maximized(), None);
        assert_eq!(stage.place(true, "xqc"), Place::Pane);
        assert_eq!(stage.cells(&THREE), vec![0, 1, 2]);

        let mut stage: Stage<String> = Stage::default();
        assert!(stage.pop_out("xqc", "window of xqc".to_string()));
        assert!(stage.toggle_maximize("forsen", &THREE));
        let gone = stage.retain(&["forsen", "quin69"], |_| true);
        assert_eq!(gone.len(), 1);
        assert_eq!(
            stage.maximized(),
            Some("forsen"),
            "a popped pane closed is not coming home"
        );
    }

    /// Down to one pane, nothing is maximized: the lone pane has the page
    /// already, and a pane added later must not open offstage.
    #[test]
    fn one_pane_left_ends_the_maximize() {
        let mut stage = maximized("forsen");
        stage.retain(&["forsen"], |_| true);
        assert_eq!(stage.maximized(), None);
        assert_eq!(stage.place(true, "quin69"), Place::Pane);
    }

    /// A recording swapped in for the maximized live pane keeps the page,
    /// under its own key.
    #[test]
    fn a_renamed_pane_keeps_its_maximize() {
        let mut stage = maximized("forsen");
        stage.rename("forsen", "vod:2868644730");
        let panes = ["vod:2868644730", "quin69", "xqc"];
        stage.retain(&panes, |_| true);
        assert_eq!(stage.maximized(), Some("vod:2868644730"));
        assert_eq!(stage.cells(&panes), vec![0]);

        stage.rename("quin69", "vod:1");
        assert_eq!(
            stage.maximized(),
            Some("vod:2868644730"),
            "another pane renamed takes nothing"
        );
    }

    /// The control offers what a press would do: nothing for a lone pane or
    /// one popped out, the page for a pane on it, and the grid back for the
    /// pane that has the page.
    #[test]
    fn the_button_follows_the_stage() {
        let mut stage: Stage<()> = Stage::default();
        assert_eq!(
            stage.maximize_button("forsen", &["forsen"]),
            MaximizeButton::Hidden
        );
        for key in THREE {
            assert_eq!(stage.maximize_button(key, &THREE), MaximizeButton::Maximize);
        }
        assert!(stage.toggle_maximize("quin69", &THREE));
        assert_eq!(
            stage.maximize_button("quin69", &THREE),
            MaximizeButton::Restore
        );
        assert_eq!(
            stage.maximize_button("forsen", &THREE),
            MaximizeButton::Maximize,
            "a pane offstage would be given the page"
        );
        assert!(stage.pop_out("xqc", ()));
        assert_eq!(stage.maximize_button("xqc", &THREE), MaximizeButton::Hidden);
    }
}
