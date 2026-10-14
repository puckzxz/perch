//! Who a chatter is, worked out: the accounts Helix has answered with for
//! the chats' viewer cards, which logins to ask about and when, and what a
//! card says of an account. Pure and tested. The asking is
//! `root::viewer_cards`, the request is the worker's `Request::Profile`, the
//! data is `twitch_api::users`, and the drawing is `chat/viewer_card.rs`.
//!
//! A press on a name opens that person's card, and what it shows beyond
//! their lines — their picture, how old their account is, whether their
//! channel is a partner's or an affiliate's — is in no chat line, so it is
//! asked of Helix when a card for them opens, once per login a session. The
//! answers live on the root as [`Profiles`], the newest [`MAX_PROFILES`]
//! kept, and the card open in each chat is told what is known of its person
//! as a [`Lookup`], the way a composer is told its `Access`. Nothing here is
//! saved.
//!
//! Helix needs the session's token, so signed out nothing is asked and a
//! card shows what its chat already knows: the names and the lines. An
//! answer outlives a sign-out, as the badges' books do. A failure is one
//! line in the log, and is asked again only when that person's card is
//! opened afresh ([`Profiles::reopened`]), never by the sync that tells the
//! open cards after every worker event, which would ask in a loop.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use chrono::{DateTime, Datelike, Local, NaiveDate, Utc};
use twitch_api::users::{BroadcasterType, User};

/// How many answers the root keeps. A session of channel-hopping meets a
/// great many names, and a card is opened for few of them; this is far more
/// than anyone opens in an evening, and an account let go is asked about
/// again if its card is opened after.
pub const MAX_PROFILES: usize = 256;

/// The picture's size at a scale below [`LARGE_AVATAR_FROM`], in pixels
/// square: the smallest Twitch serves that is still crisp at the card's
/// `theme::VIEWER_CARD_AVATAR`, as the rail asks for its smaller one.
const AVATAR_SMALL: u32 = 70;

/// The picture's size at [`LARGE_AVATAR_FROM`] and above, where the card's
/// 40 points are 60 pixels or more: the next size Twitch serves.
const AVATAR_LARGE: u32 = 150;

/// The screen scale from which the larger picture is asked for.
const LARGE_AVATAR_FROM: f32 = 1.5;

/// What a card is told about its person's account.
#[derive(Clone, Debug)]
pub enum Lookup {
    /// Nobody is signed in and nothing was known before: no picture and
    /// no account line, and no room kept for either.
    SignedOut,
    /// Asked, and not answered yet: the picture's and the line's room is
    /// kept, so the card does not jump when the answer lands.
    Asking,
    Known(Arc<User>),
    /// Twitch has no account by that name, the ask failed, or it could not
    /// be made: no picture and no line, as signed out.
    Unknown,
}

impl Lookup {
    /// Whether this says the same as `other`, so a card told what it
    /// already has does nothing: the very same answer, for a known account.
    pub fn same_as(&self, other: &Lookup) -> bool {
        match (self, other) {
            (Lookup::Known(a), Lookup::Known(b)) => Arc::ptr_eq(a, b),
            (Lookup::SignedOut, Lookup::SignedOut)
            | (Lookup::Asking, Lookup::Asking)
            | (Lookup::Unknown, Lookup::Unknown) => true,
            _ => false,
        }
    }
}

/// One login's answer.
#[derive(Clone, Debug)]
enum Answer {
    Known(Arc<User>),
    /// Twitch has no account by that name.
    Missing,
    /// The ask failed. Not asked again until a card for the login is opened
    /// afresh; see [`Profiles::reopened`].
    Failed,
}

/// The accounts asked about and answered this session, by lowercase login,
/// and the asks out. Lives on the root and is never saved.
#[derive(Debug, Default)]
pub struct Profiles {
    answers: HashMap<String, Answer>,
    /// The logins answered, the oldest answer first: which goes when there
    /// are more than [`MAX_PROFILES`].
    order: VecDeque<String>,
    /// Logins with an ask out that the worker has not answered.
    out: HashSet<String>,
}

