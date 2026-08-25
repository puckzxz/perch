//! IRCv3 line parsing, and the Twitch-specific bits layered on top.
//!
//! Twitch chat is IRC with message tags, so a line looks like:
//!
//! ```text
//! @color=#FF0000;display-name=Foo :foo!foo@foo.tmi.twitch.tv PRIVMSG #bar :hello
//! ```
//!
//! Kept free of any network or UI types so it can be tested directly.

use std::collections::HashMap;

/// A parsed IRC line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrcMessage {
    pub tags: HashMap<String, String>,
    /// The part between `:` and the command, e.g. `foo!foo@foo.tmi.twitch.tv`.
    pub prefix: Option<String>,
    pub command: String,
    /// Middle params plus the trailing param, if any, as one list.
    pub params: Vec<String>,
}

impl IrcMessage {
    pub fn param(&self, index: usize) -> Option<&str> {
        self.params.get(index).map(String::as_str)
    }

    pub fn tag(&self, key: &str) -> Option<&str> {
        self.tags.get(key).map(String::as_str).filter(|v| !v.is_empty())
    }

    /// The nickname portion of the prefix, e.g. `foo` from `foo!foo@foo.tmi...`.
    pub fn nick(&self) -> Option<&str> {
        let prefix = self.prefix.as_deref()?;
        Some(prefix.split(['!', '@']).next().unwrap_or(prefix))
    }
}

