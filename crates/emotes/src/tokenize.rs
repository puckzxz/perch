//! Split a chat message into text and emote runs.
//!
//! Twitch tells us exactly where its emotes are via the IRC `emotes` tag:
//!
//! ```text
//! emotes=emotesv2_97e86b26bef74d74bf952ebce0d1fa01:0-7,23-30/25:12-16
//! ```
//!
//! Each entry is `id:start-end[,start-end]`, ids separated by `/`, ranges
//! inclusive. The indices count **characters, not bytes** — slicing a Rust
//! `&str` with them directly is wrong the moment a message contains any
//! non-ASCII, which on Twitch is constantly.

use crate::providers::Provider;

/// One piece of a rendered message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Text(String),
    Emote(Emote),
    /// A word that spent Bits, drawn as its cheermote ([`apply_cheers`]).
    Cheer(Cheer),
}

/// A cheer in a line that spent Bits: `Cheer100` as the picture of the
/// tier its amount reaches, with the amount written after it in the
/// tier's colour, as Twitch's own chat draws it. What a word is worth and
/// which picture it gets are the lookup's (`perch::chat_cheers`); this
/// crate only places it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cheer {
    /// The word as sent, `Cheer100`, kept for the tooltip.
    pub name: String,
    /// The Bits it spent.
    pub amount: u32,
    pub url: String,
    /// The tier's colour, 0xRRGGBB, for the amount.
    pub color: u32,
}

/// What an emote does to the emotes written beside it, as its provider says
/// when its set is loaded (`providers`). [`stack`](crate::stack()) is what
/// acts on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A picture of its own. Every Twitch emote, and most of the rest.
    Normal,
    /// Drawn over the emote before it, in that emote's box: 7TV's zero-width
    /// flag, BetterTTV's eight zero-width globals, and a FrankerFaceZ
    /// modifier without its Hidden flag. With no emote before it, it is a
    /// picture of its own.
    Overlay,
    /// An effect on the emote before it: a FrankerFaceZ modifier with its
    /// Hidden flag (`ffzW`, `ffzX`, `ffzY`, `ffzCursed`). With no emote
    /// before it, it is a picture of its own, as FrankerFaceZ draws it.
    EffectOnPrevious,
    /// An effect on the emote after it: a BetterTTV modifier (`w!`, `h!`,
    /// `v!`, `z!`, `c!`, `l!`, `r!`, `p!`, `s!`). With no emote after it,
    /// it stays its word, as BetterTTV leaves it.
    EffectOnNext,
}

/// What looking a name up finds: a third-party emote's picture, the
/// provider whose set won the name, and its kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Named {
    pub url: String,
    pub provider: Provider,
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Emote {
    /// The word this replaced, kept for tooltips and for copying text out.
    pub name: String,
    pub url: String,
    /// Whose emote it is: Twitch's, placed by the `emotes` tag, or the
    /// third-party set that won the name.
    pub provider: Provider,
    pub kind: Kind,
    /// What was written against this emote and is drawn with it, in the
    /// order written: BetterTTV's effects before it, then the overlays and
    /// FrankerFaceZ's effects after it. Filled by [`stack`](crate::stack())
    /// and never nested: a layer's own `attached` is empty.
    pub attached: Vec<Emote>,
}