/// What a login is kept by: Twitch's logins are lowercase, and a mention
/// may be written in any case.
fn key(login: &str) -> String {
    login.to_ascii_lowercase()
}

impl Profiles {
    /// Whether `login` is worth asking the worker about now: not answered,
    /// a failure included, and not already asked about.
    pub fn wants(&self, login: &str) -> bool {
        let key = key(login);
        !self.answers.contains_key(&key) && !self.out.contains(&key)
    }

    /// Note that an ask about `login` has gone to the worker.
    pub fn asked(&mut self, login: &str) {
        self.out.insert(key(login));
    }

    /// Take the worker's answer about `login`, and say what is worth a line
    /// in the log, if anything: a failure, once, as it comes back.
    pub fn answered(
        &mut self,
        login: &str,
        result: Result<Option<User>, String>,
    ) -> Option<String> {
        let key = key(login);
        self.out.remove(&key);
        let (answer, line) = match result {
            Ok(Some(user)) => (Answer::Known(Arc::new(user)), None),
            Ok(None) => (Answer::Missing, None),
            Err(reason) => (Answer::Failed, Some(format!("{key}'s account: {reason}"))),
        };
        self.order.retain(|kept| *kept != key);
        self.order.push_back(key.clone());
        self.answers.insert(key, answer);
        while self.order.len() > MAX_PROFILES {
            if let Some(oldest) = self.order.pop_front() {
                self.answers.remove(&oldest);
            }
        }
        line
    }

    /// A card for `login` has been opened afresh: a failure is forgotten,
    /// so it is asked about again. Only for an opening, which a person
    /// does once a press; the sync after every worker event never calls
    /// this, so a failure cannot be asked about in a loop.
    pub fn reopened(&mut self, login: &str) {
        let key = key(login);
        if matches!(self.answers.get(&key), Some(Answer::Failed)) {
            self.answers.remove(&key);
            self.order.retain(|kept| *kept != key);
        }
    }

    /// Forget the asks out, for a worker that will never answer them. The
    /// answers stay, as the badges' do.
    pub fn forget(&mut self) {
        self.out.clear();
    }

    /// What a card for `login` is told, with someone signed in or not.
    pub fn lookup(&self, login: &str, signed_in: bool) -> Lookup {
        let key = key(login);
        match self.answers.get(&key) {
            Some(Answer::Known(user)) => Lookup::Known(user.clone()),
            Some(Answer::Missing | Answer::Failed) => Lookup::Unknown,
            None if self.out.contains(&key) => Lookup::Asking,
            None if !signed_in => Lookup::SignedOut,
            // Signed in, and the ask could not be made.
            None => Lookup::Unknown,
        }
    }
}

/// The card's line about an account's age: `Account created 14 Dec 2016 (9
/// years)`, `(1 year)`, `(3 months)`, `(12 days)`, or `Account created
/// today`. Counted on the local calendar, as `channel_page::when` counts, so
/// an account made after midnight is a day old at breakfast; a month is
/// passed on its day of the month. `None` for a date that does not read.
pub fn account_line(created_at: &str, now: DateTime<Utc>) -> Option<String> {
    let created = DateTime::parse_from_rfc3339(created_at)
        .ok()?
        .with_timezone(&Local)
        .date_naive();
    let today = now.with_timezone(&Local).date_naive();
    if created >= today {
        return Some("Account created today".to_string());
    }
    Some(format!(
        "Account created {} ({})",
        created.format("%-d %b %Y"),
        age(created, today)
    ))
}

/// How long from `from` to `to`, in the largest whole unit there is one of.
fn age(from: NaiveDate, to: NaiveDate) -> String {
    let months = months_between(from, to);
    let (count, unit) = if months >= 12 {
        (months / 12, "year")
    } else if months >= 1 {
        (months, "month")
    } else {
        ((to - from).num_days(), "day")
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural}")
}

