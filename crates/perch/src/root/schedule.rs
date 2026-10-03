//! When offline channels say they will be on next, as the root runs it:
//! asking the worker the first time a view needs a channel's schedule, and
//! keeping its answer. Which broadcast is next, and the line that says it,
//! are worked out purely in `crate::schedule`; this is where that meets the
//! views and the sign-in.

use chrono::Utc;
use gpui::Context;
use twitch_api::schedule::Schedule;

use super::RootView;
use crate::browse::SignIn;
use crate::twitch::Request;

impl RootView {
    /// Ask the worker for `login`'s schedule, if nobody has this session.
    ///
    /// Called while drawing the two views that say it — an offline channel's
    /// page, and a pane that found its channel off — so a channel is asked
    /// about the first time one of them is on screen, however it got there:
    /// a click, the palette, back and forward, a pane stopping. Cheap enough
    /// to be: `Schedules::wants` is a lookup, and false for good once the
    /// channel has an answer or a failure. Only signed in, as `ask_badges`
    /// is, since Helix wants the token; signed out nothing is noted, so the
    /// first frame after sign-in asks. An ask the worker could not take is
    /// not noted as made, and the next frame tries again.
    pub(super) fn ask_schedule(&mut self, login: &str) {
        if !matches!(self.sign_in, SignIn::SignedIn(_)) || !self.schedules.wants(login) {
            return;
        }
        let user_id = self.known_user_id(login);
        if self.twitch.request(Request::Schedule {
            login: login.to_string(),
            user_id,
        }) {
            self.schedules.asked(login);
        }
    }

    /// The line saying when `login` is on next, now, if its schedule has
    /// one: see `schedule::Schedules::words`.
    pub(super) fn schedule_words(&self, login: &str) -> Option<String> {
        self.schedules.words(login, Utc::now())
    }

    /// The worker's answer about `login`'s schedule. A failure is one line
    /// in the log and no line on screen, and that channel is not asked
    /// about again this session; an answer redraws whatever says it.
    pub(super) fn on_schedule(
        &mut self,
        login: String,
        result: Result<Schedule, String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(reason) = self.schedules.answered(&login, result) {
            eprintln!("schedule: {reason}");
        }
        cx.notify();
    }
}
