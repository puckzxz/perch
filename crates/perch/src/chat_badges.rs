//! Chat badges, worked out: what Helix's answers are kept as, which picture
//! and title a badge on a line gets, and when the root asks. Pure and tested.
//! The asking is `root::badges`, the request is the worker's
//! `Request::Badges`, the data is `twitch_api::badges`, the names on a line
//! are `twitch_chat::message::Badge`, and the drawing is
//! `ChatView::badge_group`.
//!
//! A line names its badges by set and version and nothing more. What they
//! look like is two lists from Helix: the global sets, which every channel
//! shares, asked once a session; and each channel's own, its subscriber and
//! bits pictures, asked once per channel the first time a chat meets it (its
//! `ROOMSTATE` room id, or a recording's channel id), which stand in for the
//! global picture of the same set and version in that channel's chat
//! ([`ChatBadges::art`]). The answers live on the root for the session, as
//! [`Library`], and each chat mirrors the two that are its own as
//! [`ChatBadges`], the way the Shared Chat names are mirrored. Nothing here is
//! saved.
//!
//! Helix needs the session's token, so signed out nothing is asked and no
//! chat has any art: a line draws no badges and keeps no gap for them. A
//! badge Helix has no picture for is left out the same way. A failure is one
//! line in the log, and the next chat to meet that room (a pane opening, a
//! reconnect) asks again, as does sign-in completing.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::SharedString;
use twitch_api::badges::BadgeSet;
use twitch_chat::Badge;

/// What one badge version is drawn as: its picture's address, for the image
/// cache, and what the pointer is told it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BadgeArt {
    pub url: String,
    /// Helix's title (`Moderator`, `6-Month Subscriber`); see [`tooltip`]
    /// for what a line's months make of it.
    pub title: String,
}

/// One answer, by set and then version, for looking a line's badges up.
pub type Book = HashMap<String, HashMap<String, BadgeArt>>;

/// An answer as a [`Book`].
pub fn book(sets: Vec<BadgeSet>) -> Book {
    sets.into_iter()
        .map(|set| {
            let versions = set
                .versions
                .into_iter()
                .map(|version| {
                    (
                        version.id,
                        BadgeArt {
                            url: version.image_url,
                            title: version.title,
                        },
                    )
                })
                .collect();
            (set.set_id, versions)
        })
        .collect()
}

/// The badges one chat can draw: the global book and its own channel's,
/// shared with the root's [`Library`] rather than copied, since a global
/// book is a few hundred entries and every chat has it.
#[derive(Clone, Debug, Default)]
pub struct ChatBadges {
    global: Option<Arc<Book>>,
    channel: Option<Arc<Book>>,
}

impl ChatBadges {
    /// What `badge` is drawn as, if anything: the channel's own picture
    /// where it has one for that set and version, the global one where it
    /// does not, and nothing at all where neither does.
    pub fn art(&self, badge: &Badge) -> Option<&BadgeArt> {
        fn find<'a>(book: &'a Option<Arc<Book>>, badge: &Badge) -> Option<&'a BadgeArt> {
            book.as_ref()?.get(&badge.set)?.get(&badge.version)
        }
        find(&self.channel, badge).or_else(|| find(&self.global, badge))
    }

    /// Whether these are the very books `other` holds, so a chat handed
    /// what it already has need not remeasure its rows.
    pub fn same_as(&self, other: &ChatBadges) -> bool {
        let same = |a: &Option<Arc<Book>>, b: &Option<Arc<Book>>| match (a, b) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        same(&self.global, &other.global) && same(&self.channel, &other.channel)
    }
}

/// What the pointer is told a badge is: Helix's title, with the months a
/// line's `badge-info` gave for it after a comma, a sentence as every
/// tooltip is. A subscriber badge's own title already names a tier of
/// months (`6-Month Subscriber`), which the exact count makes wrong, so with
/// months it is simply `Subscriber, 14 months`. A title Helix left empty
/// falls back to the set's name.
pub fn tooltip(badge: &Badge, art: &BadgeArt) -> SharedString {
    let title = if art.title.is_empty() {
        badge.set.as_str()
    } else {
        art.title.as_str()
    };
    SharedString::from(match badge.months {
        Some(months) => {
            let title = if badge.set == "subscriber" {
                "Subscriber"
            } else {
                title
            };
            let unit = if months == 1 { "month" } else { "months" };
            format!("{title}, {months} {unit}")
        }
        None => title.to_string(),
    })
}

