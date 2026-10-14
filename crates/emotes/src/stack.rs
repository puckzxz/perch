//! Which emotes written together are drawn together.
//!
//! Some third-party emotes are not pictures of their own but something done
//! to the emote beside them ([`Kind`]), and each provider says which way:
//!
//! - **7TV's zero-width emotes** (`SteerR`, `RainTime`) are laid over the
//!   emote before them, across spaces. With nothing before them, at the
//!   start of a message or after a word, they are drawn as pictures of their
//!   own. 7TV's extension, `src/common/chat/Tokenizer.ts`.
//! - **BetterTTV's zero-width globals** (`cvMask`, `cvHazmat`) are the same,
//!   by a fixed list rather than a flag (Chatterino's `BttvEmotes.cpp`).
//!   BetterTTV's **modifiers** (`w!`, `h!` and the rest) are written *before*
//!   the emote they act on, and one with no emote after it stays the word it
//!   is. BetterTTV's `src/modules/chat/index.js`.
//! - **FrankerFaceZ's modifiers** act on the emote before them, across
//!   spaces, a duplicate skipped. One with the Hidden flag (`ffzX`, `ffzW`)
//!   is an effect and not a picture; one without is laid over it. With no
//!   emote before it, either is drawn as a picture of its own. FrankerFaceZ's
//!   `src/modules/chat/tokenizers.jsx`.
//!
//! All read on 14 October 2026. [`stack`] applies those rules to a message
//! already split into words and emotes: an overlay or an effect moves into
//! the emote it belongs to (`Emote::attached`), and the stack is drawn as
//! one picture. Only spaces join two emotes; any word between breaks the
//! stack, which keeps matching as word-exact as the name lookups are.
//!
//! The walk matches [`Token`] with no wildcard arm, here and in
//! `base_before`, so a new kind of token cannot compile until it says how
//! it stacks. The rule for anything that is not an emote: it releases the
//! BetterTTV modifiers still waiting as their words, is pushed as it is, and
//! breaks adjacency, so nothing before it takes a layer written after it.

use crate::tokenize::{Emote, Kind, Token};

/// Move every overlay and effect in `tokens` into the emote it belongs to,
/// by its provider's rule (see the module). Pure; `message_tokens` runs it
/// last, once the emotes are known.
pub fn stack(tokens: Vec<Token>) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
    // BetterTTV modifiers waiting for the emote after them, and the text
    // they were written as, kept to be given back as words if none comes.
    let mut waiting: Vec<Emote> = Vec::new();
    let mut written = String::new();

    for token in tokens {
        match token {
            Token::Text(text) if text.trim().is_empty() => {
                if waiting.is_empty() {
                    out.push(Token::Text(text));
                } else {
                    written.push_str(&text);
                }
            }
            // A word, or a cheer, which is never a base: what waits for an
            // emote after it goes back as its words, and nothing before it
            // takes a layer written after it (`base_before`).
            other @ (Token::Text(_) | Token::Cheer(_)) => {
                release(&mut waiting, &mut written, &mut out);
                out.push(other);
            }
            Token::Emote(emote) => match emote.kind {
                Kind::EffectOnNext => {
                    written.push_str(&emote.name);
                    waiting.push(emote);
                }
                Kind::Overlay | Kind::EffectOnPrevious if waiting.is_empty() => {
                    match base_before(&out) {
                        Some(index) => {
                            // The spaces between it and its base go with it.
                            out.truncate(index + 1);
                            if let Some(Token::Emote(base)) = out.last_mut() {
                                attach(base, emote);
                            }
                        }
                        None => out.push(Token::Emote(emote)),
                    }
                }
                // A picture of its own, and the base for whatever comes
                // next: a plain emote, or an overlay or FrankerFaceZ
                // effect written straight after a BetterTTV modifier,
                // which is what the modifier acts on.
                Kind::Normal | Kind::Overlay | Kind::EffectOnPrevious => {
                    let mut base = emote;
                    for prefix in waiting.drain(..) {
                        attach(&mut base, prefix);
                    }
                    written.clear();
                    out.push(Token::Emote(base));
                }
            },
        }
    }
    release(&mut waiting, &mut written, &mut out);
    out
}

/// Where in `out` the emote is that a layer written now would attach to:
/// the last token, looking back across whitespace only. `None` when a word
/// comes first, or nothing does.
fn base_before(out: &[Token]) -> Option<usize> {
    for (index, token) in out.iter().enumerate().rev() {
        match token {
            Token::Text(text) if text.trim().is_empty() => {}
            Token::Text(_) | Token::Cheer(_) => return None,
            Token::Emote(_) => return Some(index),
        }
    }
    None
}

