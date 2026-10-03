//! A recording's muted stretches, as the root keeps them: on the pane's
//! video (`watch::Source::Video`), and from there on its player's seek bar
//! (`VideoView::muted`, a mirror, through `Start::muted` and `set_muted`).
//!
//! Most ways in carry them, since they come from Helix, which says each
//! video's `muted_segments`: a channel's page lists its videos, a stopped
//! pane and a rewind list a channel's broadcasts, and a link is looked up.
//! The root hears every one of those answers go by (`hear_muted`, beside
//! `refresh_history`, and in `on_linked_video`), and keeps what each said
//! of the stretches, for the session ([`Muted`]). The history does not
//! carry them: it keeps what a pane needs to play a recording again
//! (`history_page::video`), and the muted stretches are not that. So a pane
//! opening a recording with none takes them from what was heard, and only
//! one Helix has said nothing about this session is asked about, once, with
//! the request a link is looked up by (`Request::Video`). Whatever comes
//! back goes onto every pane playing that video. Signed out, nothing is
//! asked and the bar marks nothing, until sign-in lands and asks for every
//! pane still without word (`ask_muted_for_open`).
//!
//! A link and a pane's question share the request and so the answer
//! (`TwitchEvent::Video`), which says only the id. The worker answers in
//! order, so whichever was sent first is answered first: a pane does not ask
//! about a recording a link is waiting on, since the link's answer brings
//! the stretches, and an answer for an id a pane is asking about is the
//! pane's ([`Muted::claim`]), leaving any link sent after it for its own.

use std::collections::{HashMap, HashSet};

use twitch_api::{MutedSegment, Video};

use super::RootView;
use crate::browse::SignIn;
use crate::twitch::Request;
use crate::watch::Source;

/// What the root knows of recordings' muted stretches this session, and
/// which it is asking about. Never saved: a listing is the newer word on a
/// recording, and Twitch mutes an archive after the broadcast.
#[derive(Default)]
pub(super) struct Muted {
    /// Each recording Helix has said anything about this session, by id,
    /// with its stretches, which are most often none.
    heard: HashMap<String, Vec<MutedSegment>>,
    /// The recordings a pane has asked about whose answer has not come.
    asking: HashSet<String>,
}

impl Muted {
    /// Note what Helix said of `video`'s stretches.
    fn hear(&mut self, video: &Video) {
        self.heard
            .insert(video.id.clone(), video.muted_segments.clone());
    }

    /// Give `video` the stretches heard for it, if it came with none:
    /// whether anything is known of them, so whether there is no need to
    /// ask. A video that came with stretches knows them.
    fn fill(&self, video: &mut Video) -> bool {
        if !video.muted_segments.is_empty() {
            return true;
        }
        match self.heard.get(&video.id) {
            Some(heard) => {
                video.muted_segments = heard.clone();
                true
            }
            None => false,
        }
    }

    /// Whether the answer about `id` is a pane's question's, which it is
    /// while one is out, the worker answering in order; taking it off the
    /// questions out if so.
    fn claim(&mut self, id: &str) -> bool {
        self.asking.remove(id)
    }

    /// Forget the questions out, whose answers die with the worker they
    /// were sent to; the next sign-in asks again (`ask_muted_for_open`).
    pub(super) fn forget_asks(&mut self) {
        self.asking.clear();
    }
}

impl RootView {
    /// Note what Helix said of these recordings' stretches, for a pane that
    /// opens one later from the history; see the module docs.
    pub(super) fn hear_muted(&mut self, videos: &[Video]) {
        for video in videos {
            self.muted.hear(video);
        }
    }

    /// `video` as a pane opens it: with the stretches heard for it if it
    /// came with none, and asked about if nothing is known of them yet.
    pub(super) fn with_muted(&mut self, mut video: Video) -> Video {
        if !self.muted.fill(&mut video) {
            self.ask_muted(&video.id);
        }
        video
    }