impl Emote {
    /// An emote standing alone, with nothing attached.
    pub fn new(
        name: impl Into<String>,
        url: impl Into<String>,
        provider: Provider,
        kind: Kind,
    ) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            provider,
            kind,
            attached: Vec::new(),
        }
    }

    /// The layers drawn over this emote's picture, bottom first.
    pub fn overlays(&self) -> impl Iterator<Item = &Emote> {
        self.attached
            .iter()
            .filter(|layer| layer.kind == Kind::Overlay)
    }

    /// The effects written against this emote, in the order written:
    /// BetterTTV's before it, then FrankerFaceZ's after it. Each is known by
    /// its name (`w!`, `ffzX`), which is what says what it does to the
    /// picture.
    pub fn effects(&self) -> impl Iterator<Item = &Emote> {
        self.attached
            .iter()
            .filter(|layer| matches!(layer.kind, Kind::EffectOnPrevious | Kind::EffectOnNext))
    }

    /// The stack in the order it was written: BetterTTV's effects, this
    /// emote itself, then its overlays and FrankerFaceZ's effects.
    pub fn written(&self) -> impl Iterator<Item = &Emote> {
        let before = self
            .attached
            .iter()
            .filter(|layer| layer.kind == Kind::EffectOnNext);
        let after = self
            .attached
            .iter()
            .filter(|layer| layer.kind != Kind::EffectOnNext);
        before.chain(std::iter::once(self)).chain(after)
    }

    /// The stack as it was written, one space between its words
    /// (`w! KEKW SteerR ffzX`). What copying the emote's name copies, so
    /// that pasting it draws the same picture.
    pub fn typed(&self) -> String {
        self.written()
            .map(|emote| emote.name.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// What tells this picture apart from any other in a line: its URL, then
    /// each attached layer's, space-joined. Just the URL for an emote
    /// standing alone, so a lone emote keeps the element id it always had,
    /// and a stack gets one of its own: its tooltip says more than the emote
    /// alone does, and a tooltip keeps the words it came up with for as long
    /// as its id lasts.
    pub fn id_key(&self) -> String {
        std::iter::once(self.url.as_str())
            .chain(self.attached.iter().map(|layer| layer.url.as_str()))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Twitch's emote CDN. `format` accepts `static` (PNG) or `default`, which
/// serves an animated GIF when the emote has one.
pub fn twitch_emote_url(id: &str, scale: &str, animated: bool) -> String {
    let format = if animated { "default" } else { "static" };
    format!("https://static-cdn.jtvnw.net/emoticons/v2/{id}/{format}/dark/{scale}")
}

/// Parse the `emotes` tag into `(id, start, end)` triples, sorted by position.
///
/// Malformed entries are skipped rather than failing the whole message: a chat
/// line is not worth dropping over one bad range.
fn parse_tag(tag: &str) -> Vec<(&str, usize, usize)> {
    let mut spans = Vec::new();
    for entry in tag.split('/').filter(|e| !e.is_empty()) {
        let Some((id, ranges)) = entry.split_once(':') else {
            continue;
        };
        for range in ranges.split(',').filter(|r| !r.is_empty()) {
            let Some((start, end)) = range.split_once('-') else {
                continue;
            };
            let (Ok(start), Ok(end)) = (start.parse::<usize>(), end.parse::<usize>()) else {
                continue;
            };
            if end >= start {
                spans.push((id, start, end));
            }
        }
    }
    spans.sort_by_key(|(_, start, _)| *start);
    spans
}

/// Split `text` into text and emote tokens using the `emotes` tag.
///
/// `animated` selects the GIF variant where Twitch has one.
pub fn tokenize(text: &str, emote_tag: Option<&str>, animated: bool) -> Vec<Token> {
    let spans = emote_tag.map(parse_tag).unwrap_or_default();
    if spans.is_empty() {
        return if text.is_empty() {
            Vec::new()
        } else {
            vec![Token::Text(text.to_string())]
        };
    }

    // Indexing by character is the whole point here, so pay for the Vec once.
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut cursor = 0usize;

    for (id, start, end) in spans {
        // Overlapping or out-of-bounds ranges mean a malformed tag; skip them
        // rather than panicking on a slice.
        if start < cursor || end >= chars.len() {
            continue;
        }
        if start > cursor {
            let text: String = chars[cursor..start].iter().collect();
            tokens.push(Token::Text(text));
        }
        let name: String = chars[start..=end].iter().collect();
        tokens.push(Token::Emote(Emote::new(
            name,
            twitch_emote_url(id, "2.0", animated),
            Provider::Twitch,
            Kind::Normal,
        )));
        cursor = end + 1;
    }

    if cursor < chars.len() {
        let text: String = chars[cursor..].iter().collect();
        tokens.push(Token::Text(text));
    }
    tokens
}

/// Replace whole words with third-party emotes, leaving other text alone.
///
/// BetterTTV and friends have no positional data — they are name lookups, so
/// matching has to be word-exact or `LUL` starts matching inside `LULW`.
/// Each emote carries the provider and kind its lookup found.
pub fn apply_named_emotes(
    tokens: Vec<Token>,
    lookup: &impl Fn(&str) -> Option<Named>,
) -> Vec<Token> {
    replace_words(tokens, |word| {
        let named = lookup(word)?;
        Some(Token::Emote(Emote::new(
            word,
            named.url,
            named.provider,
            named.kind,
        )))
    })
}

/// Replace whole words with cheers, in a message whose `bits` tag says it
/// spent `bits`, leaving every other word alone.
///
/// Word-exact as the named emotes are, and only in the text: a word Twitch
/// placed an emote on stays that emote. `lookup` says what a word would be
/// (`Cheer100`, in any case, is 100 Bits of `Cheer`), and the tag says how
/// far the words go: each cheer is taken from what is left, in the order
/// written, and one that would spend more than is left stays a word. So a
/// `Cheer100` typed beside a 1-Bit cheer is not drawn as a hundred.
/// Chatterino keeps the same running budget.
pub fn apply_cheers(
    tokens: Vec<Token>,
    bits: u32,
    lookup: &impl Fn(&str) -> Option<Cheer>,
) -> Vec<Token> {
    let mut left = bits;
    replace_words(tokens, |word| {
        let cheer = lookup(word).filter(|cheer| cheer.amount <= left)?;
        left -= cheer.amount;
        Some(Token::Cheer(cheer))
    })
}

/// The word walk both replacements share: each space-separated word of the
/// text tokens offered to `replace` in order, and put in the line as what
/// it gives back, or left text where it gives nothing. The spaces between
/// words are kept as they were sent, a token that is not text is passed
/// through untouched, and an empty word, between two spaces, is never
/// offered.
fn replace_words(tokens: Vec<Token>, mut replace: impl FnMut(&str) -> Option<Token>) -> Vec<Token> {
    let mut out = Vec::new();
    for token in tokens {
        let Token::Text(text) = token else {
            out.push(token);
            continue;
        };

        let mut pending = String::new();
        for word in text.split_inclusive(' ') {
            let trimmed = word.trim_end_matches(' ');
            let replaced = if trimmed.is_empty() {
                None
            } else {
                replace(trimmed)
            };
            match replaced {
                Some(replaced) => {
                    if !pending.is_empty() {
                        out.push(Token::Text(std::mem::take(&mut pending)));
                    }
                    out.push(replaced);
                    // Preserve the separator that split_inclusive kept.
                    if word.ends_with(' ') {
                        pending.push(' ');
                    }
                }
                None => pending.push_str(word),
            }
        }
        if !pending.is_empty() {
            out.push(Token::Text(pending));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(tokens: &[Token]) -> Vec<String> {
        tokens
            .iter()
            .map(|t| match t {
                Token::Text(s) => format!("T:{s}"),
                Token::Emote(e) => format!("E:{}", e.name),
                Token::Cheer(c) => format!("C:{}", c.name),
            })
            .collect()
    }

    #[test]
    fn plain_text_when_no_tag() {
        assert_eq!(
            tokenize("hello", None, false),
            vec![Token::Text("hello".into())]
        );
        assert!(tokenize("", None, false).is_empty());
    }

    #[test]
    fn empty_tag_is_plain_text() {
        assert_eq!(names(&tokenize("hello", Some(""), false)), vec!["T:hello"]);
    }

    #[test]
    fn splits_a_single_emote() {
        let tokens = tokenize("hey Kappa there", Some("25:4-8"), false);
        assert_eq!(names(&tokens), vec!["T:hey ", "E:Kappa", "T: there"]);
    }

    #[test]
    fn handles_repeated_ranges_for_one_id() {
        let tokens = tokenize("Kappa and Kappa", Some("25:0-4,10-14"), false);
        assert_eq!(names(&tokens), vec!["E:Kappa", "T: and ", "E:Kappa"]);
    }

    #[test]
    fn orders_emotes_from_different_ids() {
        let tokens = tokenize("aa LUL bb Kappa", Some("25:10-14/=:3-5"), false);
        assert_eq!(names(&tokens), vec!["T:aa ", "E:LUL", "T: bb ", "E:Kappa"]);
    }

    /// The regression this module exists to prevent: Twitch counts characters,
    /// Rust slices bytes, and every multi-byte character shifts the two apart.
    #[test]
    fn ranges_are_character_indices_not_byte_indices() {
        // "日本語" is 9 bytes but 3 characters, so Kappa starts at char 4.
        let text = "日本語 Kappa";
        assert_eq!(text.len(), 15);
        let tokens = tokenize(text, Some("25:4-8"), false);
        assert_eq!(names(&tokens), vec!["T:日本語 ", "E:Kappa"]);
    }

    #[test]
    fn emoji_before_an_emote_still_lines_up() {
        let text = "🎉 Kappa";
        let tokens = tokenize(text, Some("25:2-6"), false);
        assert_eq!(names(&tokens), vec!["T:🎉 ", "E:Kappa"]);
    }

    #[test]
    fn skips_out_of_bounds_ranges() {
        let tokens = tokenize("short", Some("25:0-99"), false);
        assert_eq!(names(&tokens), vec!["T:short"]);
    }

    #[test]
    fn skips_malformed_entries() {
        let tokens = tokenize("hello", Some("garbage/25:bad-range/:"), false);
        assert_eq!(names(&tokens), vec!["T:hello"]);
    }

    #[test]
    fn animated_flag_picks_the_gif_variant() {
        assert!(twitch_emote_url("1", "2.0", true).contains("/default/"));
        assert!(twitch_emote_url("1", "2.0", false).contains("/static/"));
    }

    fn named(url: &str) -> Named {
        Named {
            url: url.into(),
            provider: Provider::Bttv,
            kind: Kind::Normal,
        }
    }

    #[test]
    fn named_emotes_match_whole_words_only() {
        let lookup = |name: &str| (name == "LUL").then(|| named("u"));
        let tokens = apply_named_emotes(vec![Token::Text("LULW LUL x".into())], &lookup);
        assert_eq!(names(&tokens), vec!["T:LULW ", "E:LUL", "T: x"]);
    }

    /// Twitch's emotes are Twitch's and plain; a name looked up keeps the
    /// provider and the kind its set gave it.
    #[test]
    fn named_emotes_carry_their_provider_and_kind() {
        let lookup = |name: &str| {
            (name == "SteerR").then(|| Named {
                url: "steer".into(),
                provider: Provider::SevenTv,
                kind: Kind::Overlay,
            })
        };
        let tokens = apply_named_emotes(tokenize("Kappa SteerR", Some("25:0-4"), false), &lookup);
        let emotes: Vec<_> = tokens
            .iter()
            .filter_map(|token| match token {
                Token::Emote(emote) => Some((emote.name.as_str(), emote.provider, emote.kind)),
                Token::Text(_) | Token::Cheer(_) => None,
            })
            .collect();
        assert_eq!(
            emotes,
            [
                ("Kappa", Provider::Twitch, Kind::Normal),
                ("SteerR", Provider::SevenTv, Kind::Overlay),
            ]
        );
    }

    #[test]
    fn named_emotes_leave_existing_emotes_alone() {
        let lookup = |_: &str| Some(named("u"));
        let input = vec![Token::Emote(Emote::new(
            "Kappa",
            "orig",
            Provider::Twitch,
            Kind::Normal,
        ))];
        let tokens = apply_named_emotes(input, &lookup);
        assert_eq!(tokens.len(), 1);
        match &tokens[0] {
            Token::Emote(e) => assert_eq!(e.url, "orig"),
            _ => panic!("emote was replaced"),
        }
    }

    /// A cheer lookup over `Cheer` in any case, each amount one picture:
    /// what a word is worth is the lookup's to say, not this module's.
    fn cheer(word: &str) -> Option<Cheer> {
        let prefix = word.trim_end_matches(|c: char| c.is_ascii_digit());
        let amount = word[prefix.len()..].parse().ok()?;
        prefix.eq_ignore_ascii_case("cheer").then(|| Cheer {
            name: word.to_string(),
            amount,
            url: "https://example.invalid/cheer".into(),
            color: 0x979797,
        })
    }

    #[test]
    fn cheers_replace_whole_words_within_the_bits() {
        let text = vec![Token::Text("cheer100 great Cheer1".into())];
        let tokens = apply_cheers(text, 101, &cheer);
        assert_eq!(names(&tokens), ["C:cheer100", "T: great ", "C:Cheer1"]);
        let Token::Cheer(first) = &tokens[0] else {
            panic!("not a cheer");
        };
        assert_eq!(first.amount, 100);
        // Inside a word is not a cheer, nor is the prefix alone.
        let text = vec![Token::Text("xCheer100 Cheer Cheer100!".into())];
        assert_eq!(
            names(&apply_cheers(text, 100, &cheer)),
            ["T:xCheer100 Cheer Cheer100!"]
        );
    }

    /// The `bits` tag is the most the words can spend, taken in the order
    /// written: a word past what is left stays the word it was typed as.
    #[test]
    fn a_cheer_past_the_bits_left_stays_a_word() {
        let text = vec![Token::Text("Cheer100 Cheer100".into())];
        assert_eq!(
            names(&apply_cheers(text, 100, &cheer)),
            ["C:Cheer100", "T: Cheer100"]
        );
        let text = vec![Token::Text("Cheer500 Cheer1".into())];
        assert_eq!(
            names(&apply_cheers(text, 1, &cheer)),
            ["T:Cheer500 ", "C:Cheer1"]
        );
    }

    /// A word Twitch put an emote on stays that emote.
    #[test]
    fn cheers_leave_emotes_alone() {
        let tokens = apply_cheers(tokenize("Kappa Cheer1", Some("25:0-4"), false), 1, &cheer);
        assert_eq!(names(&tokens), ["E:Kappa", "T: ", "C:Cheer1"]);
    }

    #[test]
    fn cheers_keep_the_spaces_between_words() {
        let text = vec![Token::Text("a  Cheer1  b ".into())];
        assert_eq!(
            names(&apply_cheers(text, 1, &cheer)),
            ["T:a  ", "C:Cheer1", "T:  b "]
        );
    }
}