/// Every badge book asked for and answered this session, and the asks for
/// them: the global book (keyed `None`) and each channel's by its numeric
/// id. Lives on the root and is never saved.
#[derive(Debug, Default)]
pub struct Library {
    global: Option<Arc<Book>>,
    channels: HashMap<String, Arc<Book>>,
    /// Books with an ask out that the worker has not answered.
    out: HashSet<Option<String>>,
    /// Books a chat needed while no ask could go out (signed out, or the
    /// worker not yet taking asks), or whose ask failed: asked about when
    /// sign-in completes ([`take_met`](Self::take_met)), or the next time a
    /// chat needs them.
    met: HashSet<Option<String>>,
}

impl Library {
    /// What a chat in `room` (its numeric id, once it knows it) draws with.
    pub fn for_room(&self, room: Option<&str>) -> ChatBadges {
        ChatBadges {
            global: self.global.clone(),
            channel: room.and_then(|room| self.channels.get(room).cloned()),
        }
    }

    /// Whether `book` is worth asking the worker for now: one not answered
    /// and not already asked for.
    pub fn wants(&self, book: &Option<String>) -> bool {
        let known = match book {
            None => self.global.is_some(),
            Some(room) => self.channels.contains_key(room),
        };
        !known && !self.out.contains(book)
    }

    /// Note that an ask for `book` has gone to the worker.
    pub fn asked(&mut self, book: Option<String>) {
        self.met.remove(&book);
        self.out.insert(book);
    }

    /// Note that a chat needed `book` and it could not be asked for yet.
    pub fn met(&mut self, book: Option<String>) {
        self.met.insert(book);
    }

    /// The books needed while nothing could be asked that are still worth
    /// asking for, the global one first, for sign-in completing.
    pub fn take_met(&mut self) -> Vec<Option<String>> {
        let mut met: Vec<Option<String>> = std::mem::take(&mut self.met)
            .into_iter()
            .filter(|book| self.wants(book))
            .collect();
        met.sort();
        met
    }

    /// Take the worker's answer for `book`, and say what is worth a line in
    /// the log, if anything. A failure puts the book back among those met,
    /// so the next chat to need it, or the next sign-in, asks again.
    pub fn answered(
        &mut self,
        book: Option<String>,
        result: Result<Vec<BadgeSet>, String>,
    ) -> Option<String> {
        self.out.remove(&book);
        match result {
            Ok(sets) => {
                let answer = Arc::new(self::book(sets));
                match book {
                    None => self.global = Some(answer),
                    Some(room) => {
                        self.channels.insert(room, answer);
                    }
                }
                None
            }
            Err(reason) => {
                let which = match &book {
                    None => "the global badges".to_string(),
                    Some(room) => format!("channel {room}'s badges"),
                };
                self.met.insert(book);
                Some(format!("{which}: {reason}"))
            }
        }
    }