/// Add `layer` to `base`, unless it is the same picture as the base or as a
/// layer already there. FrankerFaceZ skips a duplicate modifier the same
/// way, and it keeps every picture in one stack under an id of its own.
fn attach(base: &mut Emote, layer: Emote) {
    let seen = layer.url == base.url || base.attached.iter().any(|had| had.url == layer.url);
    if !seen {
        base.attached.push(layer);
    }
}

/// Give BetterTTV modifiers that found no emote after them back as the
/// words they were written as, as BetterTTV leaves them.
fn release(waiting: &mut Vec<Emote>, written: &mut String, out: &mut Vec<Token>) {
    if !waiting.is_empty() {
        waiting.clear();
        out.push(Token::Text(std::mem::take(written)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message_tokens;
    use crate::providers::Provider;
    use crate::tokenize::{apply_named_emotes, tokenize, Cheer, Named};

    /// A lookup over a few emotes of every kind, each its own provider's.
    fn lookup(name: &str) -> Option<Named> {
        let (provider, kind) = match name {
            "KEKW" | "LUL" => (Provider::Bttv, Kind::Normal),
            "SteerR" | "RainTime" => (Provider::SevenTv, Kind::Overlay),
            "cvMask" => (Provider::Bttv, Kind::Overlay),
            "ffzX" | "ffzW" => (Provider::Ffz, Kind::EffectOnPrevious),
            "w!" | "h!" => (Provider::Bttv, Kind::EffectOnNext),
            _ => return None,
        };
        Some(Named {
            url: format!("https://example.invalid/{name}"),
            provider,
            kind,
        })
    }

    fn tokens(text: &str) -> Vec<Token> {
        message_tokens(text, None, false, None, &|_| None, &lookup)
    }

    /// `Cheer` with an amount, any amount: the line's Bits decide what is
    /// drawn, and a cheer lookup proper is `perch::chat_cheers`.
    fn cheer(word: &str) -> Option<Cheer> {
        let amount = word.strip_prefix("Cheer")?.parse().ok()?;
        Some(Cheer {
            name: word.to_string(),
            amount,
            url: "https://example.invalid/cheer".into(),
            color: 0x979797,
        })
    }

    /// A message that spent `bits`.
    fn cheered(text: &str, bits: u32) -> Vec<Token> {
        message_tokens(text, None, false, Some(bits), &cheer, &lookup)
    }

    /// Each token as a short string: `T:` and its text, or `E:` and the
    /// emote's name with what is attached to it after a `+`.
    fn shape(tokens: &[Token]) -> Vec<String> {
        tokens
            .iter()
            .map(|token| match token {
                Token::Text(text) => format!("T:{text}"),
                Token::Emote(emote) => {
                    let mut out = format!("E:{}", emote.name);
                    for layer in &emote.attached {
                        out.push('+');
                        out.push_str(&layer.name);
                    }
                    out
                }
                Token::Cheer(cheer) => format!("C:{}", cheer.name),
            })
            .collect()
    }

    #[test]
    fn an_overlay_sits_on_the_emote_before_it() {
        assert_eq!(shape(&tokens("KEKW SteerR")), ["E:KEKW+SteerR"]);
        assert_eq!(
            shape(&tokens("look KEKW  SteerR here")),
            ["T:look ", "E:KEKW+SteerR", "T: here"]
        );
        assert_eq!(shape(&tokens("LUL cvMask")), ["E:LUL+cvMask"]);
    }

    /// At the start of a message, or after a word, an overlay is a picture
    /// of its own.
    #[test]
    fn an_overlay_with_no_emote_before_it_is_drawn_alone() {
        assert_eq!(shape(&tokens("SteerR KEKW")), ["E:SteerR", "T: ", "E:KEKW"]);
        assert_eq!(shape(&tokens("lol SteerR")), ["T:lol ", "E:SteerR"]);
    }

    #[test]
    fn a_word_between_breaks_the_stack() {
        assert_eq!(
            shape(&tokens("KEKW and SteerR")),
            ["E:KEKW", "T: and ", "E:SteerR"]
        );
    }

    /// Overlays go on in the order written, each picture once: a repeat of
    /// one already in the stack, or of the base itself, is dropped.
    #[test]
    fn overlays_stack_in_order_once_each() {
        assert_eq!(
            shape(&tokens("KEKW SteerR RainTime SteerR")),
            ["E:KEKW+SteerR+RainTime"]
        );
        assert_eq!(shape(&tokens("SteerR SteerR")), ["E:SteerR"]);
    }

    /// Twitch's own emotes, placed by the tag, take overlays like any other.
    #[test]
    fn an_overlay_sits_on_a_twitch_emote() {
        let stacked = message_tokens(
            "Kappa SteerR",
            Some("25:0-4"),
            false,
            None,
            &|_| None,
            &lookup,
        );
        assert_eq!(shape(&stacked), ["E:Kappa+SteerR"]);
        let Token::Emote(kappa) = &stacked[0] else {
            panic!("not an emote");
        };
        assert_eq!(kappa.provider, Provider::Twitch);
        assert_eq!(kappa.attached[0].provider, Provider::SevenTv);
    }

    /// A FrankerFaceZ effect goes on the emote before it, and with none
    /// there it is a picture of its own, as FrankerFaceZ draws it.
    #[test]
    fn an_ffz_effect_attaches_back_or_stands_alone() {
        assert_eq!(shape(&tokens("KEKW ffzX ffzW")), ["E:KEKW+ffzX+ffzW"]);
        assert_eq!(shape(&tokens("ffzX")), ["E:ffzX"]);
        assert_eq!(shape(&tokens("so ffzX")), ["T:so ", "E:ffzX"]);
    }

    /// A BetterTTV effect goes on the emote after it, several in a row
    /// included; with none there it stays the word it was written as.
    #[test]
    fn a_bttv_effect_attaches_forward_or_stays_a_word() {
        assert_eq!(shape(&tokens("w! KEKW")), ["E:KEKW+w!"]);
        assert_eq!(shape(&tokens("w! h! KEKW")), ["E:KEKW+w!+h!"]);
        assert_eq!(shape(&tokens("w! hello")), ["T:w!", "T: hello"]);
        assert_eq!(shape(&tokens("w!  h! hello")), ["T:w!  h!", "T: hello"]);
        assert_eq!(shape(&tokens("KEKW w!")), ["E:KEKW", "T: ", "T:w!"]);
        // An overlay straight after one is what the effect acts on.
        assert_eq!(shape(&tokens("w! SteerR")), ["E:SteerR+w!"]);
    }

    /// A message with no overlay or effect in it comes out as it went in,
    /// emotes side by side and spaces included.
    #[test]
    fn a_message_without_kinds_is_unchanged() {
        let plain = apply_named_emotes(tokenize("hey KEKW  LUL and LUL", None, false), &lookup);
        assert_eq!(stack(plain.clone()), plain);
        assert_eq!(
            shape(&plain),
            ["T:hey ", "E:KEKW", "T:  ", "E:LUL", "T: and ", "E:LUL"]
        );
    }

    /// Copying a stack's name copies it as it was written, BetterTTV's
    /// effects first; its effects are known by name, in that order too.
    #[test]
    fn typed_writes_the_stack_as_it_was_sent() {
        let stacked = tokens("w! KEKW SteerR ffzX");
        let [Token::Emote(kekw)] = stacked.as_slice() else {
            panic!("not one stack: {:?}", shape(&stacked));
        };
        assert_eq!(kekw.typed(), "w! KEKW SteerR ffzX");
        let effects: Vec<_> = kekw.effects().map(|layer| layer.name.as_str()).collect();
        assert_eq!(effects, ["w!", "ffzX"]);
        let overlays: Vec<_> = kekw.overlays().map(|layer| layer.name.as_str()).collect();
        assert_eq!(overlays, ["SteerR"]);
    }

    /// A lone emote is keyed on its URL, as it always was; a stack on every
    /// picture in it.
    #[test]
    fn a_stack_is_keyed_apart_from_its_base_alone() {
        let lone = tokens("KEKW");
        let stacked = tokens("KEKW SteerR");
        let (Token::Emote(lone), Token::Emote(stacked)) = (&lone[0], &stacked[0]) else {
            panic!("not emotes");
        };
        assert_eq!(lone.id_key(), "https://example.invalid/KEKW");
        assert_eq!(
            stacked.id_key(),
            "https://example.invalid/KEKW https://example.invalid/SteerR"
        );
    }

    /// A cheer is no emote to stack on: an overlay after it is drawn
    /// alone, and a BetterTTV effect before it stays a word.
    #[test]
    fn a_cheer_is_never_a_base() {
        assert_eq!(
            shape(&cheered("Cheer100 SteerR", 100)),
            ["C:Cheer100", "T: ", "E:SteerR"]
        );
        assert_eq!(shape(&cheered("w! Cheer100", 100)), ["T:w! ", "C:Cheer100"]);
    }

    /// Cheers come out of the text before the third-party names are looked
    /// up, and only in a message that spent Bits; without them the same
    /// words are text.
    #[test]
    fn a_message_with_bits_draws_its_cheers_among_its_emotes() {
        assert_eq!(
            shape(&cheered("Cheer100 KEKW", 100)),
            ["C:Cheer100", "T: ", "E:KEKW"]
        );
        assert_eq!(shape(&tokens("Cheer100 KEKW")), ["T:Cheer100 ", "E:KEKW"]);
    }
}
