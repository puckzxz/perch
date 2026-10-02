//! Shared Chat labels, worked out: which partner channels are worth asking
//! Twitch about, what the answers are kept as, and what a copied line says
//! about where it came from. Pure and tested. The asking is
//! `root::shared_chat`, the request is the worker's `Request::ChannelNames`,
//! the data is `twitch_api::recommend::channel_names`, and the drawing is
//! `ChatView::source_tag`.
//!
//! In a Shared Chat session up to a handful of channels share one chat: what
//! is said in any of them is copied into the others, and the copy carries the
//! numeric id of the room it was really said in
//! (`twitch_chat::ChatMessage::source_room`). Shown as it arrives, a copy
//! reads as if it were said in the pane's own channel, to a streamer who
//! never saw it. So a copy wears a small tag before the speaker's name naming
//! the channel it came from, with `Said in <name>'s chat` under the pointer.
//! A copied event (a partner's sub, gift or raid) carries the id on the
//! notice itself (`twitch_chat::ChatNotice::source_room`), and wears the tag
//! once, on its first line: Twitch's sentence, which is the line that would
//! otherwise read as if it happened here, or its note when it has none.
//!
//! An id is all the line carries, so the name has to be asked for. Each id is
//! asked about once a session, the first time a chat meets it, and the answer
//! is kept for every chat, so a partner met in one pane is named in all of
//! them and in panes opened later. One met before the worker can take asks
//! (a pane opened while sign-in is still under way loads its history, copies
//! and all) is remembered and asked about once sign-in completes, rather than
//! waiting for that partner to speak again. A failure is one line in the log
//! and no label on that partner's lines for the rest of the session; a refusal,
//! Twitch declining to run the query at all, is one line too, and then no
//! more asking. Until a name is known a copy wears nothing: a placeholder
//! would have to be taken off again if the ask failed, and a line saying
//! less than it could is better than one that changes under the reader for
//! nothing. Nothing here is saved.
//!
//! Live chat only. A replay's comments carry no source room, so a recording
//! of a shared session reads as it always did.

use std::collections::{HashMap, HashSet};

use gpui::SharedString;
use twitch_api::Channel;

use crate::twitch::RecommendError;

/// What a copied line says about where it came from: the tag's word and the
/// sentence under the pointer. Made once per partner, when its name arrives
/// ([`Label::of`]), rather than on every repaint of every copied row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    /// The channel's display name, as its own chat writes it.
    pub name: SharedString,
    /// `Said in <name>'s chat`, a sentence as every tooltip is.
    pub tooltip: SharedString,
}

impl Label {
    /// The label for lines copied from `channel`.
    pub fn of(channel: &Channel) -> Self {
        let name = if channel.display_name.is_empty() {
            &channel.login
        } else {
            &channel.display_name
        };
        Self {
            name: SharedString::from(name.clone()),
            tooltip: SharedString::from(format!("Said in {name}'s chat")),
        }
    }
}

/// Whether a line is labelled, and with what: only a copy (`source_room`
/// set, which `twitch_chat` does only when the room differs from the chat's
/// own) from a partner whose name is known. Everything else, a line said
/// here included, wears nothing.
pub fn label<'a>(
    source_room: Option<&str>,
    labels: &'a HashMap<String, Label>,
) -> Option<&'a Label> {
    labels.get(source_room?)
}

/// The asks out and the names in, for every chat. Lives on the root for the
/// session and is never saved.
#[derive(Debug, Default)]
pub struct SourceRooms {
    /// Each partner's label, by its numeric id.
    labels: HashMap<String, Label>,
    /// Ids with an ask out that the worker has not answered yet.
    out: HashSet<String>,
    /// Ids a chat met while no ask could go out: before sign-in completed,
    /// or with no worker to take it. Asked about when sign-in completes
    /// ([`take_met`](Self::take_met)).
    met: HashSet<String>,
    /// Ids asked about and answered, named or not: a channel Twitch has no
    /// account for, or one whose ask failed, is not asked about again.
    settled: HashSet<String>,
    /// Twitch would not run the query (`RecommendError::Refused`), so nothing
    /// more is asked this session.
    refused: bool,
}

impl SourceRooms {
    /// Every name known, by id: what a chat opened now starts with.
    pub fn labels(&self) -> &HashMap<String, Label> {
        &self.labels
    }

