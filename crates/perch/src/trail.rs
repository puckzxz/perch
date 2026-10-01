//! Back and forward: the places the app has been, and the places it went
//! back from.
//!
//! Generic, and free of gpui, because the rules are the whole of it and they
//! are easy to get subtly wrong: what a visit does to the way forward, which
//! places a step skips, and what happens to a place that has stopped existing.
//! What a place *is* belongs to the caller — the root's `navigation::Route` —
//! and so does whether one can still be reached. A trail holds no notion of
//! "here": the caller says where it is at every step, so the trail can never
//! become a second account of what the window shows.

/// How many steps either way the trail keeps. Past it, the oldest go. Fifty
/// is more than anybody walks back through, and few enough that a session
/// left open for days does not grow a list nobody will reach the end of.
pub const LIMIT: usize = 50;

/// Where the app has been, behind it and ahead of it.
#[derive(Debug)]
pub struct Trail<R> {
    /// The places behind, oldest first: `back` steps to the last of them.
    back: Vec<R>,
    /// The places stepped back from, furthest first: `forward` steps to the
    /// last of them.
    forward: Vec<R>,
}

// Hand-written rather than derived: `derive(Default)` would demand
// `R: Default`, and an empty trail needs no such thing from its places.
impl<R> Default for Trail<R> {
    fn default() -> Self {
        Self {
            back: Vec::new(),
            forward: Vec::new(),
        }
    }
}

impl<R: Clone + PartialEq> Trail<R> {
    /// The app just left `left` for somewhere new. It goes on the way back,
    /// once — the same place twice in a row is one step — and the way forward
    /// is gone, as it is in every browser: going somewhere new is a new road.
    pub fn visit(&mut self, left: R) {
        push(&mut self.back, left);
        self.forward.clear();
    }

    /// The place one step back from `here`, if there is one, with `here` put
    /// on the way forward. Places that are no longer `valid`, or that are
    /// `here` already, are passed over and dropped on the way: a step that
    /// arrived where you stand would be a press that did nothing.
    ///
    /// With nowhere to go, nothing changes.
    pub fn back(&mut self, here: &R, valid: impl Fn(&R) -> bool) -> Option<R> {
        step(&mut self.back, &mut self.forward, here, valid)
    }

    /// The mirror of [`back`](Self::back).
    pub fn forward(&mut self, here: &R, valid: impl Fn(&R) -> bool) -> Option<R> {
        step(&mut self.forward, &mut self.back, here, valid)
    }

    /// Whether [`back`](Self::back) would go anywhere, without going.
    pub fn can_back(&self, here: &R, valid: impl Fn(&R) -> bool) -> bool {
        self.back.iter().any(|route| valid(route) && route != here)
    }

    /// Whether [`forward`](Self::forward) would go anywhere, without going.
    pub fn can_forward(&self, here: &R, valid: impl Fn(&R) -> bool) -> bool {
        self.forward
            .iter()
            .any(|route| valid(route) && route != here)
    }

    /// Take every place `gone` names off the trail, both ways, for good.
    ///
    /// What was either side of a forgotten place closes up, and two equal
    /// places that end up side by side become one: `A, Watch, A` with the
    /// watch page gone is `A`, not two steps back to where you already were.
    pub fn forget(&mut self, gone: impl Fn(&R) -> bool) {
        for stack in [&mut self.back, &mut self.forward] {
            stack.retain(|route| !gone(route));
            stack.dedup();
        }
    }

    /// Nowhere behind and nowhere ahead.
    pub fn clear(&mut self) {
        self.back.clear();
        self.forward.clear();
    }
}

/// Put `route` on top of `stack`, unless it is on top already, and let the
/// oldest go past [`LIMIT`].
fn push<R: PartialEq>(stack: &mut Vec<R>, route: R) {
    if stack.last() == Some(&route) {
        return;
    }
    stack.push(route);
    if stack.len() > LIMIT {
        stack.remove(0);
    }
}

