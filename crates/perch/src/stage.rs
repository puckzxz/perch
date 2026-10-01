//! Where each pane is drawn: in the watch grid, as a tile in the mini
//! player, or in a window of its own.
//!
//! The one owner of that answer. A pane's player is one `VideoView`, and a
//! view drawn by two windows at once freezes in the second: it skips
//! uploading a frame it already holds, so the second window would go on
//! painting the first tile it was given. So every drawer asks here before it
//! draws a player — the main window through `RootView::video_in_main`, a
//! pop-out through `RootView::pop_out_video` — and both answers come from
//! the same [`Stage`], which cannot say yes to both.
//!
//! The order of the panes is not here: that is `RootView::slots`, and
//! nothing else. A popped pane keeps its place in it, its number and its
//! chat in the main window; only its picture moves.
//!
//! Generic over what a popped pane carries, and free of gpui, in the
//! `trail` pattern: the app keeps its window and focus there
//! (`root::pop_out::PoppedOut`), and the tests keep nothing.

/// Where a pane's player is drawn, and so what it draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// In the watch grid, with its bar, its menus and its double-click.
    Pane,
    /// A tile in the mini player on the browse page: no bar, no hover, and a
    /// click that goes back to watching.
    Tile,
    /// In a window of its own, on top of everything: the bar with play,
    /// volume, Bring back and Close, and the picture to drag the window by.
    PopOut,
}

/// Which panes are drawn in windows of their own, each with what its window
/// needs, `P`. Every other pane is drawn by the main window, as a pane or a
/// tile by the page.
#[derive(Debug)]
pub struct Stage<P> {
    /// The panes popped out, by key, oldest first.
    popped: Vec<(String, P)>,
}

// Hand-written rather than derived: `derive(Default)` would demand
// `P: Default`, and an empty stage needs no such thing from its payloads.
impl<P> Default for Stage<P> {
    fn default() -> Self {
        Self { popped: Vec::new() }
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
    pub fn pop_out(&mut self, key: &str, payload: P) -> bool {
        if self.is_popped(key) {
            return false;
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

    /// Bring home every popped pane `keep` says no to — one that has closed,
    /// or stalled — and hand back what each carried, so the caller can close
    /// those windows. A pane that stops playing comes home to its main cell,
    /// where what it offers next is drawn.
    pub fn retain(&mut self, keep: impl Fn(&str) -> bool) -> Vec<(String, P)> {
        let (kept, gone) = std::mem::take(&mut self.popped)
            .into_iter()
            .partition(|(key, _)| keep(key));
        self.popped = kept;
        gone
    }

    /// Where the pane `key` names is drawn, with the watch page up or not:
    /// in its own window if it is popped, whichever page is up, and
    /// otherwise wherever the page puts it.
    pub fn place(&self, watching: bool, key: &str) -> Place {
        if self.is_popped(key) {
            Place::PopOut
        } else if watching {
            Place::Pane
        } else {
            Place::Tile
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
        let open = ["quin69"];
        let gone = stage.retain(|key| open.contains(&key));
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
        let gone = stage.retain(|_| false);
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
}
