//! Emote resolution and image caching for chat.
//!
//! Splits into two halves that stay independent of any UI:
//!
//! - [`tokenize`] turns a chat message plus its `emotes` tag into text and
//!   emote runs. Pure logic, no network.
//! - [`providers`] fetches FrankerFaceZ / BetterTTV / 7TV name lookups, which
//!   have no positional data and so are matched word-exact.
//! - [`cache`] downloads images to disk, because GPUI's `img` takes a path.

pub mod cache;
pub mod providers;
pub mod tokenize;

pub use cache::ImageCache;
pub use providers::{EmoteLoader, EmoteSets};
pub use tokenize::{apply_named_emotes, tokenize, twitch_emote_url, Emote, Token};
