//! A Twitch account, as a chat's viewer card shows it: its picture, when it
//! was made, and whether its channel is a partner's or an affiliate's.
//!
//! Helix's Get Users (`/users?login=`), with the user's token and no scope:
//! only the email needs one (`user:read:email`), and it is not asked for.
//! It takes up to a hundred logins at once, repeated as `login=` pairs, and
//! leaves out anyone it has no account for (renamed, suspended, deleted, or
//! a word that was never a login), which is how [`user`] tells an account
//! from none, as `user_id_for` does.
//!
//! Helix, like the rest of this crate's root, and kept in its own file only
//! because the root is long enough already. `fixtures/users.json` is an
//! answer in the shape Twitch's reference gives, and what the parser is
//! tested against.

use serde_json::Value;

use super::{entries, helix_get, text, text_or_empty, Error};

/// What Twitch says a channel is, beyond being one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BroadcasterType {
    Partner,
    Affiliate,
    /// Neither: Helix's empty string, or a word it has since invented.
    Other,
}

impl BroadcasterType {
    /// Helix's word for it, as its `broadcaster_type` field writes it.
    fn from_helix(word: &str) -> Self {
        match word {
            "partner" => BroadcasterType::Partner,
            "affiliate" => BroadcasterType::Affiliate,
            _ => BroadcasterType::Other,
        }
    }
}

/// One account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: String,
    pub login: String,
    /// The name as the person writes it, in their own case; the login when
    /// Helix left it empty.
    pub display_name: String,
    /// The picture at Twitch's 300-pixel size, or empty when there is none;
    /// see [`profile_image_at`](super::profile_image_at) for another size.
    pub profile_image_url: String,
    /// When the account was made, in RFC 3339 (UTC).
    pub created_at: String,
    pub broadcaster_type: BroadcasterType,
}

/// What a users answer means. An entry with no login is left out: there is
/// nobody to key it on.
fn parse_users(json: &Value) -> Vec<User> {
    entries(json)
        .filter_map(|entry| {
            let login = text(entry, "login").filter(|login| !login.is_empty())?;
            Some(User {
                id: text_or_empty(entry, "id"),
                login: login.to_string(),
                display_name: text(entry, "display_name")
                    .filter(|name| !name.is_empty())
                    .unwrap_or(login)
                    .to_string(),
                profile_image_url: text_or_empty(entry, "profile_image_url"),
                created_at: text_or_empty(entry, "created_at"),
                broadcaster_type: BroadcasterType::from_helix(
                    text(entry, "broadcaster_type").unwrap_or_default(),
                ),
            })
        })
        .collect()
}

/// The account `login` names, or `None` when Twitch has none by that name.
pub fn user(client_id: &str, token: &str, login: &str) -> Result<Option<User>, Error> {
    let json = helix_get(client_id, token, "/users", &[("login", login)])?;
    Ok(parse_users(&json).into_iter().next())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A users answer in the shape Twitch's reference gives, without the
    /// email and the view count, beside an account with no picture and an
    /// entry with no login.
    const USERS_ANSWER: &str = include_str!("fixtures/users.json");

    #[test]
    fn a_users_answer_is_people() {
        let json: Value = serde_json::from_str(USERS_ANSWER).unwrap();
        assert_eq!(
            parse_users(&json),
            [
                User {
                    id: "141981764".into(),
                    login: "twitchdev".into(),
                    display_name: "TwitchDev".into(),
                    profile_image_url: "https://static-cdn.jtvnw.net/jtv_user_pictures/8a6381c7-d0c0-4576-b179-38bd5ce1d6af-profile_image-300x300.png".into(),
                    created_at: "2016-12-14T20:32:28Z".into(),
                    broadcaster_type: BroadcasterType::Partner,
                },
                User {
                    id: "52834821".into(),
                    login: "quietfisher".into(),
                    display_name: "quietfisher".into(),
                    profile_image_url: String::new(),
                    created_at: "2013-10-30T02:15:07Z".into(),
                    broadcaster_type: BroadcasterType::Affiliate,
                },
                User {
                    id: "780412933".into(),
                    login: "newcomer_42".into(),
                    display_name: "Newcomer_42".into(),
                    profile_image_url: "https://static-cdn.jtvnw.net/user-default-pictures-uv/de130ab0-def7-11e9-b668-784f43822e80-profile_image-300x300.png".into(),
                    created_at: "2026-10-02T09:41:55Z".into(),
                    broadcaster_type: BroadcasterType::Other,
                },
            ]
        );
        assert!(parse_users(&json!({ "data": [] })).is_empty());
        assert!(parse_users(&json!({})).is_empty());
    }
}
