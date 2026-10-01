//! The browse page's side of the conversation with the worker: which list is
//! showing, what to fetch for it, and what a click on the page means.

use gpui::{Context, Point, Window};

use super::{Page, RootView};
use twitch_api::VideoKind;

use crate::browse::{Action, ChannelPage, Place, SearchResults, SignIn, Tab, Unanswered};
use crate::twitch::Request;

impl RootView {
    /// Ask the worker for one of the browse page's lists, and wait on it by
    /// name: what the request fills is its `ListKey`, and the answer, or its
    /// failure, is matched back to that list rather than to whichever list
    /// happens to be on screen when it lands.
    ///
    /// Only claim to be loading if something is going to answer. Browsing
    /// needs a token like everything else. Before sign-in the worker is still
    /// parked on the device-code poll, so a request would sit in the queue
    /// behind it and the page would pulse "Loading…" until the user noticed
    /// the code on another tab. Say what is actually wrong instead, on the
    /// list that asked — that nobody is signed in, which is not a failure to
    /// reach Twitch — and come back to it in [`fill_shown`](Self::fill_shown)
    /// once signed in.
    pub(super) fn fetch(&mut self, request: Request) {
        let key = request.list_key();
        debug_assert!(key.is_some(), "{request:?} fills no browse list");
        let Some(key) = key else {
            return;
        };
        // Asked again, the list's last failure is no longer the news; another
        // list's stays with that list.
        if self
            .discovery
            .error
            .as_ref()
            .is_some_and(|(failed, _)| *failed == key)
        {
            self.discovery.error = None;
        }

        if !matches!(self.sign_in, SignIn::SignedIn(_)) {
            self.discovery.error = Some((key, Unanswered::SignedOut));
            return;
        }
        // Fails once the worker has returned, which it does when sign-in fails.
        if self.twitch.request(request) {
            self.discovery.start(key);
        } else {
            self.discovery.error =
                Some((key, Unanswered::Failed("Not connected to Twitch.".into())));
        }
    }

    /// Fetch whatever list is on screen, if it has nothing and is not already
    /// being asked for: a tab, a category, a search or a channel's shelf.
    ///
    /// Asked for once and kept: these are network round trips, and the top of
    /// Twitch does not move in the time it takes to look at another tab. And
    /// called when sign-in lands, so a list opened while signed out — a
    /// search typed into the title bar, a channel's page from the palette —
    /// fills in then, rather than going on saying it needs a sign-in that has
    /// already happened.
    pub(super) fn fill_shown(&mut self) {
        let discovery = &self.discovery;
        let place = discovery.place();
        let empty = match &place {
            Place::Tab(Tab::Popular) => discovery.popular.is_empty(),
            Place::Tab(Tab::Categories) => discovery.categories.is_empty(),
            Place::Category(_) => discovery.streams.is_empty(),
            Place::Search(results) => results.is_empty(),
            Place::Channel(page) => page.videos().is_empty(),
            // Following is the follows poll's, and the history is the app's
            // own; neither has anything to ask for here.
            Place::Tab(Tab::Following | Tab::History) => false,
        };
        let request = place.first_page().filter(|_| empty);
        if let Some(request) = request {
            if !self.discovery.is_loading() {
                self.fetch(request);
            }
        }
    }

    /// Ask again for whatever is on screen.
    ///
    /// One control, whichever list is up, because "refresh" means the thing you
    /// are looking at. The discovery lists are otherwise fetched once per tab
    /// and kept forever, which is right for a page you glance at and wrong for
    /// one you have had open all evening.
    pub(super) fn refresh(&mut self, cx: &mut Context<Self>) {
        // Refresh starts the list again rather than continuing it: the point is
        // to see what is on *now*, and appending a fresh page one onto a stale
        // page two would be neither.
        let request = match self.discovery.place() {
            // Through `ask_search`, which seeds the results afresh, so the
            // page says what it is waiting for and a stale reply is known.
            Place::Search(results) => {
                let query = results.query.to_string();
                self.ask_search(query, cx);
                return;
            }
            // Nothing on the history came from Twitch, so there is nothing to
            // ask again; the follows are what goes stale on that page.
            Place::Tab(Tab::Following | Tab::History) => {
                self.refresh_follows();
                cx.notify();
                return;
            }
            // The rest from their first page again — a channel's page the
            // kind on screen.
            place => place.first_page(),
        };
        if let Some(request) = request {
            self.fetch(request);
        }
        cx.notify();
    }