    /// Ask Twitch about recording `id` for its muted stretches; see the
    /// module docs. Not while signed out or still signing in, where the
    /// request would wait behind the sign-in (`ask_muted_for_open` asks once
    /// it lands); not while a question about it is already out; and not for
    /// a recording a link is waiting on, whose answer brings them.
    fn ask_muted(&mut self, id: &str) {
        if !matches!(self.sign_in, SignIn::SignedIn(_))
            || self.muted.asking.contains(id)
            || self.linked_videos.iter().any(|linked| linked.id == id)
        {
            return;
        }
        if self.twitch.request(Request::Video { id: id.to_string() }) {
            self.muted.asking.insert(id.to_string());
        }
    }

    /// Ask about every open recording nothing is known of: what sign-in
    /// landing calls, for the panes opened before it.
    pub(super) fn ask_muted_for_open(&mut self) {
        let unknown: Vec<String> = self
            .slots
            .iter()
            .filter_map(|slot| slot.recording())
            .filter(|video| {
                video.muted_segments.is_empty() && !self.muted.heard.contains_key(&video.id)
            })
            .map(|video| video.id.clone())
            .collect();
        for id in unknown {
            self.ask_muted(&id);
        }
    }

    /// Whether the worker's answer about recording `id` is a pane's
    /// question's rather than a link's; see [`Muted::claim`].
    pub(super) fn claim_muted(&mut self, id: &str) -> bool {
        self.muted.claim(id)
    }

    /// The worker's word on `video`, whoever asked: what it says of the
    /// stretches is heard, and goes onto every pane playing it, and to each
    /// one's player, which marks them on its seek bar from the next frame. A
    /// pane whose player is still starting takes them from its video when
    /// the player is made.
    pub(super) fn take_muted(&mut self, video: &Video, cx: &mut gpui::Context<Self>) {
        self.muted.hear(video);
        for slot in &mut self.slots {
            let Source::Video { video: playing, .. } = &mut slot.source else {
                continue;
            };
            if playing.id != video.id || playing.muted_segments == video.muted_segments {
                continue;
            }
            playing.muted_segments = video.muted_segments.clone();
            if let Some(view) = slot.video() {
                let muted = video.muted_segments.clone();
                view.update(cx, |view, cx| view.set_muted(muted, cx));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use twitch_api::VideoKind;

    fn video(id: &str, muted: &[(u64, u64)]) -> Video {
        Video {
            id: id.into(),
            stream_id: None,
            user_id: "1".into(),
            user_login: "someone".into(),
            user_name: "Someone".into(),
            title: String::new(),
            created_at: String::new(),
            length_secs: 3600,
            thumbnail_url: String::new(),
            view_count: 0,
            kind: VideoKind::Archive,
            muted_segments: muted
                .iter()
                .map(|&(offset_secs, duration_secs)| MutedSegment {
                    offset_secs,
                    duration_secs,
                })
                .collect(),
        }
    }

    #[test]
    fn a_recording_from_the_history_takes_what_a_listing_said() {
        let mut muted = Muted::default();
        muted.hear(&video("1", &[(60, 30)]));
        let mut from_history = video("1", &[]);
        assert!(muted.fill(&mut from_history));
        assert_eq!(
            from_history.muted_segments,
            video("1", &[(60, 30)]).muted_segments
        );
    }

    #[test]
    fn a_listing_that_said_none_is_still_word_on_it() {
        let mut muted = Muted::default();
        muted.hear(&video("1", &[]));
        let mut from_history = video("1", &[]);
        assert!(muted.fill(&mut from_history), "nothing to ask");
        assert!(from_history.muted_segments.is_empty());
    }

    #[test]
    fn only_a_recording_nobody_has_said_anything_about_is_asked_about() {
        let mut muted = Muted::default();
        muted.hear(&video("1", &[]));
        assert!(!muted.fill(&mut video("2", &[])));
        // One that came with its stretches knows them, heard or not.
        assert!(muted.fill(&mut video("3", &[(0, 10)])));
    }

    #[test]
    fn an_answer_is_a_panes_once_and_then_a_links() {
        let mut muted = Muted::default();
        muted.asking.insert("1".into());
        assert!(muted.claim("1"));
        assert!(!muted.claim("1"), "the next answer is the link's");
        assert!(!muted.claim("2"));
    }
}
