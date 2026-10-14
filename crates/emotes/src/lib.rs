//! Emote resolution and image caching for chat.
//!
//! Five parts, each independent of any UI:
//!
//! - [`tokenize`](mod@tokenize) turns a chat message plus its `emotes` tag
//!   into text and emote runs. Pure logic, no network.
//! - [`providers`] fetches FrankerFaceZ / BetterTTV / 7TV name lookups, which
//!   have no positional data and so are matched word-exact, and what kind
//!   of emote each is.
//! - [`stack`](mod@stack) decides which emotes written together are drawn
//!   together: an overlay on the emote under it, an effect on the emote it
//!   changes. Pure.
//! - [`effects`] works out what those effects do to an emote: its decoded
//!   frames flipped, turned, recoloured or shaken, and where it goes. Pure.
//! - [`cache`] downloads images to disk, because GPUI's `img` takes a path.
//!
//! [`message_tokens`] runs the pure passes over a message in their order.

pub mod cache;
pub mod effects;
pub mod providers;
pub mod stack;
pub mod tokenize;

pub use cache::ImageCache;
pub use effects::{Effect, Effects};
pub use providers::{EmoteLoader, EmoteSets, Provider};
pub use stack::stack;
pub use tokenize::{apply_named_emotes, tokenize, twitch_emote_url, Emote, Kind, Named, Token};

/// A message as a line draws it: its text split into words and emotes,
/// Twitch's from the `emotes` tag, then the third-party ones by `lookup`,
/// then each overlay and effect moved into the emote it belongs to.
///
/// The one place the order of the passes is written. Each depends on the one
/// before: names are only looked up in the text Twitch's tag left, and
/// stacking needs every emote's kind. `animated` picks Twitch's animated
/// variant where it has one.
pub fn message_tokens(
    text: &str,
    tag: Option<&str>,
    animated: bool,
    lookup: &impl Fn(&str) -> Option<Named>,
) -> Vec<Token> {
    stack(apply_named_emotes(tokenize(text, tag, animated), lookup))
}