/// One step from `from` towards `to`: the nearest place on `from` that is
/// `valid` and not `here`, everything above it dropped, and `here` pushed
/// onto `to`. Found before anything moves, so a step with nowhere to go
/// leaves both stacks as they were.
fn step<R: Clone + PartialEq>(
    from: &mut Vec<R>,
    to: &mut Vec<R>,
    here: &R,
    valid: impl Fn(&R) -> bool,
) -> Option<R> {
    let at = from
        .iter()
        .rposition(|route| valid(route) && route != here)?;
    let target = from[at].clone();
    from.truncate(at);
    push(to, here.clone());
    Some(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn always(_: &&str) -> bool {
        true
    }

    /// A trail walked through `places` in order, standing on the last.
    fn walked(places: &[&'static str]) -> (Trail<&'static str>, &'static str) {
        let mut trail = Trail::default();
        for pair in places.windows(2) {
            trail.visit(pair[0]);
        }
        (trail, places[places.len() - 1])
    }

    #[test]
    fn visiting_clears_the_way_forward() {
        let (mut trail, here) = walked(&["a", "b", "c"]);
        assert_eq!(trail.back(&here, always), Some("b"));
        assert!(trail.can_forward(&"b", always));

        // From b somewhere new: c is no longer ahead.
        trail.visit("b");
        assert!(!trail.can_forward(&"d", always));
        assert_eq!(trail.forward(&"d", always), None);
    }

    /// Elsewhere than any place a test puts on the trail. Asked from the
    /// place a step landed on, `can_back` passes over every copy of it, so a
    /// duplicate left behind would never show; asked from here, it does.
    const ELSEWHERE: &str = "elsewhere";

    #[test]
    fn the_same_place_twice_is_one_step() {
        let mut trail = Trail::default();
        trail.visit("a");
        trail.visit("a");
        assert_eq!(trail.back(&"b", always), Some("a"));
        assert!(!trail.can_back(&ELSEWHERE, always), "a was kept twice");
    }

    #[test]
    fn back_then_forward_returns_to_where_you_were() {
        let (mut trail, here) = walked(&["a", "b", "c"]);
        let back = trail.back(&here, always).unwrap();
        assert_eq!(back, "b");
        let back_again = trail.back(&back, always).unwrap();
        assert_eq!(back_again, "a");
        assert!(!trail.can_back(&back_again, always));

        assert_eq!(trail.forward(&back_again, always), Some("b"));
        assert_eq!(trail.forward(&"b", always), Some("c"));
        assert!(!trail.can_forward(&"c", always));
        assert_eq!(trail.back(&"c", always), Some("b"), "the way back is whole");
    }

    #[test]
    fn back_skips_places_that_are_gone() {
        let (mut trail, here) = walked(&["a", "watch", "b"]);
        let reachable = |route: &&str| *route != "watch";
        assert!(trail.can_back(&here, reachable));
        assert_eq!(trail.back(&here, reachable), Some("a"));
        // The skipped place went with the step; b waits ahead.
        assert_eq!(trail.forward(&"a", reachable), Some("b"));
    }

    #[test]
    fn a_place_equal_to_here_is_skipped() {
        // Where `here` changed under the trail without a step — the last pane
        // closing drops the watch page back onto the tab it left.
        let mut trail = Trail::default();
        trail.visit("a");
        trail.visit("b");
        assert_eq!(trail.back(&"b", always), Some("a"));
    }

    #[test]
    fn forgetting_the_watch_page_closes_the_gap() {
        let (mut trail, here) = walked(&["a", "watch", "a", "b"]);
        trail.forget(|route| *route == "watch");
        assert_eq!(trail.back(&here, always), Some("a"));
        assert!(
            !trail.can_back(&ELSEWHERE, always),
            "a, watch, a should have closed up to one a"
        );
    }

    #[test]
    fn the_trail_forgets_its_oldest_past_the_limit() {
        let mut trail = Trail::default();
        for step in 0..LIMIT + 10 {
            trail.visit(step);
        }
        let mut here = LIMIT + 10;
        let mut steps = 0;
        while let Some(back) = trail.back(&here, |_| true) {
            here = back;
            steps += 1;
        }
        assert_eq!(steps, LIMIT);
        assert_eq!(here, 10, "the oldest ten should have gone");
    }

    #[test]
    fn back_with_nowhere_to_go_changes_nothing() {
        let mut trail: Trail<&str> = Trail::default();
        assert_eq!(trail.back(&"a", always), None);
        assert!(!trail.can_forward(&"a", always), "a step went nowhere");

        // Only places that cannot be reached: they stay for when they can.
        trail.visit("watch");
        let reachable = |route: &&str| *route != "watch";
        assert_eq!(trail.back(&"a", reachable), None);
        assert!(!trail.can_forward(&"a", always));
        assert_eq!(trail.back(&"a", always), Some("watch"));
    }
}