/// Whole calendar months from `from` to `to`: one is passed on the day of
/// the month it started on.
fn months_between(from: NaiveDate, to: NaiveDate) -> i64 {
    let months =
        i64::from(to.year() - from.year()) * 12 + i64::from(to.month()) - i64::from(from.month());
    if to.day() < from.day() {
        months - 1
    } else {
        months
    }
}

/// The size, in pixels square, to ask Twitch for a card's picture at on a
/// screen of `scale_factor`; see `twitch_api::profile_image_at`.
pub fn avatar_request(scale_factor: f32) -> u32 {
    if scale_factor >= LARGE_AVATAR_FROM {
        AVATAR_LARGE
    } else {
        AVATAR_SMALL
    }
}

/// The quiet tag after a card's name: `Partner` or `Affiliate`, and nothing
/// for anybody else.
pub fn broadcaster_tag(user: &User) -> Option<&'static str> {
    match user.broadcaster_type {
        BroadcasterType::Partner => Some("Partner"),
        BroadcasterType::Affiliate => Some("Affiliate"),
        BroadcasterType::Other => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(login: &str) -> User {
        User {
            id: "1".into(),
            login: login.into(),
            display_name: login.into(),
            profile_image_url: String::new(),
            created_at: "2016-12-14T12:00:00Z".into(),
            broadcaster_type: BroadcasterType::Other,
        }
    }

    fn at(instant: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(instant)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn each_login_is_asked_about_once() {
        let mut profiles = Profiles::default();
        assert!(profiles.wants("Fisher"));
        profiles.asked("Fisher");
        assert!(!profiles.wants("fisher"), "case is not a new person");
        assert!(!profiles.wants("FISHER"));
        assert!(profiles.wants("someone"));

        assert_eq!(profiles.answered("fisher", Ok(Some(user("fisher")))), None);
        assert!(!profiles.wants("Fisher"), "answered");
        assert_eq!(profiles.answered("nobody", Ok(None)), None);
        assert!(!profiles.wants("nobody"), "no account is an answer too");
    }

    #[test]
    fn a_lookup_says_what_is_known() {
        let mut profiles = Profiles::default();
        assert!(matches!(profiles.lookup("a", false), Lookup::SignedOut));
        assert!(
            matches!(profiles.lookup("a", true), Lookup::Unknown),
            "signed in but not asked: the ask could not be made"
        );

        profiles.asked("a");
        assert!(matches!(profiles.lookup("A", true), Lookup::Asking));
        assert!(
            matches!(profiles.lookup("a", false), Lookup::Asking),
            "an ask out is waited on whatever happens meanwhile"
        );

        profiles.answered("a", Ok(Some(user("a"))));
        let Lookup::Known(known) = profiles.lookup("a", true) else {
            panic!("an answer is known");
        };
        assert_eq!(known.login, "a");
        assert!(
            matches!(profiles.lookup("a", false), Lookup::Known(_)),
            "an answer outlives a sign-out"
        );
        assert!(profiles
            .lookup("a", true)
            .same_as(&profiles.lookup("a", false)));

        profiles.answered("gone", Ok(None));
        assert!(matches!(profiles.lookup("gone", true), Lookup::Unknown));
        profiles.answered("broken", Err("HTTP 500".into()));
        assert!(matches!(profiles.lookup("broken", true), Lookup::Unknown));
    }

    #[test]
    fn a_failure_is_logged_once_and_asked_again_only_on_a_fresh_open() {
        let mut profiles = Profiles::default();
        profiles.asked("fisher");
        assert_eq!(
            profiles
                .answered("fisher", Err("HTTP 503".into()))
                .as_deref(),
            Some("fisher's account: HTTP 503")
        );
        assert!(
            !profiles.wants("fisher"),
            "the sync after every event must not ask again"
        );

        profiles.reopened("someone else");
        assert!(!profiles.wants("fisher"));
        profiles.reopened("Fisher");
        assert!(profiles.wants("fisher"), "a fresh open asks again");

        // A known answer is not forgotten by an opening.
        profiles.answered("known", Ok(Some(user("known"))));
        profiles.reopened("known");
        assert!(!profiles.wants("known"));
    }

    #[test]
    fn a_forgotten_ask_is_asked_again() {
        let mut profiles = Profiles::default();
        profiles.asked("fisher");
        profiles.forget();
        assert!(profiles.wants("fisher"));
        assert!(matches!(
            profiles.lookup("fisher", false),
            Lookup::SignedOut
        ));
    }

    #[test]
    fn the_store_keeps_the_newest_answers() {
        let mut profiles = Profiles::default();
        for index in 0..=MAX_PROFILES {
            profiles.answered(&format!("user{index}"), Ok(None));
        }
        assert!(profiles.wants("user0"), "the oldest answer went");
        assert!(!profiles.wants("user1"));
        assert!(!profiles.wants(&format!("user{MAX_PROFILES}")));
        assert_eq!(profiles.answers.len(), MAX_PROFILES);

        // Answered again, a login is the newest, not the next to go.
        profiles.answered("user1", Ok(None));
        profiles.answered("one more", Ok(None));
        assert!(!profiles.wants("user1"));
        assert!(profiles.wants("user2"));
        assert_eq!(profiles.order.len(), MAX_PROFILES);
    }

    /// Midday instants, as `channel_page`'s test uses, so a local calendar
    /// cannot land on another day than UTC's anywhere a test runs.
    #[test]
    fn account_age_counts_the_way_people_do() {
        let now = at("2026-10-14T12:00:00Z");
        let line = |created: &str| account_line(created, now);
        assert_eq!(
            line("2016-12-14T12:00:00Z").as_deref(),
            Some("Account created 14 Dec 2016 (9 years)")
        );
        assert_eq!(
            line("2025-10-14T12:00:00Z").as_deref(),
            Some("Account created 14 Oct 2025 (1 year)")
        );
        assert_eq!(
            line("2025-10-15T12:00:00Z").as_deref(),
            Some("Account created 15 Oct 2025 (11 months)"),
            "a year is passed on its day"
        );
        assert_eq!(
            line("2026-07-02T12:00:00Z").as_deref(),
            Some("Account created 2 Jul 2026 (3 months)")
        );
        assert_eq!(
            line("2026-09-14T12:00:00Z").as_deref(),
            Some("Account created 14 Sep 2026 (1 month)")
        );
        assert_eq!(
            line("2026-10-02T12:00:00Z").as_deref(),
            Some("Account created 2 Oct 2026 (12 days)")
        );
        assert_eq!(
            line("2026-10-13T12:00:00Z").as_deref(),
            Some("Account created 13 Oct 2026 (1 day)")
        );
        assert_eq!(
            line("2026-10-14T12:00:00Z").as_deref(),
            Some("Account created today")
        );
        assert_eq!(
            line("2026-10-20T12:00:00Z").as_deref(),
            Some("Account created today"),
            "a clock ahead of this one is not a negative age"
        );
        assert_eq!(line("nope"), None);
    }

    #[test]
    fn the_avatar_is_asked_for_at_the_screen_scale() {
        assert_eq!(avatar_request(1.0), 70);
        assert_eq!(avatar_request(1.25), 70);
        assert_eq!(avatar_request(1.5), 150);
        assert_eq!(avatar_request(2.0), 150);
    }

    #[test]
    fn partners_and_affiliates_are_tagged() {
        let mut partner = user("a");
        partner.broadcaster_type = BroadcasterType::Partner;
        assert_eq!(broadcaster_tag(&partner), Some("Partner"));
        let mut affiliate = user("b");
        affiliate.broadcaster_type = BroadcasterType::Affiliate;
        assert_eq!(broadcaster_tag(&affiliate), Some("Affiliate"));
        assert_eq!(broadcaster_tag(&user("c")), None);
    }
}
