//! The browse page's side of the conversation with the worker: which list is
//! showing, what to fetch for it, and what a click on the page means.

use gpui::{Context, Window};

use super::{Page, RootView};
use twitch_api::VideoKind;

use crate::browse::{Action, ChannelPage, SearchResults, SignIn, Tab};
use crate::twitch::Request;

impl RootView {
    /// Only claim to be loading if something is going to answer.
    ///
    /// Browsing needs a token like everything else. Before sign-in the worker
    /// is still parked on the device-code poll, so a request would sit in the
    /// queue behind it and the page would pulse "Loading…" until the user
    /// noticed the code on another tab. Say what is actually wrong instead, and
    /// come back to it in [`fill_tab`](Self::fill_tab) once signed in.
    pub(super) fn fetch(&mut self, request: Request) {
        self.discovery.error = None;
        self.discovery.loading = false;

        if !matches!(self.sign_in, SignIn::SignedIn(_)) {
            self.discovery.error = Some("Sign in to Twitch to browse.".into());
            return;
        }
        // Fails once the worker has returned, which it does when sign-in fails.
        if self.twitch.request(request) {
            self.discovery.loading = true;
        } else {
            self.discovery.error = Some("Not connected to Twitch.".into());
        }
    }

    /// Fetch whatever the open tab needs and does not already have.
    ///
    /// Asked for once and kept: these are network round trips, and the top of
    /// Twitch does not move in the time it takes to look at another tab.
    pub(super) fn fill_tab(&mut self) {
        match self.discovery.tab {
            Tab::Popular if self.discovery.popular.is_empty() => {
                self.fetch(Request::Popular { after: None })
            }
            Tab::Categories if self.discovery.categories.is_empty() => {
                self.fetch(Request::Categories { after: None })
            }
            // The history is the app's own; there is nothing to ask for.
            _ => {}
        }
    }

    /// Ask again for whatever is on screen.
    ///
    /// One control, whichever list is up, because "refresh" means the thing you
    /// are looking at. The discovery lists are otherwise fetched once per tab
    /// and kept forever, which is right for a page you glance at and wrong for
    /// one you have had open all evening.
    pub(super) fn refresh(&mut self, cx: &mut Context<Self>) {
        if let Some(page) = &self.discovery.channel {
            // The kind on screen, from its first page again.
            self.fetch(Request::Videos {
                login: page.login.clone(),
                user_id: page.user_id.clone(),
                kind: page.kind,
                after: None,
            });
            cx.notify();
            return;
        }
        if let Some(results) = &self.discovery.search {
            let query = results.query.to_string();
            self.run_search(query, cx);
            return;
        }
        // Refresh starts the list again rather than continuing it: the point is
        // to see what is on *now*, and appending a fresh page one onto a stale
        // page two would be neither.
        if let Some(category) = self.discovery.open.clone() {
            self.fetch(Request::Category {
                category,
                after: None,
            });
        } else {
            match self.discovery.tab {
                Tab::Following => self.refresh_follows(),
                Tab::Popular => self.fetch(Request::Popular { after: None }),
                Tab::Categories => self.fetch(Request::Categories { after: None }),
                // Nothing on it came from Twitch, so there is nothing to ask
                // again; the follows are what goes stale on this page.
                Tab::History => self.refresh_follows(),
            }
        }
        cx.notify();
    }

    /// Poll the follows lists now instead of at the next minute.
    ///
    /// Deliberately not routed through `fetch`, which owns the browse page's
    /// loading and error state: a follows refresh is not a browse request, and
    /// borrowing that flag would put "Loading…" over the popular tab.
    pub(super) fn refresh_follows(&mut self) {
        if !matches!(self.sign_in, SignIn::SignedIn(_)) {
            return;
        }
        self.refreshing = self.twitch.request(Request::Follows);
    }

