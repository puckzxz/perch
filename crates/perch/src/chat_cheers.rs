//! Cheers, worked out: what Helix's cheermotes are kept as, which picture a
//! word in a line that spent Bits is drawn as, and when the root asks. Pure
//! and tested. The asking is `root::cheers`, the request is the worker's
//! `Request::Cheermotes`, the data is `twitch_api::cheermotes`, the Bits on
//! a line are `twitch_chat::ChatMessage::bits`, the budget the words spend
//! is `emotes::apply_cheers`, and the drawing is `chat/cheers.rs`.
//!
//! A line that cheered says how many Bits in its `bits` tag, and its text
//! says which words: `Cheer100`, a cheermote's prefix in any case with an
//! amount after it. What each is drawn as comes from Helix, asked once per
//! room the first time a live chat meets it (its `ROOMSTATE` room id); one
//! answer holds the global cheermotes and that channel's own. The answers
//! live on the root for the session, as [`Library`], and each live chat
//! mirrors its room's as [`ChatCheers`], as it does its badges. Nothing here
//! is saved.
//!
//! Helix needs the session's token, so nothing is asked signed out, and
//! until a room's answer has come every cheer in it is the word that was
//! typed, as it always is in a recording's chat replay, whose comments do
//! not say who spent Bits, and so is never asked for. An answer outlives a
//! sign-out, as the badges' do: a room answered while signed in goes on
//! drawing its cheers after it, and only a room first met signed out keeps
//! them words. A failure is one line in the log, and the next `ROOMSTATE`
//! of a chat in that room asks again, as does sign-in completing.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use emotes::Cheer;
use twitch_api::cheermotes::Cheermote;

/// One answer, by prefix in lowercase, for looking a word up.
pub type Book = HashMap<String, Cheermote>;

/// An answer as a [`Book`]. Should two cheermotes share a prefix, the first
/// in Helix's order wins, which is the order its Bits card shows them in.
pub fn book(cheermotes: Vec<Cheermote>) -> Book {
    let mut book = Book::new();
    for cheermote in cheermotes {
        book.entry(cheermote.prefix.to_ascii_lowercase())
            .or_insert(cheermote);
    }
    book
}

/// The cheers one chat can draw: its room's book, shared with the root's
/// [`Library`] rather than copied. `None` until the answer comes, which
/// takes a sign-in, and always in a replay.
#[derive(Clone, Debug, Default)]
pub struct ChatCheers {
    book: Option<Arc<Book>>,
}

impl ChatCheers {
    /// What `word` is as a cheer, if it is one: a cheermote's prefix, in any
    /// case, then an amount of Bits that does not start with a zero, as the
    /// whole word (`Cheer100`, `cheer100`, `4Head5`), drawn as the highest
    /// tier the amount reaches. Chatterino's rule, a case-blind
    /// `^prefix([1-9][0-9]*)$` per prefix.
    ///
    /// Where the amount starts is not known from the word alone, since a
    /// prefix may end in a digit, so each place in the digits at its end
    /// that could start one is tried, the longest amount first: `Cheer100`
    /// is `Cheer` and 100, and with `Party2` in the book and no `Party`,
    /// `Party2100` is `Party2` and 100. A word past what a `u32` holds, or
    /// with no tier its amount reaches, is no cheer.
    ///
    /// How much the line may spend is not this lookup's to say: see
    /// `emotes::apply_cheers`.
    pub fn find(&self, word: &str) -> Option<Cheer> {
        let book = self.book.as_ref()?;
        let prefix = word.trim_end_matches(|c: char| c.is_ascii_digit());
        // Byte offsets are char offsets in the digits, which are ASCII.
        (prefix.len()..word.len())
            .filter(|&start| start > 0 && !word[start..].starts_with('0'))
            .find_map(|start| {
                let amount: u32 = word[start..].parse().ok()?;
                let cheermote = book.get(&word[..start].to_ascii_lowercase())?;
                let tier = cheermote
                    .tiers
                    .iter()
                    .rev()
                    .find(|tier| tier.min_bits <= amount)?;
                Some(Cheer {
                    name: word.to_string(),
                    amount,
                    url: tier.image_url.clone(),
                    color: tier.color,
                })
            })
    }