    /// Whether `id` is worth asking the worker about now: one nobody has
    /// named, asked about, or given up on, and nothing at all once Twitch
    /// has refused the query. So a chat can say every time it meets an id it
    /// cannot name, and the worker hears of each one once.
    pub fn wants(&self, id: &str) -> bool {
        !self.refused
            && !self.labels.contains_key(id)
            && !self.out.contains(id)
            && !self.settled.contains(id)
    }

    /// Note that an ask for `id` has gone to the worker. Only once it has:
    /// an ask nobody could take is not one to wait on, and the next line
    /// from that room asks again.
    pub fn asked(&mut self, id: &str) {
        self.met.remove(id);
        self.out.insert(id.to_string());
    }

    /// Note that a chat met `id` and it could not be asked about yet, so it
    /// is asked about once it can be.
    pub fn met(&mut self, id: &str) {
        self.met.insert(id.to_string());
    }

    /// The ids met while nothing could be asked that are still worth asking
    /// about, for sign-in completing: each is either asked about now, and
    /// noted by [`asked`](Self::asked), or met again.
    pub fn take_met(&mut self) -> Vec<String> {
        let mut met: Vec<String> = std::mem::take(&mut self.met)
            .into_iter()
            .filter(|id| self.wants(id))
            .collect();
        met.sort();
        met
    }

    /// Take the worker's answer for `ids`: the labels it brought, for the
    /// chats to learn, and what is worth a line in the log, if anything.
    ///
    /// An id the answer left out, a channel Twitch has no account for, is
    /// settled unnamed. A failure settles its ids unnamed too, so a partner
    /// whose name could not be had is not asked about at every line it
    /// sends; one line in the log says so, once for the ask. A refusal is a
    /// line once and the end of asking, as it is for the rail's
    /// (`recommended::Recommended::answered`).
    pub fn answered(
        &mut self,
        ids: &[String],
        result: Result<Vec<Channel>, RecommendError>,
    ) -> (Vec<(String, Label)>, Option<String>) {
        for id in ids {
            self.out.remove(id);
            self.settled.insert(id.clone());
        }
        match result {
            Ok(channels) => {
                let learned: Vec<(String, Label)> = channels
                    .iter()
                    .filter(|channel| ids.contains(&channel.user_id))
                    .map(|channel| (channel.user_id.clone(), Label::of(channel)))
                    .collect();
                self.labels.extend(learned.iter().cloned());
                (learned, None)
            }
            Err(RecommendError::Refused(message)) => {
                if self.refused {
                    return (Vec::new(), None);
                }
                self.refused = true;
                (
                    Vec::new(),
                    Some(format!(
                        "Twitch would not run the query ({message}); \
                         not asking again this session"
                    )),
                )
            }
            Err(RecommendError::Failed(message)) => (
                Vec::new(),
                Some(format!("{message}; {} left unlabelled", ids.join(", "))),
            ),
        }
    }

