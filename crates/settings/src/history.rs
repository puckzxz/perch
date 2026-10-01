//! What has been watched: every recording opened, how far into it, and when.
//!
//! A file of its own beside `settings.json` rather than more fields in it, for
//! two reasons. It changes all the time — a position is written down every few
//! seconds while a recording plays — and `settings.json` holds the sign-in,
//! whose file is better rewritten as seldom as possible. And it is the app's
//! memory rather than the user's choices: deleting `history.json` forgets what
//! was watched and changes nothing else.
//!
//! Each entry keeps enough of its video to be listed and played again without
//! asking Twitch anything, the way a recording is opened from its channel's
//! page. What that costs is freshness: a title edited since, or the picture
//! Twitch puts on a recording once it has finished being made, arrives only
//! when the video is next listed somewhere the app reads it from.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Error;

/// How many recordings are remembered, newest first.
///
/// Twitch keeps a recording for a week, or two months for the channels it
/// pays, so a list much longer than this is mostly videos that are gone. The
/// oldest falls off the end as a new one arrives.
pub const HISTORY_LIMIT: usize = 50;

/// How near the end a recording counts as watched, in seconds.
///
/// Not the very end: a broadcast's last minute is a goodbye and an outro, and
/// a viewer who closes the pane there has finished with it. Picking up
/// thirty seconds from the end would open straight onto "finished".
pub const FINISHED_WITHIN_SECS: f64 = 60.0;

/// Whether playback that got to `position` of a `length`-second recording has
/// finished with it.
///
/// Within [`FINISHED_WITHIN_SECS`] of the end, or the last tenth of anything
/// short enough that a minute would be most of it. Never while it is still
/// being recorded: a viewer who has caught up with a broadcast that is still
/// going is at its edge, not its end.
pub fn finished_at(position: f64, length: f64, growing: bool) -> bool {
    if growing || length <= 0.0 {
        return false;
    }
    position >= (length - FINISHED_WITHIN_SECS).max(length * 0.9)
}

/// One recording, and where it was left.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Watched {
    /// Twitch's id for the video: what it is played and linked by.
    pub id: String,
    pub channel_login: String,
    /// The channel's name as it writes it, for the list.
    pub channel_name: String,
    /// The channel's Helix id, which its chat replay is asked for by.
    pub channel_id: String,
    pub title: String,
    /// RFC 3339: when the broadcast started.
    pub created_at: String,
    /// How long it was last known to be. A recording still being made when
    /// it was watched grows here as it is watched.
    pub length_secs: u64,
    /// Twitch's template for its picture, `%{width}` and all.
    pub thumbnail_url: String,
    /// What Twitch calls it: `archive`, `highlight` or `upload`.
    pub kind: String,
    /// Where it was left, in whole seconds from the start.
    pub position_secs: u64,
    /// Watched to the end, or near enough — see [`finished_at`]. Opening it
    /// again starts from the top.
    pub finished: bool,
    /// Unix seconds: when it was last opened, or last got further.
    pub watched_at: u64,
}

impl Watched {
    /// Whose it is, as the list says it: the channel's name as it writes it,
    /// or its login for an entry that never learned the name.
    pub fn channel(&self) -> &str {
        if self.channel_name.is_empty() {
            &self.channel_login
        } else {
            &self.channel_name
        }
    }

    /// How much of it has been watched, from nothing to all of it. A length
    /// Twitch never said is nothing rather than a division by zero.
    pub fn progress(&self) -> f32 {
        if self.finished {
            return 1.0;
        }
        if self.length_secs == 0 {
            return 0.0;
        }
        (self.position_secs as f64 / self.length_secs as f64).clamp(0.0, 1.0) as f32
    }
}

/// Every recording watched, newest first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct History {
    pub videos: Vec<Watched>,
}

/// What [`History::forget`] or [`History::clear`] took off the list, each
/// entry with the place it held, so [`History::restore`] can put it back
/// there. A slip of the pointer onto "forget" used to lose where a twelve-hour
/// broadcast was left, for good.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Forgotten {
    /// In the order the entries stood, which is the order they go back in.
    entries: Vec<(usize, Watched)>,
}

impl Forgotten {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The recording taken off, when it was just the one.
    pub fn only(&self) -> Option<&Watched> {
        match self.entries.as_slice() {
            [(_, watched)] => Some(watched),
            _ => None,
        }
    }
}

/// Where the history lives: beside the settings, as `history.json`.
pub fn default_path(app_name: &str) -> PathBuf {
    crate::default_path(app_name).with_file_name("history.json")
}

