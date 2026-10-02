//! When the offline follows were last live, as the root runs it: when to ask
//! the worker, and what its answer does. When to ask, and the words an
//! answer comes to, are worked out purely in `crate::last_live`; this is
//! where that meets the follows and the sign-in.

use std::collections::HashMap;
use std::time::Instant;

use gpui::Context;
use twitch_api::recommend::LastBroadcast;

use super::RootView;
use crate::browse::SignIn;
use crate::twitch::{RecommendError, Request};

impl RootView {
    /// Ask the worker about whichever offline follows are due, if any are.
    ///
    /// Called when a follows list lands and when an answer comes back, and
    /// cheap enough to be: `LastLive::next_ask` asks nothing when nobody is
    /// new and the interval is not up, however often it is called. Only
    /// signed in, as `ask_recommended` is and for its reason: the worker
    /// reads its requests only once sign-in has got it into its loop. An ask
    /// the worker could not take is not noted as made, so the next follows
    /// list tries again.
    pub(super) fn ask_last_live(&mut self) {
        if !matches!(self.sign_in, SignIn::SignedIn(_)) {
            return;
        }
        let now = Instant::now();
        let offline = self.offline.iter().map(|channel| channel.login.as_str());
        let Some(ask) = self.last_live.next_ask(offline, now) else {
            return;
        };
        if self.twitch.request(Request::LastLive {
            logins: ask.logins.clone(),
        }) {
            self.last_live.asked(ask, now);
        }
    }

    /// The worker's answer to the ask that was out. A failure is one line in
    /// the log, and the names it was for go without the words; nothing on
    /// screen says so. A refusal is one line too, and the end of asking. Then whoever came due meanwhile — a stream that ended
    /// while this ask was out — is asked about.
    pub(super) fn on_last_live(
        &mut self,
        result: Result<HashMap<String, LastBroadcast>, RecommendError>,
        cx: &mut Context<Self>,
    ) {
        if let Some(reason) = self.last_live.answered(result) {
            eprintln!("last live: {reason}");
        }
        self.ask_last_live();
        cx.notify();
    }
}
