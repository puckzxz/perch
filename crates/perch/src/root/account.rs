//! The sign-in as the user drives it: signing out from the settings sheet,
//! signing in from the empty state or a chat's composer, signing in again
//! for a scope the old sign-in lacks, and starting the Twitch worker over,
//! which a new client id does too.
//!
//! The worker owns the session (see `crate::twitch`), so every one of these
//! is the old worker dropped and, unless the user is signing out, a new one
//! started. Dropping a worker sets its `stop` flag, which `persist` checks
//! before it writes a token, so the tokens are forgotten only after it is
//! stopped and a refresh it was in the middle of cannot write them back.
//! Only signing out forgets them: signing in again leaves them on disk
//! until the new sign-in replaces them (`SignInStart::Fresh`).

use gpui::{Context, Task, Window};
use settings::Settings;

use super::RootView;
use crate::browse::{Discovery, SignIn};
use crate::twitch::{SignInStart, TwitchService};

impl RootView {
    /// Stop the worker, with nothing in its place, and forget everything it
    /// fed: the follows, the browse lists fetched with its token and the
    /// trail through them, the panes' asks, whether its token may send, and
    /// the messages it was sending, each of which says so in its chat. What
    /// signing out leaves, and what every restart begins with.
    fn stop_twitch(&mut self, cx: &mut Context<Self>) {
        self.twitch = TwitchService::stopped();
        self._twitch_pump = Task::ready(());
        self.chat_scope = None;
        // "May": the worker declines anything still queued once it is
        // stopped (see `twitch::run`), but one already on its way to Twitch
        // finishes, and its answer has nowhere to go.
        self.drop_chat_sends("Message may not have been sent: the sign-in stopped", cx);
        self.follows.clear();
        self.offline.clear();
        self.home_offline.clear();
        self.known_live.clear();
        self.streams_seeded = false;
        self.avatars.clear();
        self.follows_loaded = false;
        self.follows_complete = false;
        self.refreshing = false;
        // Browsing was fetched with the old token, so it goes with it — and
        // so does the trail through it, whose places were read from what
        // just went.
        self.discovery = Discovery::default();
        self.trail.clear();
        // And the panes' asks about past broadcasts, whose answers die with
        // the old worker; the new one's sign-in asks again.
        self.forget_asks();
        // And the panes' asks about muted stretches, for the same reason.
        self.muted.forget_asks();
    }

    /// Start the worker over from what `settings.json` says: a new client
    /// id saved from the sheet, or a sign-in asked for. From `Stored`, with
    /// stored tokens it signs in with them; without, or from `Fresh`, it
    /// starts the device flow and the code goes up where the sign-in is
    /// said (the title bar, the empty state, each chat's composer).
    pub(super) fn restart_twitch(
        &mut self,
        start: SignInStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stop_twitch(cx);
        self.sign_in = SignIn::Connecting;
        let (service, pump) = Self::spawn_twitch(self.settings_path.clone(), start, window, cx);
        self.twitch = service;
        self._twitch_pump = pump;
        self.sync_chat_access(cx);
        self.sync_settings_sign_in(cx);
        cx.notify();
    }

    /// Tell the settings sheet, if it is up, how the sign-in stands: after
    /// signing out or in from it, and after every worker event
    /// (`spawn_twitch`), so a sign-in started from the sheet says how it is
    /// going there.
    pub(super) fn sync_settings_sign_in(&mut self, cx: &mut Context<Self>) {
        if let Some(panel) = self.settings_panel.clone() {
            let sign_in = self.sign_in.clone();
            panel.update(cx, |panel, cx| panel.set_sign_in(&sign_in, cx));
        }
    }

    /// Sign in: the empty state's, the browse lists' and the settings
    /// sheet's `Sign in` once signed out, and a closed composer's after a
    /// sign-in that failed. Nothing is forgotten first, so stored tokens are
    /// tried before a new code is asked for — which is also what brings back
    /// a sign-in whose `Sign in again` code was never entered.
    pub(super) fn start_sign_in(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window);
        self.restart_twitch(SignInStart::Stored, window, cx);
    }

    /// The settings sheet's `Sign out`: the worker stopped, the stored
    /// tokens forgotten through `Settings::sign_out` (which keeps every
    /// other field the file holds, under the lock every save takes), and
    /// the app as it is with no sign-in: the follows gone, the rail and
    /// Home saying `Signed out` with a `Sign in` beside it, and every
    /// composer the same. No worker runs, so nothing asks Twitch for a code
    /// until the user presses Sign in.
    pub(super) fn sign_out(&mut self, cx: &mut Context<Self>) {
        self.stop_twitch(cx);
        self.forget_tokens(cx);
        self.sign_in = SignIn::SignedOut;
        self.sync_chat_access(cx);
        self.sync_settings_sign_in(cx);
        cx.notify();
    }

    /// A closed composer's `Sign in again`: a sign-in from before sending
    /// was asked for cannot gain the scope by refreshing, so the device flow
    /// starts over, asking for every scope (`twitch_api::SCOPES`). The old
    /// tokens are not forgotten: they still do everything but chat, and stay
    /// on disk until the new sign-in is written over them, so a code the
    /// user walks away from leaves `Sign in`, which signs in with them
    /// again, not a sign-in lost. The follows go while the code is up, as on
    /// any sign-in, and come back with it.
    pub(super) fn sign_in_again(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window);
        self.restart_twitch(SignInStart::Fresh, window, cx);
    }

    /// Forget the stored sign-in on disk and in the root's copy. Only once
    /// the worker is stopped; see the module.
    fn forget_tokens(&mut self, cx: &mut Context<Self>) {
        self.settings.credentials.oauth = None;
        if let Err(e) = Settings::sign_out(&self.settings_path) {
            eprintln!("settings: could not forget the sign-in: {e}");
            self.toast(format!("Could not forget the sign-in: {e}"), cx);
        }
    }
}
