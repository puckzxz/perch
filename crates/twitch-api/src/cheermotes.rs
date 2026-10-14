//! Cheermotes: the pictures a cheer is drawn as.
//!
//! A chat line that spent Bits carries a `bits` tag and words such as
//! `Cheer100` (`twitch_chat::ChatMessage::bits`), and nothing says what they
//! look like. Helix's Get Cheermotes (`/bits/cheermotes`) does: each prefix
//! (`Cheer`, `BibleThump`, a channel's own) with its tiers, the least Bits
//! each takes, its colour and its pictures by theme, motion and size. Asked
//! with a channel's id it answers the global cheermotes and that channel's
//! own together, so one ask per channel is the whole of what its chat can
//! draw. It takes the user's token and needs no scope.
//!
//! Helix, like the rest of this crate's root, and kept in its own file only
//! because the root is long enough already.

use serde_json::Value;

use super::{entries, helix_get, text, Error};

/// One tier of a cheermote: the least Bits a cheer must be to be drawn as
/// it, and what it is drawn with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheerTier {
    pub min_bits: u32,
    /// 0xRRGGBB, from Helix's `#979797`: what the amount after the picture
    /// is written in, as Twitch's own chat does.
    pub color: u32,
    /// The dark theme's animated picture at size `2`, twice its 28-pixel
    /// size, which is crisp at the size a chat line draws it on a display at
    /// up to twice the pixels; a smaller or still one where Helix has no
    /// such picture.
    pub image_url: String,
}

/// A cheermote: the word a cheer starts with and its tiers, ascending by
/// `min_bits`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cheermote {
    /// As Helix writes it (`Cheer`, `4Head`); a cheer may write it in any
    /// case.
    pub prefix: String,
    pub tiers: Vec<CheerTier>,
}

/// `#rrggbb` as 0xRRGGBB, or `None` for anything else.
fn hex_color(raw: &str) -> Option<u32> {
    let hex = raw.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

/// The picture a tier is drawn as: dark, animated, at size `2`, then `1`,
/// then the same sizes still. A chat is dark, and a cheer moves on Twitch.
fn image_url(tier: &Value) -> Option<&str> {
    let dark = tier.get("images")?.get("dark")?;
    ["animated", "static"]
        .iter()
        .filter_map(|motion| dark.get(motion))
        .flat_map(|sizes| ["2", "1"].map(|size| text(sizes, size)))
        .flatten()
        .find(|url| !url.is_empty())
}

/// What a cheermotes answer means. Left out: an entry Helix marks
/// `display_only` (reserved for its own use, its reference says, and not to
/// be used; Chatterino keeps every type, and which prefixes are typed so is
/// yet to be seen, HANDOFF known limit 38), one with no prefix, a tier
/// missing its least Bits, its colour or a picture, and an entry left with
/// no tier. `can_cheer` and `show_in_bits_card` are not read: they say what
/// Twitch's Bits card offers, not what a line that already cheered is drawn
/// as.
fn parse_cheermotes(json: &Value) -> Vec<Cheermote> {
    entries(json)
        .filter(|entry| text(entry, "type") != Some("display_only"))
        .filter_map(|entry| {
            let prefix = text(entry, "prefix").filter(|prefix| !prefix.is_empty())?;
            let mut tiers: Vec<CheerTier> = entry
                .get("tiers")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|tier| {
                    Some(CheerTier {
                        min_bits: u32::try_from(tier.get("min_bits")?.as_u64()?).ok()?,
                        color: hex_color(text(tier, "color")?)?,
                        image_url: image_url(tier)?.to_string(),
                    })
                })
                .collect();
            if tiers.is_empty() {
                return None;
            }
            tiers.sort_by_key(|tier| tier.min_bits);
            Some(Cheermote {
                prefix: prefix.to_string(),
                tiers,
            })
        })
        .collect()
}

/// The cheermotes a chat in the channel `broadcaster_id` (the `room-id` of
/// its chat) can draw: the global ones and the channel's own, in the order
/// Helix gives them.
pub fn cheermotes(
    client_id: &str,
    token: &str,
    broadcaster_id: &str,
) -> Result<Vec<Cheermote>, Error> {
    let json = helix_get(
        client_id,
        token,
        "/bits/cheermotes",
        &[("broadcaster_id", broadcaster_id)],
    )?;
    Ok(parse_cheermotes(&json))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Pictures for a tier as Helix nests them, with the sizes given.
    fn images(animated: &[(&str, &str)], still: &[(&str, &str)]) -> Value {
        let sizes = |pairs: &[(&str, &str)]| {
            Value::Object(
                pairs
                    .iter()
                    .map(|(size, url)| ((*size).to_string(), json!(url)))
                    .collect(),
            )
        };
        json!({
            "dark": { "animated": sizes(animated), "static": sizes(still) },
            "light": { "animated": { "2": "https://x.test/light" }, "static": {} }
        })
    }

    /// An answer in the shape Helix's reference gives: the tiers out of
    /// order, pictures missing at some sizes, and entries and tiers that
    /// cannot be drawn.
    #[test]
    fn a_cheermotes_answer_is_prefixes_and_tiers() {
        let body = json!({ "data": [
            { "prefix": "Cheer", "type": "global_first_party", "order": 1, "tiers": [
                { "min_bits": 100, "id": "100", "color": "#9c3ee8",
                  "images": images(&[("1", "https://x.test/100/1")], &[("2", "https://x.test/100/s2")]),
                  "can_cheer": true, "show_in_bits_card": true },
                { "min_bits": 1, "id": "1", "color": "#979797",
                  "images": images(&[("1", "https://x.test/1/1"), ("2", "https://x.test/1/2")], &[]),
                  "can_cheer": true, "show_in_bits_card": true },
                { "min_bits": 1000, "id": "1000", "color": "#1db2a5",
                  "images": images(&[], &[("1", "https://x.test/1000/s1")]),
                  "can_cheer": false, "show_in_bits_card": false },
                { "min_bits": 5000, "id": "5000", "color": "0099fe",
                  "images": images(&[("2", "https://x.test/5000/2")], &[]) },
                { "min_bits": 10000, "id": "10000", "color": "#f43021" }
            ]},
            { "prefix": "Internal", "type": "display_only", "tiers": [
                { "min_bits": 1, "color": "#979797", "images": images(&[("2", "u")], &[]) }
            ]},
            { "prefix": "", "type": "global_third_party", "tiers": [
                { "min_bits": 1, "color": "#979797", "images": images(&[("2", "u")], &[]) }
            ]},
            { "prefix": "Empty", "type": "channel_custom", "tiers": [] }
        ]});
        let cheermotes = parse_cheermotes(&body);
        assert_eq!(
            cheermotes,
            [Cheermote {
                prefix: "Cheer".into(),
                tiers: vec![
                    // Dark animated `2`, chosen over `1`.
                    CheerTier {
                        min_bits: 1,
                        color: 0x979797,
                        image_url: "https://x.test/1/2".into(),
                    },
                    // No animated `2`: the animated `1`, over a still `2`.
                    CheerTier {
                        min_bits: 100,
                        color: 0x9c3ee8,
                        image_url: "https://x.test/100/1".into(),
                    },
                    // Nothing animated: the still picture.
                    CheerTier {
                        min_bits: 1000,
                        color: 0x1db2a5,
                        image_url: "https://x.test/1000/s1".into(),
                    },
                    // The colour without its `#` and the tier with no
                    // pictures are gone.
                ],
            }]
        );
        assert!(parse_cheermotes(&json!({ "data": [] })).is_empty());
    }
}
