//! What has been watched, as the root keeps it: every recording opened goes
//! into the history, where each one has got to is written down every few
//! seconds and whenever its pane goes, and a recording opened again picks up
//! there — from its channel's page, from the history tab, from the palette.
//! The list itself is `settings::history`; the tab that shows it is
//! `crate::history_page`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::Utc;
use gpui::{App, Context, Task};
use settings::history;
use twitch_api::Video;

use super::RootView;
use crate::channel_page;
use crate::history_page;
use crate::watch::{Slot, Source, StreamState};

/// How often a playing recording's place is written down.
///
/// A crash, a kill or a power cut loses at most this much, which picking up
/// a few seconds early costs nothing. The ordinary ways out — closing the
/// pane, the window, opening something else in its place — are written at
/// the time, so this is only the backstop; and a paused recording, standing
/// still, is not written at all.
const NOTE_EVERY: Duration = Duration::from_secs(15);

/// How long a run of history changes is left to settle before the file is
/// written: opening one recording solo closes three others, and that is one
/// write rather than four.
const SAVE_SETTLE: Duration = Duration::from_secs(1);

/// Now, as the history counts time.
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

/// Where one recording pane has got to: its video, the position, the length
/// as far as anyone knows it, whether it is still being recorded, and
/// whether it has been watched to the end.
fn progress_of(slot: &Slot, cx: &App) -> Option<(Video, f64, f64, bool, bool)> {
    let Source::Video { video, position } = &slot.source else {
        return None;
    };
    // The pane's handle rather than the player's, so a pane still opening
    // reads the place it is opening at rather than zero, and one whose
    // player has stopped reads where it stopped.
    let at = position.get();
    // The player's measure of the length when there is a player: the
    // playlist's, which grows as a broadcast still being made does.
    let (length, growing) = match slot.video().and_then(|view| view.read(cx).timeline()) {
        Some(timeline) => (timeline.extent(at), timeline.growing),
        None => (
            video.length_secs as f64,
            channel_page::in_progress(video, Utc::now()),
        ),
    };
    let finished =
        matches!(slot.state, StreamState::Ended) || history::finished_at(at, length, growing);
    Some(((**video).clone(), at, length, growing, finished))
}

impl RootView {
    /// Note every recording pane's place every [`NOTE_EVERY`], for as long as
    /// this view lives.
    pub(super) fn keep_history(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(NOTE_EVERY).await;
            let alive = this.update(cx, |this: &mut RootView, cx| {
                if this.note_watching(cx) {
                    this.save_history();
                }
            });
            if alive.is_err() {
                break;
            }
        })
    }

    /// Where `video_id` picks up when it is opened: where it was left, or
    /// the top.
    pub(super) fn resume_point(&self, video_id: &str) -> f64 {
        self.history.resume_at(video_id).unwrap_or(0.0)
    }

    /// Note that `video` has just been opened at `start_at`: it leads the
    /// history from now, whether or not it gets any further.
    pub(super) fn note_opened(&mut self, video: &Video, start_at: f64, cx: &mut Context<Self>) {
        let length = video.length_secs as f64;
        let growing = channel_page::in_progress(video, Utc::now());
        self.history.opened(history_page::watched(
            video,
            start_at,
            length,
            growing,
            false,
            unix_now(),
        ));
        self.save_history_soon(cx);
    }

    /// Note where every recording pane has got to. Returns whether that
    /// changed the history.
    pub(super) fn note_watching(&mut self, cx: &App) -> bool {
        let now = unix_now();
        let mut changed = false;
        for (video, at, length, growing, finished) in
            self.slots.iter().filter_map(|slot| progress_of(slot, cx))
        {
            changed |= self
                .history
                .progressed(&video.id, at, length, finished, now);
            // The player knows what the listing could not — that a broadcast
            // being recorded when it was opened has since finished — and
            // what the history keeps of the video follows it.
            if !growing && channel_page::placeholder(&video.thumbnail_url) {
                let kept = history_page::watched(&video, at, length, growing, finished, now);
                changed |= self.history.refresh(&kept);
            }
        }
        changed
    }

    /// Take a channel's freshly listed recordings as the newest word on any
    /// the history holds: a title edited since, the picture Twitch puts on a
    /// broadcast once it has finished.
    pub(super) fn refresh_history(&mut self, videos: &[Video], cx: &mut Context<Self>) {
        let mut changed = false;
        for video in videos {
            if self.history.get(&video.id).is_some() {
                let listed = history_page::watched(video, 0.0, 0.0, true, false, 0);
                changed |= self.history.refresh(&listed);
            }
        }
        if changed {
            self.save_history_soon(cx);
        }
    }

    /// Take one recording off the history.
    pub(super) fn forget_video(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.history.forget(id) {
            self.save_history_soon(cx);
            cx.notify();
        }
    }

    /// Take every recording off the history.
    ///
    /// Anything playing goes back on as it plays: it is still open, and
    /// forgetting what is on screen would only last until the next note.
    pub(super) fn clear_history(&mut self, cx: &mut Context<Self>) {
        if self.history.clear() {
            self.save_history_soon(cx);
            cx.notify();
        }
    }

    /// Write the history down once it has stopped changing; see
    /// [`SAVE_SETTLE`].
    pub(super) fn save_history_soon(&mut self, cx: &mut Context<Self>) {
        self.history_epoch += 1;
        let epoch = self.history_epoch;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_SETTLE).await;
            let _ = this.update(cx, |this: &mut RootView, _cx| {
                if this.history_epoch == epoch {
                    this.save_history();
                }
            });
        })
        .detach();
    }

    /// Write the history down now.
    ///
    /// A failure goes to the log rather than to a toast: this runs every few
    /// seconds while a recording plays, and a disk that refuses once will
    /// refuse again, which would be a toast every fifteen seconds for as
    /// long as it did.
    pub(super) fn save_history(&mut self) {
        if let Err(e) = self.history.save(&self.history_path) {
            eprintln!("history: could not save: {e}");
        }
    }

    /// Where every recording had got to, written down on the way out: the
    /// window is closing, and nothing scheduled will run after it.
    pub(crate) fn remember_watching(&mut self, cx: &mut Context<Self>) {
        self.note_watching(cx);
        self.save_history();
    }
}