impl History {
    /// Load from `path`. A missing file is an empty history, which is what
    /// the first run has.
    pub fn load(path: &Path) -> Result<Self, Error> {
        let Some(text) = crate::read_if_present(path)? else {
            return Ok(Self::default());
        };
        let mut history: Self = serde_json::from_str(&text).map_err(|source| Error::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        // A hand-edited file can hold anything, including more than the app
        // would ever have written.
        history.videos.truncate(HISTORY_LIMIT);
        Ok(history)
    }

    /// Write to `path`, whole.
    pub fn save(&self, path: &Path) -> Result<(), Error> {
        let text = serde_json::to_string_pretty(self).expect("history is always serialisable");
        crate::write_atomically(path, &text)
    }

    pub fn get(&self, id: &str) -> Option<&Watched> {
        self.videos.iter().find(|watched| watched.id == id)
    }

    /// Where to pick `id` up: where it was left, unless it was finished or
    /// barely begun. `None` means from the top.
    pub fn resume_at(&self, id: &str) -> Option<f64> {
        self.get(id)
            .filter(|watched| !watched.finished && watched.position_secs > 0)
            .map(|watched| watched.position_secs as f64)
    }

    /// Note that `watched` has just been opened. It leads the list, replacing
    /// whatever was known about it — the pane holds the fresher copy of the
    /// video — and the oldest entry falls off the end if there are too many.
    pub fn opened(&mut self, watched: Watched) {
        self.videos.retain(|known| known.id != watched.id);
        self.videos.insert(0, watched);
        self.videos.truncate(HISTORY_LIMIT);
    }

    /// Note how far into `id` playback has got, at `now`.
    ///
    /// Returns whether anything changed, so a caller asking every few seconds
    /// writes the file only when there is something new in it: a paused
    /// recording is still at the same second, and says so by returning false
    /// rather than by moving to the top of the list again. A recording that
    /// got further does move there, since it is the one most recently watched.
    /// A video that is not in the history is left out of it; opening is what
    /// puts one there.
    pub fn progressed(
        &mut self,
        id: &str,
        position: f64,
        length: f64,
        finished: bool,
        now: u64,
    ) -> bool {
        let Some(index) = self.videos.iter().position(|watched| watched.id == id) else {
            return false;
        };
        let position = position.max(0.0).floor() as u64;
        // Only ever longer: a recording that was still being made when it was
        // listed is at least as long as the furthest anyone has played it.
        let length = self.videos[index]
            .length_secs
            .max(length.max(0.0).floor() as u64);
        let entry = &self.videos[index];
        if entry.position_secs == position
            && entry.finished == finished
            && entry.length_secs == length
        {
            return false;
        }
        let mut entry = self.videos.remove(index);
        entry.position_secs = position;
        entry.length_secs = length;
        entry.finished = finished;
        entry.watched_at = now;
        self.videos.insert(0, entry);
        true
    }

    /// Take what a fresher listing says about a video, where the history
    /// already has it: the title, the picture, the length. Where it was left
    /// and when are the history's own, and stay. Returns whether anything
    /// changed.
    pub fn refresh(&mut self, fresh: &Watched) -> bool {
        let Some(entry) = self.videos.iter_mut().find(|known| known.id == fresh.id) else {
            return false;
        };
        let updated = Watched {
            position_secs: entry.position_secs,
            finished: entry.finished,
            watched_at: entry.watched_at,
            length_secs: entry.length_secs.max(fresh.length_secs),
            ..fresh.clone()
        };
        if *entry == updated {
            return false;
        }
        *entry = updated;
        true
    }

    /// Forget one recording, and say what was taken off: nothing, when it was
    /// not there.
    pub fn forget(&mut self, id: &str) -> Forgotten {
        let entries = match self.videos.iter().position(|watched| watched.id == id) {
            Some(index) => vec![(index, self.videos.remove(index))],
            None => Vec::new(),
        };
        Forgotten { entries }
    }

    /// Forget everything, and say what that was.
    pub fn clear(&mut self) -> Forgotten {
        Forgotten {
            entries: std::mem::take(&mut self.videos)
                .into_iter()
                .enumerate()
                .collect(),
        }
    }

    /// Put back what was forgotten, each entry where it stood. One opened
    /// again since is already back, with a newer place than the one
    /// forgotten, and keeps it. Returns whether anything went back.
    pub fn restore(&mut self, forgotten: Forgotten) -> bool {
        let mut changed = false;
        for (index, watched) in forgotten.entries {
            if self.get(&watched.id).is_some() {
                continue;
            }
            self.videos.insert(index.min(self.videos.len()), watched);
            changed = true;
        }
        self.videos.truncate(HISTORY_LIMIT);
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watched(id: &str, position_secs: u64) -> Watched {
        Watched {
            id: id.into(),
            channel_login: "someone".into(),
            channel_name: "Someone".into(),
            title: format!("broadcast {id}"),
            length_secs: 36_000,
            kind: "archive".into(),
            position_secs,
            watched_at: 1,
            ..Default::default()
        }
    }

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join("perch-tests")
            .join(format!("{name}.json"))
    }

    /// The end of a long recording, the last tenth of a short one, and
    /// nothing at all while it is still being made.
    #[test]
    fn finished_means_near_the_end_of_something_that_has_one() {
        let twelve_hours = 12.0 * 3600.0;
        assert!(!finished_at(twelve_hours - 120.0, twelve_hours, false));
        assert!(finished_at(twelve_hours - 59.0, twelve_hours, false));
        assert!(finished_at(twelve_hours, twelve_hours, false));
        assert!(
            !finished_at(twelve_hours, twelve_hours, true),
            "caught up with a broadcast still going is its edge, not its end"
        );

        // Forty-five seconds long: a minute from the end is before the start.
        assert!(!finished_at(10.0, 45.0, false));
        assert!(finished_at(41.0, 45.0, false));
        assert!(!finished_at(0.0, 0.0, false), "no length, no end");
    }

    /// Picked up where it was left — unless it was finished, or never got
    /// anywhere, both of which start from the top.
    #[test]
    fn a_recording_resumes_where_it_was_left_unless_it_was_finished() {
        let mut history = History::default();
        assert_eq!(history.resume_at("1"), None, "never watched");

        history.opened(watched("1", 4_000));
        assert_eq!(history.resume_at("1"), Some(4_000.0));

        history.opened(watched("2", 0));
        assert_eq!(history.resume_at("2"), None, "barely begun");

        history.progressed("1", 35_990.0, 36_000.0, true, 2);
        assert_eq!(history.resume_at("1"), None, "finished");
        assert_eq!(history.get("1").unwrap().progress(), 1.0);
    }

    /// Opening leads the list; so does getting further. Standing still —
    /// a paused recording asked about every few seconds — changes nothing,
    /// including the order and the file.
    #[test]
    fn the_list_is_newest_first_and_a_pause_does_not_reorder_it() {
        let mut history = History::default();
        history.opened(watched("1", 0));
        history.opened(watched("2", 0));
        assert_eq!(history.videos[0].id, "2");

        assert!(history.progressed("1", 120.4, 36_000.0, false, 50));
        assert_eq!(history.videos[0].id, "1", "got further, so it leads");
        assert_eq!(history.videos[0].position_secs, 120, "whole seconds");
        assert_eq!(history.videos[0].watched_at, 50);

        assert!(
            !history.progressed("1", 120.9, 36_000.0, false, 90),
            "the same second again"
        );
        assert_eq!(history.videos[0].watched_at, 50, "a pause is not watching");

        assert!(
            !history.progressed("3", 10.0, 100.0, false, 90),
            "only opening puts a video in the history"
        );
        assert!(history.get("3").is_none());

        // Opened again: back to the top, once, with its fresh copy.
        history.opened(watched("2", 0));
        assert_eq!(
            history
                .videos
                .iter()
                .map(|w| w.id.as_str())
                .collect::<Vec<_>>(),
            ["2", "1"]
        );
    }

    /// A recording listed while it was still being made is as long as the
    /// furthest it has been played, and never shrinks back.
    #[test]
    fn a_recording_only_ever_gets_longer() {
        let mut history = History::default();
        history.opened(Watched {
            length_secs: 1_000,
            ..watched("1", 0)
        });
        assert!(history.progressed("1", 900.0, 1_500.0, false, 2));
        assert_eq!(history.get("1").unwrap().length_secs, 1_500);
        history.progressed("1", 950.0, 1_200.0, false, 3);
        assert_eq!(history.get("1").unwrap().length_secs, 1_500);
    }

    #[test]
    fn the_oldest_falls_off_the_end() {
        let mut history = History::default();
        for n in 0..(HISTORY_LIMIT + 5) {
            history.opened(watched(&n.to_string(), 0));
        }
        assert_eq!(history.videos.len(), HISTORY_LIMIT);
        assert_eq!(history.videos[0].id, (HISTORY_LIMIT + 4).to_string());
        assert!(history.get("0").is_none(), "the first one opened is gone");
    }

    /// A fresher listing brings the title and the picture, and leaves where
    /// the video was left, and when, alone.
    #[test]
    fn a_fresher_listing_updates_what_the_video_is_but_not_where_it_was_left() {
        let mut history = History::default();
        history.opened(Watched {
            thumbnail_url:
                "https://vod-secure.twitch.tv/_404/404_processing_%{width}x%{height}.png".into(),
            ..watched("1", 4_000)
        });

        let listed = Watched {
            title: "renamed".into(),
            thumbnail_url:
                "https://static-cdn.jtvnw.net/cf_vods/x/thumb/thumb0-%{width}x%{height}.jpg".into(),
            length_secs: 40_000,
            ..watched("1", 0)
        };
        assert!(history.refresh(&listed));
        let entry = history.get("1").unwrap();
        assert_eq!(entry.title, "renamed");
        assert!(entry.thumbnail_url.contains("thumb0"));
        assert_eq!(entry.length_secs, 40_000);
        assert_eq!(entry.position_secs, 4_000, "the history's own");
        assert!(!history.refresh(&listed), "nothing new the second time");
        assert!(
            !history.refresh(&watched("9", 0)),
            "a video the history does not have stays out of it"
        );
    }

    fn ids(history: &History) -> Vec<&str> {
        history.videos.iter().map(|w| w.id.as_str()).collect()
    }

    #[test]
    fn one_can_be_forgotten_or_all_of_them() {
        let mut history = History::default();
        history.opened(watched("1", 0));
        history.opened(watched("2", 0));
        let forgotten = history.forget("1");
        assert_eq!(forgotten.only().map(|w| w.id.as_str()), Some("1"));
        assert!(history.forget("1").is_empty(), "already gone");
        assert_eq!(history.clear().len(), 1);
        assert!(history.clear().is_empty(), "nothing left to clear");
        assert!(history.videos.is_empty());
    }

    /// Undo: one forgotten goes back where it stood, and so does a whole
    /// list, in its order, with where each was left.
    #[test]
    fn what_was_forgotten_goes_back_where_it_was() {
        let mut history = History::default();
        history.opened(watched("1", 100));
        history.opened(watched("2", 200));
        history.opened(watched("3", 300));
        assert_eq!(ids(&history), ["3", "2", "1"]);

        let forgotten = history.forget("2");
        assert_eq!(ids(&history), ["3", "1"]);
        assert!(history.restore(forgotten));
        assert_eq!(ids(&history), ["3", "2", "1"]);

        let cleared = history.clear();
        assert_eq!(cleared.len(), 3);
        assert_eq!(cleared.only(), None, "three, not one");
        assert!(history.restore(cleared));
        assert_eq!(ids(&history), ["3", "2", "1"]);
        assert_eq!(history.resume_at("2"), Some(200.0));
    }

    /// Opened again between the forgetting and the undo, a recording is
    /// already back with a newer place, and the undo leaves that alone
    /// while still putting back the rest.
    #[test]
    fn an_undo_keeps_what_was_opened_again_since() {
        let mut history = History::default();
        history.opened(watched("1", 4_000));
        history.opened(watched("2", 600));
        let cleared = history.clear();

        history.opened(watched("1", 4_500));
        assert!(history.restore(cleared));
        assert_eq!(history.resume_at("1"), Some(4_500.0), "the newer place");
        assert_eq!(ids(&history), ["2", "1"]);

        let forgotten = history.forget("2");
        history.opened(watched("2", 700));
        assert!(!history.restore(forgotten), "nothing to put back");
        assert_eq!(history.resume_at("2"), Some(700.0));
    }

    #[test]
    fn a_name_is_shown_as_the_channel_writes_it_or_by_login() {
        let named = watched("1", 0);
        assert_eq!(named.channel(), "Someone");
        let nameless = Watched {
            channel_name: String::new(),
            ..named
        };
        assert_eq!(nameless.channel(), "someone");
    }

    #[test]
    fn it_round_trips_through_disk_and_a_missing_file_is_empty() {
        let path = temp_file("history-round-trip");
        let _ = std::fs::remove_file(&path);
        assert_eq!(History::load(&path).unwrap(), History::default());

        let mut history = History::default();
        history.opened(watched("1", 4_000));
        history.save(&path).unwrap();
        assert_eq!(History::load(&path).unwrap(), history);
        let _ = std::fs::remove_file(&path);
    }

    /// Fields a newer build adds, or an older one never wrote, must not cost
    /// the rest of the list.
    #[test]
    fn a_file_from_another_build_still_loads() {
        let history: History = serde_json::from_str(
            r#"{"videos": [{"id": "1", "position_secs": 30, "someday": true}]}"#,
        )
        .unwrap();
        assert_eq!(history.resume_at("1"), Some(30.0));
        assert_eq!(history.videos[0].title, "");
    }

    #[test]
    fn it_lives_beside_the_settings() {
        let settings = crate::default_path("perch");
        let history = default_path("perch");
        assert_eq!(history.parent(), settings.parent());
        assert_eq!(history.file_name().unwrap(), "history.json");
    }
}
