//! What a single word in a chat message is.
//!
//! Messages are already split into words before layout, because GPUI cannot put
//! an image inside a run of text — so a word is the unit that gets an emote, and
//! it is conveniently also the unit that wants to be a click target.
//!
//! The hard part is not finding URLs, it is *not* finding them. Chat is full of
//! `lol.`, `1.5`, `wtf.jpg` and `...`, and anything that keys off "contains a
//! dot" underlines a large fraction of ordinary words. So a bare host only
//! counts when what follows its last dot is a real top-level domain, and the
//! list below is deliberately conservative: `.so`, `.is`, `.at` and `.it` are
//! all real TLDs and all common English words, so they are left out. A link
//! that needs its scheme typed is a smaller problem than "ok.so" turning blue.
//!
//! This runs only on text that survived emote extraction, so an emote name can
//! never reach it — see `render_message`, which classifies `Token::Text` only.

/// What a word turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Plain,
    Link,
    /// An `@name` reference to another chatter.
    Mention,
}

/// A word, split from the punctuation around it.
///
/// The punctuation is kept separate so a trailing comma is neither underlined
/// nor sent to the browser, which is the detail that makes link handling feel
/// deliberate rather than approximate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word<'a> {
    pub kind: Kind,
    pub leading: &'a str,
    pub body: &'a str,
    pub trailing: &'a str,
}

impl Word<'_> {
    /// Where a link points. Bare hosts are assumed to be https, which is what
    /// every browser does with a typed host anyway.
    pub fn url(&self) -> String {
        if has_scheme(self.body) {
            self.body.to_string()
        } else {
            format!("https://{}", self.body)
        }
    }

    /// The login a mention refers to, without the `@`.
    pub fn mentioned(&self) -> Option<&str> {
        (self.kind == Kind::Mention).then(|| self.body.trim_start_matches('@'))
    }
}

/// Top-level domains a bare host is allowed to end in.
///
/// Not the IANA list. Every entry here is one that is either overwhelmingly
/// common in chat or unambiguous as an English word; see the module comment for
/// why that trade is the right way round.
const TLDS: &[&str] = &[
    "com", "net", "org", "edu", "gov", "io", "co", "tv", "gg", "dev", "app", "xyz", "info", "biz",
    "online", "site", "tech", "store", "blog", "wiki", "news", "club", "moe", "gl", "fm", "ly",
    "cc", "ws", "uk", "de", "fr", "jp", "au", "ru", "nl", "se", "es", "pl", "br", "ca", "kr", "cn",
    "mx", "pt", "cz", "dk", "fi", "nz", "za", "tw", "hk", "sg", "eu", "tk",
];

/// Punctuation that can sit in front of a word without being part of it.
const OPENERS: &[char] = &['(', '[', '{', '"', '\'', '«', '“', '‘'];
/// Punctuation that can trail a word without being part of it. Closing brackets
/// are handled separately, because they may genuinely belong to a URL.
const CLOSERS: &[char] = &['.', ',', '!', '?', ';', ':', '"', '\'', '»', '”', '’', '…'];

