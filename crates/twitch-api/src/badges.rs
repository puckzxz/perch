//! Chat badges: the pictures and titles behind the names a chat line wears.
//!
//! A chat line names its badges by set and version (`subscriber/12`,
//! `moderator/1`; `twitch_chat::message::Badge`) and nothing else, so what
//! they look like and what they are called come from Helix: Get Global Chat
//! Badges (`/chat/badges/global`), the sets every channel shares, and Get
//! Channel Chat Badges (`/chat/badges?broadcaster_id=`), a channel's own
//! subscriber and bits badges, which take the place of the global ones of
//! the same set and version in that channel. Both take the user's token and
//! need no scope. A channel with no badges of its own answers an empty list.
//!
//! Helix, like the rest of this crate's root, and kept in its own file only
//! because the root is long enough already.

use serde_json::Value;

use super::{entries, helix_get, text, Error};

/// One version of one badge set: what it is called and where its picture is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadgeVersion {
    /// The version, as a chat line names it (`1`, `12`, `3012`).
    pub id: String,
    /// What Twitch calls it, as its own chat's tooltip does: `Moderator`,
    /// `6-Month Subscriber`.
    pub title: String,
    /// The picture at twice its 18-pixel size, which is crisp at the size a
    /// chat line draws it on a display at up to twice the pixels.
    pub image_url: String,
}

/// A badge set: every version of one badge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadgeSet {
    /// The set, as a chat line names it (`subscriber`, `moderator`).
    pub set_id: String,
    pub versions: Vec<BadgeVersion>,
}

/// What a badges answer means. A version with no picture is left out: there
/// would be nothing to draw.
fn parse_badge_sets(json: &Value) -> Vec<BadgeSet> {
    entries(json)
        .filter_map(|entry| {
            let set_id = text(entry, "set_id")?.to_string();
            let versions = entry
                .get("versions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|version| {
                    let image_url = text(version, "image_url_2x")
                        .or_else(|| text(version, "image_url_1x"))
                        .filter(|url| !url.is_empty())?;
                    Some(BadgeVersion {
                        id: text(version, "id")?.to_string(),
                        title: text(version, "title").unwrap_or_default().to_string(),
                        image_url: image_url.to_string(),
                    })
                })
                .collect();
            Some(BadgeSet { set_id, versions })
        })
        .collect()
}

/// Every channel's badges: moderator, VIP, the global subscriber picture,
/// Prime, the event badges.
pub fn global_chat_badges(client_id: &str, token: &str) -> Result<Vec<BadgeSet>, Error> {
    let json = helix_get(client_id, token, "/chat/badges/global", &[])?;
    Ok(parse_badge_sets(&json))
}

/// One channel's own badges, by its numeric id (the `room-id` of its chat):
/// its subscriber and bits pictures, which stand in for the global ones of
/// the same set and version in its chat.
pub fn channel_chat_badges(
    client_id: &str,
    token: &str,
    broadcaster_id: &str,
) -> Result<Vec<BadgeSet>, Error> {
    let json = helix_get(
        client_id,
        token,
        "/chat/badges",
        &[("broadcaster_id", broadcaster_id)],
    )?;
    Ok(parse_badge_sets(&json))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// An answer in the shape Helix's reference gives, with one version that
    /// has no picture and one set with no versions at all.
    #[test]
    fn a_badges_answer_is_sets_of_versions() {
        let body = json!({ "data": [
            { "set_id": "subscriber", "versions": [
                { "id": "0", "title": "Subscriber", "description": "Subscriber",
                  "image_url_1x": "https://x.test/0/1", "image_url_2x": "https://x.test/0/2",
                  "image_url_4x": "https://x.test/0/3" },
                { "id": "3", "title": "3-Month Subscriber",
                  "image_url_1x": "https://x.test/3/1" },
                { "id": "6", "title": "6-Month Subscriber" }
            ]},
            { "set_id": "empty", "versions": [] },
            { "versions": [] }
        ]});
        let sets = parse_badge_sets(&body);
        assert_eq!(sets.len(), 2, "a set without an id is nothing to key on");
        assert_eq!(sets[0].set_id, "subscriber");
        assert_eq!(
            sets[0].versions,
            [
                BadgeVersion {
                    id: "0".into(),
                    title: "Subscriber".into(),
                    image_url: "https://x.test/0/2".into(),
                },
                // No 2x: the 1x stands in.
                BadgeVersion {
                    id: "3".into(),
                    title: "3-Month Subscriber".into(),
                    image_url: "https://x.test/3/1".into(),
                },
            ]
        );
        assert!(sets[1].versions.is_empty());
        assert!(parse_badge_sets(&json!({ "data": [] })).is_empty());
    }
}