    /// Forget the asks out, for a worker that will never answer them: one
    /// replaced by a new client id, or one stopped because sign-in failed.
    /// They are as if never made, and as if met with nobody to ask: the next
    /// sign-in asks about them, as does the next line from those rooms.
    pub fn forget(&mut self) {
        self.met.extend(self.out.drain());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(id: &str, login: &str, display_name: &str) -> Channel {
        Channel {
            login: login.into(),
            user_id: id.into(),
            display_name: display_name.into(),
        }
    }

    #[test]
    fn a_label_names_the_channel_as_its_chat_writes_it() {
        let label = Label::of(&channel("12826", "twitch", "Twitch"));
        assert_eq!(label.name.as_ref(), "Twitch");
        assert_eq!(label.tooltip.as_ref(), "Said in Twitch's chat");

        // No display name to hand: the login stands in.
        let label = Label::of(&channel("1", "quiet", ""));
        assert_eq!(label.name.as_ref(), "quiet");
        assert_eq!(label.tooltip.as_ref(), "Said in quiet's chat");
    }

    /// Only a copy from a partner whose name is known is labelled.
    #[test]
    fn only_a_named_copy_is_labelled() {
        let labels: HashMap<String, Label> = [(
            "12826".to_string(),
            Label::of(&channel("12826", "twitch", "Twitch")),
        )]
        .into();
        assert_eq!(
            label(Some("12826"), &labels).map(|l| l.name.as_ref()),
            Some("Twitch")
        );
        // Said here, shared or not.
        assert_eq!(label(None, &labels), None);
        // A copy from a partner not named yet, or never.
        assert_eq!(label(Some("999"), &labels), None);
        assert_eq!(label(Some("12826"), &HashMap::new()), None);
    }

    /// Each id is asked about once: not again while its ask is out, and not
    /// again once answered, named or not.
    #[test]
    fn each_partner_is_asked_about_once() {
        let mut rooms = SourceRooms::default();
        assert!(rooms.wants("12826"));
        rooms.asked("12826");
        assert!(!rooms.wants("12826"), "an ask is out");
        assert!(rooms.wants("141981764"), "another partner is its own ask");

        let ids = vec!["12826".to_string(), "404".to_string()];
        rooms.asked("404");
        let (learned, log) = rooms.answered(&ids, Ok(vec![channel("12826", "twitch", "Twitch")]));
        assert_eq!(log, None);
        assert_eq!(learned.len(), 1);
        assert_eq!(learned[0].0, "12826");
        assert_eq!(learned[0].1.name.as_ref(), "Twitch");
        assert_eq!(rooms.labels()["12826"].name.as_ref(), "Twitch");

        // Named, and one with no account: neither is asked about again.
        assert!(!rooms.wants("12826"));
        assert!(!rooms.wants("404"));
        assert!(!rooms.labels().contains_key("404"));
    }

    /// An answer naming a channel nobody asked about teaches nothing: the
    /// ids asked are what the answer is for.
    #[test]
    fn an_answer_names_only_what_was_asked() {
        let mut rooms = SourceRooms::default();
        rooms.asked("1");
        let (learned, _) = rooms.answered(
            &["1".to_string()],
            Ok(vec![channel("1", "one", "One"), channel("2", "two", "Two")]),
        );
        assert_eq!(learned.len(), 1);
        assert!(!rooms.labels().contains_key("2"));
    }

    /// A failure is a line in the log, once for the ask, and the partner
    /// goes unlabelled for the session rather than being asked about at
    /// every line it sends.
    #[test]
    fn a_failure_is_logged_once_and_left_unlabelled() {
        let mut rooms = SourceRooms::default();
        rooms.asked("12826");
        let (learned, log) = rooms.answered(
            &["12826".to_string()],
            Err(RecommendError::Failed("timed out".into())),
        );
        assert!(learned.is_empty());
        assert_eq!(log.as_deref(), Some("timed out; 12826 left unlabelled"));
        assert!(!rooms.wants("12826"));
        assert!(rooms.wants("141981764"), "a failure ends nothing else");
    }

    /// A refusal is a line once, and then nobody is asked about again.
    #[test]
    fn a_refusal_ends_the_asking() {
        let mut rooms = SourceRooms::default();
        rooms.asked("1");
        let (_, log) = rooms.answered(
            &["1".to_string()],
            Err(RecommendError::Refused("failed integrity check".into())),
        );
        assert!(log.unwrap().contains("failed integrity check"));
        assert!(!rooms.wants("2"));
        // A second refusal, from an ask that was already out, says nothing.
        let (_, log) = rooms.answered(
            &["3".to_string()],
            Err(RecommendError::Refused("again".into())),
        );
        assert_eq!(log, None);
    }

    /// An ask a worker died holding is as if never made, and is made again
    /// at the next sign-in.
    #[test]
    fn a_forgotten_ask_is_asked_again() {
        let mut rooms = SourceRooms::default();
        rooms.asked("12826");
        rooms.forget();
        assert!(rooms.wants("12826"));
        assert_eq!(rooms.take_met(), vec!["12826".to_string()]);
    }

    /// A partner met before sign-in completed is still asked about once it
    /// has, without waiting for it to speak again; one asked about or
    /// settled since is not, and nothing is handed out twice.
    #[test]
    fn a_partner_met_before_sign_in_is_asked_about_after() {
        let mut rooms = SourceRooms::default();
        rooms.met("12826");
        rooms.met("12826");
        rooms.met("404");
        rooms.met("7");
        // Met again by a line that did get asked about.
        rooms.asked("7");
        rooms.answered(&["404".to_string()], Ok(Vec::new()));

        assert!(rooms.wants("12826"));
        assert_eq!(rooms.take_met(), vec!["12826".to_string()]);
        assert!(rooms.take_met().is_empty(), "taken once");
    }
}