fn has_scheme(word: &str) -> bool {
    let lower = word.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// Peel punctuation off both ends.
///
/// A closing bracket only comes off when it is unmatched, so a URL that
/// genuinely contains one — Wikipedia articles are full of them — survives.
fn peel(word: &str) -> (&str, &str, &str) {
    let body = word.trim_start_matches(OPENERS);
    let leading = &word[..word.len() - body.len()];

    let mut end = body;
    loop {
        let trimmed = end.trim_end_matches(CLOSERS);
        let trimmed = match trimmed.strip_suffix(')') {
            Some(shorter) if trimmed.matches('(').count() < trimmed.matches(')').count() => shorter,
            _ => trimmed,
        };
        if trimmed.len() == end.len() {
            break;
        }
        end = trimmed;
    }

    (leading, end, &body[end.len()..])
}

/// Whether a bare host looks like one: `something.tld`, optionally with a path.
fn looks_like_host(body: &str) -> bool {
    let host = body.split(['/', '?', '#']).next().unwrap_or(body);
    // An email address is not a web link, and turning one into `https://` is
    // worse than leaving it as text.
    if host.contains('@') {
        return false;
    }
    let Some((name, tld)) = host.rsplit_once('.') else {
        return false;
    };
    if name.is_empty() || !name.contains(|c: char| c.is_ascii_alphanumeric()) {
        return false;
    }
    TLDS.contains(&tld.to_ascii_lowercase().as_str())
}

/// A login is 4-25 characters of letters, digits and underscores. Twitch's own
/// rule, which is what keeps `@` on its own or `@!!!` from becoming a mention.
fn looks_like_login(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 25
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Decide what one whitespace-delimited word is.
pub fn classify(word: &str) -> Word<'_> {
    let (leading, body, trailing) = peel(word);

    let kind = if body.is_empty() {
        Kind::Plain
    } else if has_scheme(body) {
        Kind::Link
    } else if let Some(name) = body.strip_prefix('@') {
        if looks_like_login(name) {
            Kind::Mention
        } else {
            Kind::Plain
        }
    } else if looks_like_host(body) {
        Kind::Link
    } else {
        Kind::Plain
    };

    Word {
        kind,
        leading,
        body,
        trailing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(word: &str) -> Kind {
        classify(word).kind
    }

    #[test]
    fn finds_urls_with_a_scheme() {
        assert_eq!(kind("https://twitch.tv/moonmoon"), Kind::Link);
        assert_eq!(kind("http://example.com"), Kind::Link);
        assert_eq!(kind("HTTPS://EXAMPLE.COM"), Kind::Link);
    }

    #[test]
    fn finds_bare_hosts_and_assumes_https() {
        let word = classify("twitch.tv/moonmoon");
        assert_eq!(word.kind, Kind::Link);
        assert_eq!(word.url(), "https://twitch.tv/moonmoon");
    }

    /// The whole reason for the TLD list. Every one of these appears constantly
    /// in chat and none of them is a link.
    #[test]
    fn ordinary_words_with_dots_are_not_links() {
        for word in [
            "lol.", "wtf.jpg", "1.5", "...", "e.g", "i.e", "no.1", "hi..", ".", "..gg", "a.b",
        ] {
            assert_eq!(kind(word), Kind::Plain, "{word:?} should not be a link");
        }
    }

    #[test]
    fn trailing_punctuation_is_not_part_of_the_link() {
        let word = classify("twitch.tv,");
        assert_eq!(word.kind, Kind::Link);
        assert_eq!(word.body, "twitch.tv");
        assert_eq!(word.trailing, ",");
        assert_eq!(word.url(), "https://twitch.tv");

        let word = classify("see https://example.com/x!?");
        // Whitespace splitting happens before this, so the word here is whole.
        assert_eq!(classify("https://example.com/x!?").trailing, "!?");
        let _ = word;
    }

    #[test]
    fn brackets_around_a_link_are_peeled_but_matched_ones_are_kept() {
        let word = classify("(https://example.com)");
        assert_eq!(word.kind, Kind::Link);
        assert_eq!(word.leading, "(");
        assert_eq!(word.body, "https://example.com");
        assert_eq!(word.trailing, ")");

        // A bracket the URL genuinely owns must survive.
        let word = classify("https://en.wikipedia.org/wiki/Rust_(programming_language)");
        assert_eq!(word.kind, Kind::Link);
        assert_eq!(word.trailing, "");
        assert!(word.body.ends_with(')'));
    }

    #[test]
    fn finds_mentions_and_keeps_punctuation_out_of_the_name() {
        let word = classify("@moonmoon,");
        assert_eq!(word.kind, Kind::Mention);
        assert_eq!(word.mentioned(), Some("moonmoon"));
        assert_eq!(word.trailing, ",");

        assert_eq!(kind("@ben_"), Kind::Mention);
        assert_eq!(kind("@"), Kind::Plain);
        assert_eq!(kind("@!!!"), Kind::Plain);
        assert_eq!(kind("email@example.com"), Kind::Plain);
    }

    /// Chat is not ASCII. Peeling must not slice a multi-byte character in half.
    #[test]
    fn handles_non_ascii_without_panicking() {
        for word in [
            "привет",
            "「hi」",
            "日本語。",
            "«bonjour»",
            "🎉🎉",
            "@ユーザー",
        ] {
            let classified = classify(word);
            let rebuilt = format!(
                "{}{}{}",
                classified.leading, classified.body, classified.trailing
            );
            assert_eq!(rebuilt, word, "{word:?} did not survive a round trip");
        }
    }

    /// Whatever a word turns out to be, putting it back together must give the
    /// original — otherwise rendering silently drops characters.
    #[test]
    fn every_word_round_trips() {
        for word in [
            "hello",
            "https://twitch.tv/x",
            "@name?",
            "(twitch.tv)",
            "...",
            "\"quoted\"",
        ] {
            let classified = classify(word);
            let rebuilt = format!(
                "{}{}{}",
                classified.leading, classified.body, classified.trailing
            );
            assert_eq!(rebuilt, word);
        }
    }
}