    /// Whether this is the very book `other` holds, so a chat handed what
    /// it already has need not remeasure its rows.
    pub fn same_as(&self, other: &ChatCheers) -> bool {
        match (&self.book, &other.book) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

/// Every room's cheermotes asked for and answered this session, by the
/// room's numeric id, and the asks for them. Lives on the root and is never
/// saved.
#[derive(Debug, Default)]
pub struct Library {
    rooms: HashMap<String, Arc<Book>>,
    /// Rooms with an ask out that the worker has not answered.
    out: HashSet<String>,
    /// Rooms a chat met while no ask could go out (signed out, or the
    /// worker not yet taking asks), or whose ask failed: asked about when
    /// sign-in completes ([`take_met`](Self::take_met)), or the next time a
    /// chat in the room says it is there.
    met: HashSet<String>,
}

impl Library {
    /// What a live chat in `room` draws its cheers with.
    pub fn for_room(&self, room: &str) -> ChatCheers {
        ChatCheers {
            book: self.rooms.get(room).cloned(),
        }
    }

    /// Whether `room`'s cheermotes are worth asking the worker for now: not
    /// answered and not already asked for.
    pub fn wants(&self, room: &str) -> bool {
        !self.rooms.contains_key(room) && !self.out.contains(room)
    }

    /// Note that an ask for `room` has gone to the worker.
    pub fn asked(&mut self, room: String) {
        self.met.remove(&room);
        self.out.insert(room);
    }

    /// Note that a chat met `room` and it could not be asked for yet.
    pub fn met(&mut self, room: String) {
        self.met.insert(room);
    }

    /// The rooms met while nothing could be asked that are still worth
    /// asking for, in order, for sign-in completing.
    pub fn take_met(&mut self) -> Vec<String> {
        let mut met: Vec<String> = std::mem::take(&mut self.met)
            .into_iter()
            .filter(|room| self.wants(room))
            .collect();
        met.sort();
        met
    }

    /// Take the worker's answer for `room`, and say what is worth a line in
    /// the log, if anything. A failure puts the room back among those met,
    /// so the next chat to say it is there, or the next sign-in, asks again.
    pub fn answered(
        &mut self,
        room: String,
        result: Result<Vec<Cheermote>, String>,
    ) -> Option<String> {
        self.out.remove(&room);
        match result {
            Ok(cheermotes) => {
                self.rooms.insert(room, Arc::new(book(cheermotes)));
                None
            }
            Err(reason) => {
                let line = format!("channel {room}'s cheermotes: {reason}");
                self.met.insert(room);
                Some(line)
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
    use twitch_api::cheermotes::CheerTier;

    /// A cheermote with a tier at each least amount, each with a picture
    /// named for the prefix and the amount.
    fn cheermote(prefix: &str, tiers: &[u32]) -> Cheermote {
        Cheermote {
            prefix: prefix.into(),
            tiers: tiers
                .iter()
                .map(|&min_bits| CheerTier {
                    min_bits,
                    color: min_bits,
                    image_url: format!("https://x.test/{prefix}/{min_bits}"),
                })
                .collect(),
        }
    }

    /// The cheers of a room whose answer was `cheermotes`.
    fn answered(cheermotes: Vec<Cheermote>) -> ChatCheers {
        let mut library = Library::default();
        library.answered("1".into(), Ok(cheermotes));
        library.for_room("1")
    }

    fn global() -> ChatCheers {
        answered(vec![
            cheermote("Cheer", &[1, 100, 1000, 5000, 10000, 100000]),
            cheermote("4Head", &[1, 100]),
        ])
    }

    #[test]
    fn a_cheer_is_its_prefix_in_any_case_and_an_amount() {
        let cheers = global();
        for word in ["Cheer100", "cheer100", "CHEER100", "cHeEr100"] {
            let cheer = cheers.find(word).unwrap();
            assert_eq!(cheer.name, word, "the word as sent");
            assert_eq!(cheer.amount, 100);
            assert_eq!(cheer.url, "https://x.test/Cheer/100");
            assert_eq!(cheer.color, 100);
        }
    }

    #[test]
    fn the_amount_picks_the_highest_tier_it_reaches() {
        let cheers = global();
        let tier = |word: &str| cheers.find(word).unwrap().url;
        assert_eq!(tier("Cheer1"), "https://x.test/Cheer/1");
        assert_eq!(tier("Cheer99"), "https://x.test/Cheer/1");
        assert_eq!(tier("Cheer100"), "https://x.test/Cheer/100");
        assert_eq!(tier("Cheer4999"), "https://x.test/Cheer/1000");
        assert_eq!(tier("Cheer100000"), "https://x.test/Cheer/100000");
        assert_eq!(tier("Cheer250000"), "https://x.test/Cheer/100000");
        // An amount under every tier is no cheer.
        assert!(answered(vec![cheermote("Big", &[100])])
            .find("Big50")
            .is_none());
    }

    #[test]
    fn a_word_that_is_not_a_cheer_is_not_one() {
        let cheers = global();
        for word in [
            "Cheer",
            "Cheer0",
            "Cheer0100",
            "Cheer100!",
            "Cheer99999999999",
            "Nope100",
            "100",
            "xCheer100",
            "",
        ] {
            assert!(cheers.find(word).is_none(), "{word}");
        }
    }

    /// A prefix that starts with a digit is still its own: the amount is
    /// only the digits at the word's end.
    #[test]
    fn a_prefix_may_start_with_a_digit() {
        let cheer = global().find("4Head5").unwrap();
        assert_eq!(cheer.amount, 5);
        assert_eq!(cheer.url, "https://x.test/4Head/1");
    }

    /// A prefix that ends in one takes the digits after it as the amount,
    /// where no shorter prefix in the book takes them all.
    #[test]
    fn a_prefix_may_end_with_a_digit() {
        let cheers = answered(vec![cheermote("Party2", &[1])]);
        assert_eq!(cheers.find("Party2100").unwrap().amount, 100);
        assert!(cheers.find("Party2").is_none());
        // `Party2` and `0100` is not a cheer, and `Party20` and `100` is
        // no prefix.
        assert!(cheers.find("Party20100").is_none());
    }

    /// With no book, before the answer (or a sign-in to ask with) and in a
    /// replay, nothing is a cheer.
    #[test]
    fn with_no_book_nothing_is_a_cheer() {
        assert!(ChatCheers::default().find("Cheer100").is_none());
        assert!(Library::default().for_room("1").find("Cheer100").is_none());
        assert!(ChatCheers::default().same_as(&Library::default().for_room("1")));
    }

    #[test]
    fn the_first_cheermote_of_a_prefix_wins() {
        let cheers = answered(vec![cheermote("Cheer", &[1]), cheermote("cheer", &[5])]);
        assert_eq!(cheers.find("Cheer9").unwrap().url, "https://x.test/Cheer/1");
    }

    /// Each room is asked for once: not while its ask is out, and not once
    /// answered.
    #[test]
    fn each_room_is_asked_for_once() {
        let mut library = Library::default();
        assert!(library.wants("1"));
        library.asked("1".into());
        assert!(!library.wants("1"));
        assert!(library.wants("2"));
        library.answered("1".into(), Ok(Vec::new()));
        assert!(!library.wants("1"), "an empty answer is still an answer");
    }

    /// A failure is a line in the log and is asked again: by the next chat
    /// that says it is in the room, or at sign-in.
    #[test]
    fn a_failure_is_logged_and_asked_again() {
        let mut library = Library::default();
        library.asked("7".into());
        let log = library.answered("7".into(), Err("HTTP 500".into()));
        assert_eq!(log.as_deref(), Some("channel 7's cheermotes: HTTP 500"));
        assert!(library.wants("7"));
        assert_eq!(library.take_met(), ["7"]);
    }

    /// Rooms met before sign-in are asked for after it, once each; one asked
    /// for since is not handed out again.
    #[test]
    fn rooms_met_before_sign_in_are_asked_for_after() {
        let mut library = Library::default();
        library.met("9".into());
        library.met("3".into());
        library.met("9".into());
        library.met("5".into());
        library.asked("3".into());
        assert_eq!(library.take_met(), ["5", "9"]);
        assert!(library.take_met().is_empty());
    }

    /// An ask a dead worker held is asked again at the next sign-in.
    #[test]
    fn a_forgotten_ask_is_asked_again() {
        let mut library = Library::default();
        library.asked("1".into());
        library.forget();
        assert!(library.wants("1"));
        assert_eq!(library.take_met(), ["1"]);
    }

    /// A chat handed the same book again need not remeasure; a new answer
    /// is a new book.
    #[test]
    fn the_same_book_is_recognised() {
        let mut library = Library::default();
        library.answered("1".into(), Ok(Vec::new()));
        let before = library.for_room("1");
        assert!(before.same_as(&library.for_room("1")));
        assert!(!before.same_as(&library.for_room("2")));
        library.answered("1".into(), Ok(Vec::new()));
        assert!(!before.same_as(&library.for_room("1")));
    }
}