    /// Poll the follows lists now instead of at the next minute.
    ///
    /// Deliberately not routed through `fetch`, which waits on the browse
    /// lists by their `ListKey`: the follows poll fills none of them, so it
    /// has no key to wait on, and its failure is the follows toast rather
    /// than an error said on a list. Its own wait is `refreshing`.
    pub(super) fn refresh_follows(&mut self) {
        if !matches!(self.sign_in, SignIn::SignedIn(_)) {
            return;
        }
        self.refreshing = self.twitch.request(Request::Follows);
    }

    /// A tab of the browse page, from wherever the app is — the watch page
    /// included, which it leaves the way `Esc` does. One step on the trail.
    pub(super) fn go_to_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        self.record(|this| {
            if this.page == Page::Watch {
                this.go_browse(cx);
            }
            this.show_tab(tab, window, cx);
        })
    }

    pub(super) fn show_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        self.record(|this| {
            this.discovery.tab = tab;
            this.discovery.open = None;
            this.end_search(window, cx);
            this.discovery.channel = None;
            this.fill_shown();
            cx.notify();
        })
    }

    /// Leave the search results, and empty the box that asked for them.
    ///
    /// Every way off the results comes through here — back, a tab, a category
    /// or a channel's page opened from them — because the query used to stay
    /// in the box after its results had gone, where it read as though they
    /// were still what the page was showing.
    fn end_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.discovery.search.take().is_some() {
            self.search
                .update(cx, |state, cx| state.set_value("", window, cx));
        }
    }

    /// Run a search, or clear the results if the box is empty.
    ///
    /// The box is in the title bar, over both pages, so a search can start on
    /// the watch page. It leaves it for the results the way `Esc` does —
    /// whatever is playing carries on in the mini player, or stops with that
    /// turned off — because a search is asked for to be looked at. Windowless,
    /// like the box's own subscription that calls it, which is why recording
    /// a step needs no window either. New results start at the top.
    pub(super) fn run_search(&mut self, query: String, cx: &mut Context<Self>) {
        self.record(|this| {
            if query.is_empty() {
                this.discovery.search = None;
                cx.notify();
                return;
            }
            if this.page == Page::Watch {
                this.go_browse(cx);
            }
            this.scrolls.search.set_offset(Point::default());
            this.ask_search(query, cx);
        })
    }

    /// Ask Twitch for `query`, wherever the app is.
    ///
    /// Split from [`run_search`](Self::run_search) for the sake of `refresh`,
    /// which asks again for the results already up and has no business moving
    /// the page: a refresh from the palette on the watch page refetches them
    /// where they are, as it does every other list.
    fn ask_search(&mut self, query: String, cx: &mut Context<Self>) {
        // Seeded with the query before the answer arrives, so the page can say
        // what it is waiting for and can recognise a stale reply when it lands.
        self.discovery.open = None;
        self.discovery.channel = None;
        self.discovery.search = Some(SearchResults {
            query: query.clone().into(),
            ..Default::default()
        });
        self.fetch(Request::Search(query));
        cx.notify();
    }

    pub(super) fn on_browse_action(
        &mut self,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // One step on the trail, whatever the action turns out to move: a
        // card, a category, a channel's page, a context bar's back.
        self.record(|this| match action {
            Action::Watch(channel) => this.open_channel(channel, true, window, cx),
            Action::Add(channel) => this.open_channel(channel, false, window, cx),
            Action::OpenCategory(category) => {
                this.end_search(window, cx);
                this.discovery.channel = None;
                this.discovery.streams.clear();
                // A new list starts at the top, rather than wherever the
                // last one opened here was left.
                this.scrolls.category.set_offset(Point::default());
                this.discovery.open = Some(category.clone());
                this.fetch(Request::Category {
                    category,
                    after: None,
                });
                cx.notify();
            }
            Action::CloseCategory => {
                this.discovery.open = None;
                this.discovery.streams.clear();
                cx.notify();
            }
            // What was typed into the Following tab's filter moves to the box
            // that asks Twitch, and is asked there — so the results page says
            // what it is showing, and back does not return to a filter that
            // still matches nobody.
            Action::Search(query) => {
                this.filter
                    .update(cx, |state, cx| state.set_value("", window, cx));
                this.search
                    .update(cx, |state, cx| state.set_value(query.clone(), window, cx));
                this.run_search(query, cx);
            }
            Action::CloseSearch => {
                this.end_search(window, cx);
                cx.notify();
            }
            Action::LoadMore => this.load_more(cx),
            Action::OpenSettings => this.toggle_settings(window, cx),
            Action::OpenChannel {
                login,
                display_name,
                user_id,
            } => this.open_channel_page(login, display_name, user_id, window, cx),
            Action::CloseChannel => {
                this.discovery.channel = None;
                cx.notify();
            }
            Action::ShowShelf(kind) => this.show_shelf(kind, cx),
            Action::WatchVideo(video) => this.open_video(*video, true, window, cx),
            Action::AddVideo(video) => this.open_video(*video, false, window, cx),
            Action::ForgetVideo(id) => this.forget_video(&id, cx),
            Action::ClearHistory => this.clear_history(cx),
            // Moves a row, not the app: nothing is recorded, since `record`
            // only takes a step when where the app is has changed.
            Action::SetPinned { login, pinned } => this.set_pinned(login, pinned, cx),
        })
    }

    /// Look at a channel's past broadcasts.
    ///
    /// Takes over the browse page the way a category does, and gets there
    /// from the watch page the way `Esc` does, so whatever is playing is
    /// treated as leaving the watch page always treats it. One step on the
    /// trail.
    pub(super) fn open_channel_page(
        &mut self,
        login: String,
        display_name: String,
        user_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.record(|this| {
            if this.page == Page::Watch {
                this.go_browse(cx);
            }
            this.end_search(window, cx);
            this.discovery.open = None;
            this.discovery.streams.clear();
            // Its own list, from the top; see `OpenCategory`.
            this.scrolls.channel.set_offset(Point::default());
            let page = ChannelPage::new(login.clone(), display_name, user_id.clone());
            let kind = page.kind;
            this.discovery.channel = Some(page);
            this.fetch(Request::Videos {
                login,
                user_id,
                kind,
                after: None,
            });
            cx.notify();
        })
    }

    /// Show another kind of the open channel's videos, fetching it the first
    /// time — or again, while it has come back with nothing: an empty list is
    /// one question away from being known to be empty now.
    fn show_shelf(&mut self, kind: VideoKind, cx: &mut Context<Self>) {
        let Some(page) = self.discovery.channel.as_mut() else {
            return;
        };
        page.kind = kind;
        if page.shelf(kind).is_empty() {
            let request = Request::Videos {
                login: page.login.clone(),
                user_id: page.user_id.clone(),
                kind,
                after: None,
            };
            self.fetch(request);
        }
        cx.notify();
    }

    /// Ask for the next page of whichever list is on screen.
    ///
    /// The cursor comes from the list itself rather than from the click, so a
    /// stale button cannot ask for a page that has already arrived — and a
    /// list with nothing left simply has no cursor, which is also what takes
    /// the row away.
    ///
    /// Nothing here is reachable while the list on screen is waiting on an
    /// answer (`Discovery::is_loading`): the row renders as "Loading…" and
    /// stops taking clicks, so one cursor cannot be spent twice.
    pub(super) fn load_more(&mut self, cx: &mut Context<Self>) {
        let discovery = &self.discovery;
        let request = match discovery.place() {
            Place::Channel(page) => page.videos().next.clone().map(|after| Request::Videos {
                login: page.login.clone(),
                user_id: page.user_id.clone(),
                kind: page.kind,
                after: Some(after),
            }),
            // Search results are not paginated — see `SEARCH_PAGE_SIZE`, where
            // a short list is the feature.
            Place::Search(_) => None,
            Place::Category(category) => {
                discovery
                    .streams
                    .next
                    .clone()
                    .map(|after| Request::Category {
                        category: category.clone(),
                        after: Some(after),
                    })
            }
            Place::Tab(Tab::Popular) => discovery
                .popular
                .next
                .clone()
                .map(|after| Request::Popular { after: Some(after) }),
            Place::Tab(Tab::Categories) => discovery
                .categories
                .next
                .clone()
                .map(|after| Request::Categories { after: Some(after) }),
            Place::Tab(Tab::Following | Tab::History) => None,
        };

        if let Some(request) = request {
            self.fetch(request);
            cx.notify();
        }
    }
}