    /// A tab of the browse page, from wherever the app is — the watch page
    /// included, which it leaves the way `Esc` does.
    pub(super) fn go_to_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::Watch {
            self.go_browse(cx);
        }
        self.show_tab(tab, window, cx);
    }

    pub(super) fn show_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        self.discovery.tab = tab;
        self.discovery.open = None;
        self.end_search(window, cx);
        self.discovery.channel = None;
        self.discovery.error = None;
        self.fill_tab();
        cx.notify();
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
    pub(super) fn run_search(&mut self, query: String, cx: &mut Context<Self>) {
        if query.is_empty() {
            self.discovery.search = None;
            self.discovery.error = None;
            cx.notify();
            return;
        }

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
        match action {
            Action::Watch(channel) => self.open_channel(channel, true, window, cx),
            Action::Add(channel) => self.open_channel(channel, false, window, cx),
            Action::OpenCategory(category) => {
                self.end_search(window, cx);
                self.discovery.channel = None;
                self.discovery.streams.clear();
                self.discovery.open = Some(category.clone());
                self.fetch(Request::Category {
                    category,
                    after: None,
                });
                cx.notify();
            }
            Action::CloseCategory => {
                self.discovery.open = None;
                self.discovery.streams.clear();
                self.discovery.error = None;
                cx.notify();
            }
            // What was typed into the Following tab's filter moves to the box
            // that asks Twitch, and is asked there — so the results page says
            // what it is showing, and back does not return to a filter that
            // still matches nobody.
            Action::Search(query) => {
                self.filter
                    .update(cx, |state, cx| state.set_value("", window, cx));
                self.search
                    .update(cx, |state, cx| state.set_value(query.clone(), window, cx));
                self.run_search(query, cx);
            }
            Action::CloseSearch => {
                self.end_search(window, cx);
                self.discovery.error = None;
                cx.notify();
            }
            Action::LoadMore => self.load_more(cx),
            Action::OpenSettings => self.toggle_settings(window, cx),
            Action::OpenChannel {
                login,
                display_name,
                user_id,
            } => self.open_channel_page(login, display_name, user_id, window, cx),
            Action::CloseChannel => {
                self.discovery.channel = None;
                self.discovery.error = None;
                cx.notify();
            }
            Action::ShowShelf(kind) => self.show_shelf(kind, cx),
            Action::WatchVideo(video) => self.open_video(*video, true, window, cx),
            Action::AddVideo(video) => self.open_video(*video, false, window, cx),
            Action::ForgetVideo(id) => self.forget_video(&id, cx),
            Action::ClearHistory => self.clear_history(cx),
        }
    }

    /// Look at a channel's past broadcasts.
    ///
    /// Takes over the browse page the way a category does, and gets there
    /// from the watch page the way the back control does, so whatever is
    /// playing is treated as leaving the watch page always treats it.
    pub(super) fn open_channel_page(
        &mut self,
        login: String,
        display_name: String,
        user_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.page == Page::Watch {
            self.go_browse(cx);
        }
        self.end_search(window, cx);
        self.discovery.open = None;
        self.discovery.streams.clear();
        let page = ChannelPage::new(login.clone(), display_name, user_id.clone());
        let kind = page.kind;
        self.discovery.channel = Some(page);
        self.fetch(Request::Videos {
            login,
            user_id,
            kind,
            after: None,
        });
        cx.notify();
    }

    /// Show another kind of the open channel's videos, fetching it the first
    /// time — or again, while it has come back with nothing: an empty list is
    /// one question away from being known to be empty now.
    fn show_shelf(&mut self, kind: VideoKind, cx: &mut Context<Self>) {
        let Some(page) = self.discovery.channel.as_mut() else {
            return;
        };
        page.kind = kind;
        self.discovery.error = None;
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
    /// Nothing here is reachable while `loading` is set: the row renders as
    /// "Loading…" and stops taking clicks, so one cursor cannot be spent twice.
    pub(super) fn load_more(&mut self, cx: &mut Context<Self>) {
        // Search results are not paginated — see `SEARCH_PAGE_SIZE`, where a
        // short list is the feature.
        if self.discovery.search.is_some() {
            return;
        }

        let request = if let Some(page) = &self.discovery.channel {
            page.videos().next.clone().map(|after| Request::Videos {
                login: page.login.clone(),
                user_id: page.user_id.clone(),
                kind: page.kind,
                after: Some(after),
            })
        } else if let Some(category) = self.discovery.open.clone() {
            self.discovery
                .streams
                .next
                .clone()
                .map(|after| Request::Category {
                    category,
                    after: Some(after),
                })
        } else {
            match self.discovery.tab {
                Tab::Popular => self
                    .discovery
                    .popular
                    .next
                    .clone()
                    .map(|after| Request::Popular { after: Some(after) }),
                Tab::Categories => self
                    .discovery
                    .categories
                    .next
                    .clone()
                    .map(|after| Request::Categories { after: Some(after) }),
                Tab::Following | Tab::History => None,
            }
        };

        if let Some(request) = request {
            self.fetch(request);
            cx.notify();
        }
    }
}
