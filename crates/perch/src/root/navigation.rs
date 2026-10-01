//! Back and forward: where the app is, as a place the trail can hold
//! ([`Route`]), the trail itself (`crate::trail`), and the three ways along
//! it — the title bar's arrows, `Alt+←` and `Alt+→`, and the mouse's side
//! buttons.
//!
//! The trail is history and nothing more. What the browse page shows is still
//! `Discovery` and what the window shows is still `page`; a route is *read*
//! from those two, never kept beside them, so there is no second account of
//! the view to fall out of step with the first. Steps are recorded around the
//! functions that already move the view — opening a channel, a tab, a search —
//! by [`RootView::record`], so every way into those (a card, the rail, the
//! palette, a toast, a launch, `Esc`, a context bar's back) is recorded
//! without any of them knowing. Going back replays a place through the same
//! functions, with recording held off, so walking the trail does not write
//! it.

use gpui::{
    canvas, prelude::*, Context, DispatchPhase, MouseButton, MouseDownEvent, NavigationDirection,
    Window,
};
use twitch_api::Category;

use super::{Page, RootView};
use crate::browse::{Action, Discovery, Place, Tab};

/// A place the app can be, as the trail remembers it: the watch page, or what
/// the browse page was showing.
///
/// Carries what it takes to go back there — a category's name for its bar, a
/// channel's display name and id for its page — but is the same place as
/// another by identity alone; see the `PartialEq`.
#[derive(Clone, Debug)]
pub(super) enum Route {
    Watch,
    Tab(Tab),
    Category(Category),
    Search(String),
    Channel {
        login: String,
        display_name: String,
        user_id: Option<String>,
    },
}

/// Written out rather than derived, so a place is *which* tab, category,
/// search or channel and nothing else. A channel's id arrives in the reply
/// to its page when the page was opened by name alone; derived, that fill-in
/// would make the page a different place from itself, and back would land
/// on the page you are already on.
impl PartialEq for Route {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Route::Watch, Route::Watch) => true,
            (Route::Tab(a), Route::Tab(b)) => a == b,
            (Route::Category(a), Route::Category(b)) => a.id == b.id,
            (Route::Search(a), Route::Search(b)) => a == b,
            (Route::Channel { login: a, .. }, Route::Channel { login: b, .. }) => a == b,
            _ => false,
        }
    }
}

/// Where the app is, read from the two things that decide it.
fn route_of(page: Page, discovery: &Discovery) -> Route {
    if page == Page::Watch {
        return Route::Watch;
    }
    match discovery.place() {
        Place::Tab(tab) => Route::Tab(tab),
        Place::Category(category) => Route::Category(category.clone()),
        Place::Search(results) => Route::Search(results.query.to_string()),
        Place::Channel(page) => Route::Channel {
            login: page.login.clone(),
            display_name: page.display_name.clone(),
            user_id: page.user_id.clone(),
        },
    }
}

/// Whether a place can still be gone to: the watch page only while something
/// is open on it. The one answer for stepping, for the arrows' state, and for
/// whether a place is worth recording at all.
fn reachable(route: &Route, playing: bool) -> bool {
    *route != Route::Watch || playing
}

impl RootView {
    /// Where the app is now.
    pub(super) fn here(&self) -> Route {
        route_of(self.page, &self.discovery)
    }