    /// Forget the asks out, for a worker that will never answer them: they
    /// are as if met with nobody to ask.
    pub fn forget(&mut self) {
        self.met.extend(self.out.drain());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use twitch_api::badges::BadgeVersion;

    fn set(id: &str, versions: &[(&str, &str)]) -> BadgeSet {
        BadgeSet {
            set_id: id.into(),
            versions: versions
                .iter()
                .map(|(version, title)| BadgeVersion {
                    id: (*version).into(),
                    title: (*title).into(),
                    image_url: format!("https://x.test/{id}/{version}"),
                })
                .collect(),
        }
    }

    fn badge(set: &str, version: &str, months: Option<u32>) -> Badge {
        Badge {
            set: set.into(),
            version: version.into(),
            months,
        }
    }

    /// A channel's own picture stands in for the global one of the same set
    /// and version; anything else falls through to the global book, and a
    /// badge neither has is nothing.
    #[test]
    fn a_channels_own_badge_wins_over_the_global_one() {
        let mut library = Library::default();
        library.answered(
            None,
            Ok(vec![
                set("subscriber", &[("0", "Subscriber")]),
                set("moderator", &[("1", "Moderator")]),
            ]),
        );
        library.answered(
            Some("1".into()),
            Ok(vec![set("subscriber", &[("0", "Channel Sub")])]),
        );

        let here = library.for_room(Some("1"));
        let sub = here.art(&badge("subscriber", "0", None)).unwrap();
        assert_eq!(sub.title, "Channel Sub");
        assert_eq!(sub.url, "https://x.test/subscriber/0");
        assert_eq!(
            here.art(&badge("moderator", "1", None)).unwrap().title,
            "Moderator"
        );
        assert!(here.art(&badge("moderator", "2", None)).is_none());
        assert!(here.art(&badge("vip", "1", None)).is_none());

        // Another room has only the global book.
        let elsewhere = library.for_room(Some("2"));
        assert_eq!(
            elsewhere
                .art(&badge("subscriber", "0", None))
                .unwrap()
                .title,
            "Subscriber"
        );
    }

    /// Signed out nothing is ever answered, and nothing is drawn.
    #[test]
    fn with_no_answers_there_is_no_art() {
        let none = Library::default().for_room(Some("1"));
        assert!(none.art(&badge("moderator", "1", None)).is_none());
        assert!(ChatBadges::default().same_as(&none));
    }

    #[test]
    fn a_tooltip_is_the_title_with_its_months() {
        let art = |title: &str| BadgeArt {
            url: String::new(),
            title: title.into(),
        };
        assert_eq!(
            tooltip(&badge("moderator", "1", None), &art("Moderator")).as_ref(),
            "Moderator"
        );
        // The tier's title gives way to the exact count.
        assert_eq!(
            tooltip(
                &badge("subscriber", "3012", Some(14)),
                &art("1-Year Subscriber")
            )
            .as_ref(),
            "Subscriber, 14 months"
        );
        assert_eq!(
            tooltip(&badge("subscriber", "0", Some(1)), &art("Subscriber")).as_ref(),
            "Subscriber, 1 month"
        );
        assert_eq!(
            tooltip(&badge("subscriber", "6", None), &art("6-Month Subscriber")).as_ref(),
            "6-Month Subscriber"
        );
        assert_eq!(
            tooltip(&badge("founder", "0", Some(3)), &art("Founder")).as_ref(),
            "Founder, 3 months"
        );
        assert_eq!(
            tooltip(&badge("glitchcon", "1", None), &art("")).as_ref(),
            "glitchcon"
        );
    }

    /// Each book is asked for once: not while its ask is out, and not once
    /// answered.
    #[test]
    fn each_book_is_asked_for_once() {
        let mut library = Library::default();
        assert!(library.wants(&None));
        library.asked(None);
        assert!(!library.wants(&None));
        assert!(library.wants(&Some("1".into())));
        library.answered(None, Ok(Vec::new()));
        assert!(!library.wants(&None), "an empty answer is still an answer");
    }

    /// A failure is a line in the log and is asked again: by the next chat
    /// that needs it, or at sign-in.
    #[test]
    fn a_failure_is_logged_and_asked_again() {
        let mut library = Library::default();
        library.asked(Some("7".into()));
        let log = library.answered(Some("7".into()), Err("HTTP 500".into()));
        assert_eq!(log.as_deref(), Some("channel 7's badges: HTTP 500"));
        assert!(library.wants(&Some("7".into())));
        assert_eq!(library.take_met(), [Some("7".to_string())]);
    }

    /// Books needed before sign-in are asked for after it, the global one
    /// first, once each; one asked for since is not handed out again.
    #[test]
    fn books_met_before_sign_in_are_asked_for_after() {
        let mut library = Library::default();
        library.met(Some("9".into()));
        library.met(None);
        library.met(Some("3".into()));
        library.met(Some("9".into()));
        library.asked(Some("3".into()));
        assert_eq!(library.take_met(), [None, Some("9".to_string())]);
        assert!(library.take_met().is_empty());
    }

    /// An ask a dead worker held is asked again at the next sign-in.
    #[test]
    fn a_forgotten_ask_is_asked_again() {
        let mut library = Library::default();
        library.asked(None);
        library.forget();
        assert!(library.wants(&None));
        assert_eq!(library.take_met(), [None]);
    }

    /// A chat handed the same books again need not remeasure; a new answer
    /// is a new book.
    #[test]
    fn the_same_books_are_recognised() {
        let mut library = Library::default();
        library.answered(None, Ok(Vec::new()));
        let before = library.for_room(Some("1"));
        assert!(before.same_as(&library.for_room(Some("1"))));
        library.answered(Some("1".into()), Ok(Vec::new()));
        assert!(!before.same_as(&library.for_room(Some("1"))));
    }
}
