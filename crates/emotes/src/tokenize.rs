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

/// One piece of a rendered message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Text(String),
    Emote(Emote),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Emote {
    /// The word this replaced, kept for tooltips and for copying text out.
    pub name: String,
    pub url: String,
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
        tokens.push(Token::Emote(Emote {
            url: twitch_emote_url(id, "2.0", animated),
            name,
        }));
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
pub fn apply_named_emotes(
    tokens: Vec<Token>,
    lookup: &impl Fn(&str) -> Option<String>,
) -> Vec<Token> {
    let mut out = Vec::new();
    for token in tokens {
        let Token::Text(text) = token else {
            out.push(token);
            continue;
        };

        let mut pending = String::new();
        for word in text.split_inclusive(' ') {
            let trimmed = word.trim_end_matches(' ');
            match lookup(trimmed) {
                Some(url) if !trimmed.is_empty() => {
                    if !pending.is_empty() {
                        out.push(Token::Text(std::mem::take(&mut pending)));
                    }
                    out.push(Token::Emote(Emote {
                        name: trimmed.to_string(),
                        url,
                    }));
                    // Preserve the separator that split_inclusive kept.
                    if word.ends_with(' ') {
                        pending.push(' ');
                    }
                }
                _ => pending.push_str(word),
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
            })
            .collect()
    }

    #[test]
    fn plain_text_when_no_tag() {
        assert_eq!(tokenize("hello", None, false), vec![Token::Text("hello".into())]);
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

    #[test]
    fn named_emotes_match_whole_words_only() {
        let lookup = |name: &str| (name == "LUL").then(|| "u".to_string());
        let tokens = apply_named_emotes(vec![Token::Text("LULW LUL x".into())], &lookup);
        assert_eq!(names(&tokens), vec!["T:LULW ", "E:LUL", "T: x"]);
    }

    #[test]
    fn named_emotes_leave_existing_emotes_alone() {
        let lookup = |_: &str| Some("u".to_string());
        let input = vec![Token::Emote(Emote {
            name: "Kappa".into(),
            url: "orig".into(),
        })];
        let tokens = apply_named_emotes(input, &lookup);
        assert_eq!(tokens.len(), 1);
        match &tokens[0] {
            Token::Emote(e) => assert_eq!(e.url, "orig"),
            _ => panic!("emote was replaced"),
        }
    }
}