    /// Run `f`, and if it moved the app somewhere else, put where it was on
    /// the trail.
    ///
    /// Wrapped around every function that moves the view, which nest: a tab
    /// from the watch page is `go_to_tab`, then `go_browse`, then `show_tab`.
    /// Only the outermost records, so that is one step. Needs no `Window`,
    /// because the search box runs its search from a subscription that has
    /// none.
    ///
    /// A watch page left with nothing on it — the mini player off, so leaving
    /// stopped every stream — is not recorded: it is somewhere back could
    /// never go.
    pub(super) fn record<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        if self.recording {
            return f(self);
        }
        self.recording = true;
        let before = self.here();
        let out = f(self);
        self.recording = false;
        if self.here() != before && reachable(&before, !self.slots.is_empty()) {
            self.trail.visit(before);
        }
        out
    }

    /// Whether back has anywhere to go, for the title bar's arrow.
    pub(super) fn can_back(&self) -> bool {
        let playing = !self.slots.is_empty();
        self.trail
            .can_back(&self.here(), |route| reachable(route, playing))
    }

    /// Whether forward has anywhere to go, for the title bar's arrow.
    pub(super) fn can_forward(&self) -> bool {
        let playing = !self.slots.is_empty();
        self.trail
            .can_forward(&self.here(), |route| reachable(route, playing))
    }

    /// One step back along the trail, if there is one. Says whether there
    /// was: the side buttons only claim a press that went somewhere.
    pub(super) fn go_back(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let here = self.here();
        let playing = !self.slots.is_empty();
        match self.trail.back(&here, |route| reachable(route, playing)) {
            Some(route) => {
                self.show_route(route, window, cx);
                true
            }
            None => false,
        }
    }

    /// One step forward along the trail, if there is one; the mirror of
    /// [`go_back`](Self::go_back).
    pub(super) fn go_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let here = self.here();
        let playing = !self.slots.is_empty();
        match self.trail.forward(&here, |route| reachable(route, playing)) {
            Some(route) => {
                self.show_route(route, window, cx);
                true
            }
            None => false,
        }
    }

    /// Go to `route` the way the app gets anywhere — through the functions
    /// that open a tab, a category, a search or a channel's page — with the
    /// recording held off, since the trail has already taken the step.
    ///
    /// Only what differs is done. Back from the watch page to the list that
    /// opened it is the common case, and the list is still there: leaving
    /// the watch page shows it without asking Twitch again. A takeover whose
    /// list has been replaced since is asked for again, and starts at the
    /// top.
    fn show_route(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
        let was = std::mem::replace(&mut self.recording, true);
        match route {
            Route::Watch => self.go_watch(cx),
            Route::Tab(tab) => self.go_to_tab(tab, window, cx),
            takeover => {
                if self.page == Page::Watch {
                    self.go_browse(cx);
                }
                if self.here() != takeover {
                    self.open_takeover(takeover, window, cx);
                }
            }
        }
        self.recording = was;
        cx.notify();
    }

    /// A category, a search or a channel's page, opened again the way it was
    /// opened the first time.
    fn open_takeover(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
        match route {
            Route::Category(category) => {
                self.on_browse_action(Action::OpenCategory(category), window, cx)
            }
            Route::Search(query) => {
                // The box says what the results are for, as it does when the
                // search is typed there.
                self.search
                    .update(cx, |state, cx| state.set_value(query.clone(), window, cx));
                self.run_search(query, cx);
            }
            Route::Channel {
                login,
                display_name,
                user_id,
            } => self.open_channel_page(login, display_name, user_id, window, cx),
            // Not takeovers; `show_route` goes to these itself.
            Route::Watch | Route::Tab(_) => {}
        }
    }

    /// The mouse's back and forward buttons, as an element for the root to
    /// hold: a `canvas` that listens at the window as it paints, the way
    /// `drag_listeners` does.
    ///
    /// Window-level because nothing narrower hears every press. The root's
    /// own mouse handlers answer only while the pointer is over the root's
    /// unblocked hitbox, and that is false over the title bar, a toast, the
    /// mini player and a pane's control bar — exactly where a hand resting
    /// on the mouse is likely to be. In the capture phase, and the press is
    /// stopped there once it has moved the app, so nothing under the pointer
    /// takes it as well. A press with nowhere to go — the trail's either end
    /// — is left alone and lands like any other click would. Over the page
    /// that includes the root's own mouse-down taking the keyboard back from
    /// a text box, as a left click there does; only over the box itself, the
    /// title bar or an overlay that blocks the pointer does the box keep the
    /// cursor. On the press rather than the release: a Mac's swipe sends the
    /// press alone.
    ///
    /// With the settings sheet or the palette up the buttons do nothing, as
    /// the keys and the title bar's arrows do nothing: nothing navigates
    /// behind a modal.
    ///
    /// On Windows a side button over the title bar's empty strip or a caption
    /// button is a non-client press, which gpui never passes on, so there it
    /// does nothing; see `HANDOFF.md`, "Known limits".
    pub(super) fn side_buttons(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.entity().downgrade();
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let owner = owner.clone();
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let MouseButton::Navigate(direction) = event.button else {
                        return;
                    };
                    let moved = owner
                        .update(cx, |this, cx| {
                            if this.modal_open() {
                                return false;
                            }
                            let moved = match direction {
                                NavigationDirection::Back => this.go_back(window, cx),
                                NavigationDirection::Forward => this.go_forward(window, cx),
                            };
                            if moved {
                                // The press stops here, so the root's own
                                // mouse-down never hears it to take the keys
                                // back from a text box; taken back by hand.
                                this.focus.focus(window);
                            }
                            moved
                        })
                        .unwrap_or(false);
                    if moved {
                        cx.stop_propagation();
                    }
                });
            },
        )
        .absolute()
        .size_full()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(login: &str, user_id: Option<&str>) -> Route {
        Route::Channel {
            login: login.into(),
            display_name: login.into(),
            user_id: user_id.map(Into::into),
        }
    }

    fn category(id: &str, name: &str) -> Route {
        Route::Category(Category {
            id: id.into(),
            name: name.into(),
            box_art_url: String::new(),
        })
    }

    /// A channel's page opened by name, then given its id by the reply, is
    /// still the page you are on — or back would step to it from itself.
    #[test]
    fn a_filled_in_user_id_is_the_same_place() {
        assert_eq!(channel("someone", None), channel("someone", Some("42")));
        assert_ne!(channel("someone", None), channel("someone_else", None));
    }

    #[test]
    fn a_category_is_its_id() {
        assert_eq!(
            category("509658", "Just Chatting"),
            category("509658", "Just chatting")
        );
        assert_ne!(category("509658", "Same"), category("27471", "Same"));
    }

    /// The watch page is one place whatever the browse page was showing
    /// under it, and no browse place is the watch page.
    #[test]
    fn the_watch_page_is_its_own_route() {
        let mut discovery = Discovery::default();
        assert_eq!(route_of(Page::Watch, &discovery), Route::Watch);
        assert_eq!(
            route_of(Page::Browse, &discovery),
            Route::Tab(Tab::Following)
        );

        discovery.tab = Tab::Popular;
        assert_eq!(route_of(Page::Watch, &discovery), Route::Watch);
        for route in [
            Route::Tab(Tab::Popular),
            category("1", "a"),
            Route::Search("watch".into()),
            channel("watch", None),
        ] {
            assert_ne!(route, Route::Watch);
        }
        assert!(
            !reachable(&Route::Watch, false),
            "the watch page with nothing on it"
        );
        assert!(reachable(&Route::Watch, true));
        assert!(reachable(&Route::Tab(Tab::History), false));
    }
}