/// Undo IRCv3 tag value escaping.
///
/// A trailing lone backslash is dropped, which is what the spec requires.
fn unescape_tag(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some(':') => out.push(';'),
            Some('s') => out.push(' '),
            Some('\\') => out.push('\\'),
            Some('r') => out.push('\r'),
            Some('n') => out.push('\n'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// Parse one line. Returns `None` for blank lines or lines with no command.
pub fn parse_line(line: &str) -> Option<IrcMessage> {
    let mut rest = line.trim_end_matches(['\r', '\n']);
    if rest.is_empty() {
        return None;
    }

    let mut tags = HashMap::new();
    if let Some(stripped) = rest.strip_prefix('@') {
        let (raw_tags, remainder) = stripped.split_once(' ')?;
        for pair in raw_tags.split(';') {
            if pair.is_empty() {
                continue;
            }
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            tags.insert(key.to_string(), unescape_tag(value));
        }
        rest = remainder;
    }

    let mut prefix = None;
    if let Some(stripped) = rest.strip_prefix(':') {
        let (raw_prefix, remainder) = stripped.split_once(' ')?;
        prefix = Some(raw_prefix.to_string());
        rest = remainder;
    }

    // The trailing param starts at " :" and keeps its spaces verbatim.
    let (head, trailing) = match rest.split_once(" :") {
        Some((head, trailing)) => (head, Some(trailing.to_string())),
        None => (rest, None),
    };

    let mut parts = head.split_whitespace();
    let command = parts.next()?.to_string();
    let mut params: Vec<String> = parts.map(str::to_string).collect();
    params.extend(trailing);

    Some(IrcMessage {
        tags,
        prefix,
        command,
        params,
    })
}

/// One chat line, ready to display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub login: String,
    pub display_name: String,
    /// 0xRRGGBB. Twitch omits this for users who never picked one, so we fall
    /// back to a stable colour derived from the login, the way web chat does.
    pub color: u32,
    pub text: String,
    /// True for `/me` messages, which arrive wrapped in ACTION control codes.
    pub is_action: bool,
}

/// Twitch's default colour set, used when a user has not chosen one.
const DEFAULT_COLORS: [u32; 15] = [
    0xFF0000, 0x0000FF, 0x00FF00, 0xB22222, 0xFF7F50, 0x9ACD32, 0xFF4500, 0x2E8B57, 0xDAA520,
    0xD2691E, 0x5F9EA0, 0x1E90FF, 0xFF69B4, 0x8A2BE2, 0x00FF7F,
];

/// Stable per-login colour. Twitch's own scheme is not public, so this only
/// needs to be deterministic and evenly spread, not identical to the website.
fn fallback_color(login: &str) -> u32 {
    let hash = login
        .bytes()
        .fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
    DEFAULT_COLORS[hash as usize % DEFAULT_COLORS.len()]
}

fn parse_hex_color(raw: &str) -> Option<u32> {
    u32::from_str_radix(raw.strip_prefix('#')?, 16).ok()
}

impl ChatMessage {
    /// Build a chat message from a PRIVMSG line, or `None` if it is not one.
    pub fn from_irc(message: &IrcMessage) -> Option<Self> {
        if message.command != "PRIVMSG" {
            return None;
        }
        let login = message.nick()?.to_string();
        let raw_text = message.param(1)?;

        // `/me` arrives as \x01ACTION text\x01.
        let (text, is_action) = match raw_text
            .strip_prefix('\u{1}')
            .and_then(|t| t.strip_suffix('\u{1}'))
            .and_then(|t| t.strip_prefix("ACTION "))
        {
            Some(inner) => (inner.to_string(), true),
            None => (raw_text.to_string(), false),
        };

        let display_name = message
            .tag("display-name")
            .filter(|name| !name.is_empty())
            .unwrap_or(&login)
            .to_string();

        let color = message
            .tag("color")
            .and_then(parse_hex_color)
            .unwrap_or_else(|| fallback_color(&login));

        Some(Self {
            login,
            display_name,
            color,
            text,
            is_action,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_plain_privmsg() {
        let m = parse_line(":foo!foo@foo.tmi.twitch.tv PRIVMSG #bar :hello world").unwrap();
        assert_eq!(m.command, "PRIVMSG");
        assert_eq!(m.nick(), Some("foo"));
        assert_eq!(m.param(0), Some("#bar"));
        assert_eq!(m.param(1), Some("hello world"));
    }

    #[test]
    fn parses_tags_and_unescapes_values() {
        let line = r"@color=#FF0000;display-name=Foo\sBar;emotes= :foo!foo@foo.tmi.twitch.tv PRIVMSG #bar :hi";
        let m = parse_line(line).unwrap();
        assert_eq!(m.tag("color"), Some("#FF0000"));
        assert_eq!(m.tag("display-name"), Some("Foo Bar"));
        // Empty tag values read as absent, so callers can use unwrap_or.
        assert_eq!(m.tag("emotes"), None);
    }

    #[test]
    fn unescapes_every_documented_sequence() {
        assert_eq!(unescape_tag(r"a\sb"), "a b");
        assert_eq!(unescape_tag(r"a\:b"), "a;b");
        assert_eq!(unescape_tag(r"a\\b"), r"a\b");
        assert_eq!(unescape_tag(r"a\rb"), "a\rb");
        assert_eq!(unescape_tag(r"a\nb"), "a\nb");
        // Lone trailing backslash is dropped per spec.
        assert_eq!(unescape_tag(r"ab\"), "ab");
    }

    #[test]
    fn keeps_colons_inside_the_trailing_param() {
        let m = parse_line(":a!a@a PRIVMSG #c :look: a URL https://x.test/y").unwrap();
        assert_eq!(m.param(1), Some("look: a URL https://x.test/y"));
    }

    #[test]
    fn parses_commands_without_a_prefix() {
        let m = parse_line("PING :tmi.twitch.tv").unwrap();
        assert_eq!(m.command, "PING");
        assert_eq!(m.prefix, None);
        assert_eq!(m.param(0), Some("tmi.twitch.tv"));
    }

    #[test]
    fn ignores_blank_lines() {
        assert!(parse_line("").is_none());
        assert!(parse_line("\r\n").is_none());
    }

    #[test]
    fn falls_back_to_login_when_display_name_missing() {
        let m = parse_line(":foo!foo@foo.tmi.twitch.tv PRIVMSG #bar :hi").unwrap();
        let chat = ChatMessage::from_irc(&m).unwrap();
        assert_eq!(chat.display_name, "foo");
        assert!(!chat.is_action);
    }

    #[test]
    fn extracts_action_messages() {
        let m = parse_line(":foo!foo@foo.tmi.twitch.tv PRIVMSG #bar :\u{1}ACTION waves\u{1}").unwrap();
        let chat = ChatMessage::from_irc(&m).unwrap();
        assert!(chat.is_action);
        assert_eq!(chat.text, "waves");
    }

    #[test]
    fn fallback_colour_is_stable_and_in_palette() {
        let a = fallback_color("someone");
        assert_eq!(a, fallback_color("someone"));
        assert!(DEFAULT_COLORS.contains(&a));
    }

    #[test]
    fn non_privmsg_is_not_a_chat_message() {
        let m = parse_line(":tmi.twitch.tv ROOMSTATE #bar").unwrap();
        assert!(ChatMessage::from_irc(&m).is_none());
    }
}
