//! What FrankerFaceZ's and BetterTTV's effects do to an emote, worked out
//! on its decoded frames.
//!
//! BetterTTV draws both providers' effects, and maps FrankerFaceZ's `ffzW`,
//! `ffzX`, `ffzY` and `ffzCursed` onto its own `w!`, `h!`, `v!` and `c!`
//! (`EMOTE_MODIFIERS` in its `src/modules/chat/index.js`). Each is a class on
//! the emote, whose rule is in its `src/modules/chat/style.css` (both read on
//! 14 October 2026), every length at its 28 px emote height:
//!
//! - `w!`, wide: `width: 112px; height: 28px; object-fit: fill`.
//! - `h!` and `v!`: `transform: scaleX(-1)` and `scaleY(-1)`.
//! - `l!` and `r!`: `transform: rotate(-90deg)` and `rotate(90deg)`, with
//!   `max-width: 28px`.
//! - `z!`, zero space: `margin-left: -4px`.
//! - `c!`, cursed: `filter: grayscale(1) brightness(0.7) contrast(2.5)`.
//! - `p!`, party: `sepia(0.5) hue-rotate(0deg) saturate(2.5)` to the same
//!   at `360deg`, linear, every 1.5 s.
//! - `s!`, shake: a `translate` of up to 2 px either way, through ten
//!   keyframes in 500 ms, `step-start`.
//!
//! The classes all land on one element, so where two set the same property
//! the stylesheet's cascade keeps one, and [`Effects`] keeps the same one. Of
//! the flips and turns, all `transform`, only the last rule in the
//! stylesheet applies: `r!` over `l!` over `v!` over `h!`. Party's animated
//! filter outranks cursed's. A turn's `max-width` holds wide's 112 px to the
//! turn's square. Shake is `translate`, a property of its own, and party and
//! shake together have a rule that runs both.
//!
//! gpui's image sprite has no transform (`PolychromeSprite`), so none of
//! this is the GPU's: flips and turns move the decoded pixels, the filters
//! recolour them (`filter`), party and shake are frames made over time,
//! and wide and zero space are only where the picture goes ([`Placement`],
//! [`WIDE`], [`ZERO_SPACE`]). [`apply`] is pure; the chat runs it off the UI
//! thread on what [`decode`] reads, and draws the frames that come back as it
//! draws any animated emote.

mod decode;
mod filter;

pub use decode::decode;

use std::time::Duration;

use image::{imageops, Delay, Frame, RgbaImage};

use crate::tokenize::Emote;
use filter::Filter;

/// BetterTTV's emote height, in its stylesheet's px: every length below is
/// a share of it. Perch draws an emote 28 px tall at the default text size
/// too, and scales it with the text, so each share scales with it.
const EMOTE_PX: f32 = 28.0;

/// How wide `w!` draws its emote, in emote heights: 112 px at 28.
pub const WIDE: f32 = 112.0 / EMOTE_PX;

/// How far `z!` pulls its emote into the space before it, in emote heights:
/// 4 px at 28.
pub const ZERO_SPACE: f32 = 4.0 / EMOTE_PX;

/// One turn of party's colour wheel, in µs.
const PARTY_PERIOD: u64 = 1_500_000;

/// How long party holds a hue, in µs, where a frame would hold it longer: a
/// step of 15 degrees every 62.5 ms, 24 to a turn.
const PARTY_STEP: u64 = PARTY_PERIOD / 24;

/// How long shake holds each offset, in µs.
const SHAKE_STEP: u64 = 50_000;

/// One run of shake through its ten offsets, in µs: 500 ms.
const SHAKE_PERIOD: u64 = SHAKE_STEP * SHAKE.len() as u64;

/// How many bytes of frames party or shake may make of an animated emote by
/// running its loop again ([`timeline`]). Every frame made is a whole RGBA
/// picture, and an atlas tile once drawn, so this is what one emote's repeat
/// may cost: 256 frames of a 56 px emote shaken, its canvas 64 px square. It
/// bounds one picture only; the chat pane that keeps them holds all of its
/// made pictures to a total of its own.
const MADE_BYTES: u64 = 4 << 20;

/// Shake's offsets in turn, in px, each held for [`SHAKE_STEP`]: the
/// keyframes at 10% to 100% of BetterTTV's. Its timing is `step-start`,
/// which gives each step the value of the keyframe that ends it, so the one
/// at 0%, (0, 1), is never held.
const SHAKE: [(i32, i32); 10] = [
    (2, 0),
    (1, -2),
    (-2, 1),
    (0, -1),
    (2, 2),
    (-1, -1),
    (-2, 2),
    (2, 1),
    (-1, -2),
    (1, 0),
];

/// The furthest shake moves a picture on either axis, in px: the room its
/// canvas needs on every side, so nothing it moves is cut off.
const SHAKE_REACH: f32 = 2.0;

/// One effect, by the name it is written as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// `w!`, `ffzW`: four emote heights wide, the picture stretched to it.
    Wide,
    /// `h!`, `ffzX`: mirrored left to right.
    FlipX,
    /// `v!`, `ffzY`: upside down.
    FlipY,
    /// `l!`: a quarter turn anticlockwise.
    TurnLeft,
    /// `r!`: a quarter turn clockwise.
    TurnRight,
    /// `z!`: pulled into the space before it.
    ZeroSpace,
    /// `c!`, `ffzCursed`: grey, darker and harder.
    Cursed,
    /// `p!`: its colours cycled round the wheel.
    Party,
    /// `s!`: shaken.
    Shake,
}

impl Effect {
    /// The effect written as `name`, as BetterTTV knows it; `None` for one
    /// it does not draw (a FrankerFaceZ effect added since, say), which is
    /// still left out of the line and named in the emote's tooltip.
    pub fn named(name: &str) -> Option<Self> {
        Some(match name {
            "w!" | "ffzW" => Self::Wide,
            "h!" | "ffzX" => Self::FlipX,
            "v!" | "ffzY" => Self::FlipY,
            "l!" => Self::TurnLeft,
            "r!" => Self::TurnRight,
            "z!" => Self::ZeroSpace,
            "c!" | "ffzCursed" => Self::Cursed,
            "p!" => Self::Party,
            "s!" => Self::Shake,
            _ => return None,
        })
    }
}

/// The one `transform` an emote's flips and turns come to. In the
/// stylesheet's order, so the greatest is the one it keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Turn {
    FlipX,
    FlipY,
    Left,
    Right,
}

/// The one `filter` an emote's colour comes from. Party's is an
/// animation's, which outranks cursed's in the cascade, so the greatest is
/// the one kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tint {
    Cursed,
    Party,
}

/// Everything done to one emote, from the effects written against it,
/// settled as BetterTTV's stylesheet settles them (see the module). The
/// same effects in any order, or twice, are the same `Effects`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Effects {
    pub wide: bool,
    pub turn: Option<Turn>,
    pub zero_space: bool,
    pub tint: Option<Tint>,
    pub shake: bool,
}

impl FromIterator<Effect> for Effects {
    fn from_iter<I: IntoIterator<Item = Effect>>(effects: I) -> Self {
        let mut settled = Self::default();
        for effect in effects {
            match effect {
                Effect::Wide => settled.wide = true,
                Effect::FlipX => settled.turn = settled.turn.max(Some(Turn::FlipX)),
                Effect::FlipY => settled.turn = settled.turn.max(Some(Turn::FlipY)),
                Effect::TurnLeft => settled.turn = settled.turn.max(Some(Turn::Left)),
                Effect::TurnRight => settled.turn = settled.turn.max(Some(Turn::Right)),
                Effect::ZeroSpace => settled.zero_space = true,
                Effect::Cursed => settled.tint = settled.tint.max(Some(Tint::Cursed)),
                Effect::Party => settled.tint = settled.tint.max(Some(Tint::Party)),
                Effect::Shake => settled.shake = true,
            }
        }
        settled
    }
}

impl Effects {
    /// What the effects written against `emote` do to it, those BetterTTV
    /// knows; one it does not know does nothing to the picture.
    pub fn of(emote: &Emote) -> Self {
        emote
            .effects()
            .filter_map(|layer| Effect::named(&layer.name))
            .collect()
    }

    /// Whether the picture itself changes, and not only where it goes: what
    /// decides whether the chat makes new frames with [`apply`] or draws the
    /// emote's own, placed by [`WIDE`] and [`ZERO_SPACE`].
    pub fn changes_frames(&self) -> bool {
        self.turn.is_some() || self.tint.is_some() || self.shake
    }

    /// Where a picture `width` by `height` px, already turned, goes: in a
    /// turn's square, fitted to it with its shape kept, or stretched to it
    /// with wide, whose 112 px a turn's `max-width` holds to 28; across
    /// wide's box, stretched; or at its own shape.
    fn placement(&self, width: u32, height: u32) -> Placement {
        let (w, h) = (width.max(1) as f32, height.max(1) as f32);
        if matches!(self.turn, Some(Turn::Left | Turn::Right)) {
            let picture = if self.wide {
                Rect::boxed(1.0)
            } else {
                let scale = 1.0 / w.max(h);
                let (width, height) = (w * scale, h * scale);
                Rect {
                    x: (1.0 - width) / 2.0,
                    y: (1.0 - height) / 2.0,
                    width,
                    height,
                }
            };
            Placement {
                width: 1.0,
                picture,
            }
        } else if self.wide {
            Placement {
                width: WIDE,
                picture: Rect::boxed(WIDE),
            }
        } else {
            Placement {
                width: w / h,
                picture: Rect::boxed(w / h),
            }
        }
    }
}

/// Where an emote's picture goes in its line, in emote heights: the chat
/// multiplies by its own emote height, as BetterTTV's lengths are at its
/// 28 px.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// How much of the line the emote takes across: its box, which is one
    /// emote height tall, as every emote's is.
    pub width: f32,
    /// Where the frames are drawn, from the box's top left, stretched to
    /// fill this: the box itself, a picture fitted inside a turn's square,
    /// or the box and a shake's reach on every side.
    pub picture: Rect,
}

/// A rectangle in emote heights.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    /// The whole of a box `width` across.
    fn boxed(width: f32) -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width,
            height: 1.0,
        }
    }
}

/// An emote's frames as its effects make them, and where they go.
pub struct Changed {
    /// RGBA, each composited onto the whole picture, with its delay.
    pub frames: Vec<Frame>,
    pub placement: Placement,
}

/// Make `frames`, an emote's decoded frames ([`decode`]), into the frames
/// `effects` draw, and say where they go. Turned first, then recoloured,
/// then shaken, which is the order the browser's `translate`, `transform`
/// and `filter` come to for these: a filter is per pixel and a turn is by
/// whole pixels, so those two commute, and shake moves what they made.
pub fn apply(frames: Vec<Frame>, effects: Effects) -> Changed {
    let frames: Vec<Frame> = frames
        .into_iter()
        .map(|frame| turned(frame, effects.turn))
        .collect();
    let (width, height) = frames
        .first()
        .map_or((1, 1), |frame| frame.buffer().dimensions());
    let placement = effects.placement(width, height);
    let shake = effects
        .shake
        .then(|| Shake::of(placement.picture, width, height));
    // What one frame made costs, its canvas widened by the shake's margin.
    let (made_width, made_height) = shake.map_or((width, height), |shake| {
        (width + 2 * shake.margin.0, height + 2 * shake.margin.1)
    });
    let frame_bytes = u64::from(made_width) * u64::from(made_height) * 4;
    let budget = (MADE_BYTES / frame_bytes.max(1)) as usize;
    let made = timeline(&frames, effects, budget)
        .into_iter()
        .map(|(source, start, delay)| {
            let mut buffer = frames[source].buffer().clone();
            match effects.tint {
                Some(Tint::Cursed) => Filter::cursed().apply(&mut buffer),
                Some(Tint::Party) => Filter::party(party_degrees(start)).apply(&mut buffer),
                None => {}
            }
            if let Some(shake) = shake {
                buffer = shake.moved(&buffer, shake_at(start));
            }
            Frame::from_parts(buffer, 0, 0, delay)
        })
        .collect();
    Changed {
        frames: made,
        placement: match shake {
            Some(shake) => shake.widened(placement),
            None => placement,
        },
    }
}

/// `frame` flipped or turned by `turn`. A turn swaps its width and height.
fn turned(frame: Frame, turn: Option<Turn>) -> Frame {
    let Some(turn) = turn else {
        return frame;
    };
    let delay = frame.delay();
    let mut buffer = frame.into_buffer();
    let buffer = match turn {
        Turn::FlipX => {
            imageops::flip_horizontal_in_place(&mut buffer);
            buffer
        }
        Turn::FlipY => {
            imageops::flip_vertical_in_place(&mut buffer);
            buffer
        }
        // CSS turns clockwise for a positive angle, as `rotate90` does.
        Turn::Left => imageops::rotate270(&buffer),
        Turn::Right => imageops::rotate90(&buffer),
    };
    Frame::from_parts(buffer, 0, 0, delay)
}

/// The frames to draw, in order: which decoded frame each shows, when in
/// the loop it starts (µs), and how long it shows.
///
/// Party's hue and shake's offset run on a clock of their own, as a CSS
/// animation runs beside the picture's frames, so each frame takes them from
/// when it starts, modulo their periods, and a frame that would hold one past
/// its next step is cut there, into pieces that show the same picture: at
/// shake's 50 ms steps, or with party alone its 62.5 ms ones. An animated
/// emote so keeps its own pace, its delays adding up as they did, and a slow
/// one still shakes and turns colour. A still one is a frame held for a
/// whole period, cut the same way: party's 1.5 s into 24 frames, shake's
/// 500 ms into its ten, and with both, shake's steps over party's 1.5 s, the
/// hue taken at each.
///
/// The made loop is what gpui repeats, so it has to end where the period
/// does, or the hue or offset jumps back as it comes round; and an animated
/// emote's own loop rarely does. Made over one run of it, a 120 ms loop with
/// party reached 22 degrees and snapped back, and with shake showed two of
/// the ten offsets. So the emote's loop is run again, each frame still the
/// emote's own and the clock running on, until the two end together, at
/// their least common multiple: 25 runs of 120 ms with party, 3 s, two turns
/// of the wheel. Within `budget` frames that can be out of reach (a 990 ms
/// loop meets 1.5 s only at 49.5 s), and then the runs stop at the count, of
/// those within the budget, whose end comes nearest a period's, a loop
/// shorter than one period counting as short by all it leaves out
/// ([`loop_end`]); the hue or offset jumps by what is left over: for that
/// 990 ms loop, three runs, 30 ms of the wheel short, 7 degrees.
fn timeline(frames: &[Frame], effects: Effects, budget: usize) -> Vec<(usize, u64, Delay)> {
    let party = effects.tint == Some(Tint::Party);
    let (step, period) = if effects.shake {
        (SHAKE_STEP, if party { PARTY_PERIOD } else { SHAKE_PERIOD })
    } else if party {
        (PARTY_STEP, PARTY_PERIOD)
    } else {
        // Nothing runs on a clock: the frames as they are.
        return frames
            .iter()
            .enumerate()
            .map(|(index, frame)| (index, 0, frame.delay()))
            .collect();
    };
    let lengths: Vec<u64> = match frames {
        [_] => vec![period],
        _ => frames
            .iter()
            .map(|frame| Duration::from(frame.delay()).as_micros() as u64)
            .collect(),
    };

    let mut made = Vec::new();
    let mut start = 0;
    // Where the made loop could end, after each run of the emote's: how long
    // it is then, and how many frames it has.
    let mut ends = Vec::new();
    loop {
        for (index, (&length, frame)) in lengths.iter().zip(frames).enumerate() {
            cut(&mut made, index, start, length, frame.delay(), step);
            start += length;
        }
        ends.push((start, made.len()));
        // Ended with the period, which a still frame always does, as does a
        // loop with no length; or past the budget, where no later end is
        // kept. Every run adds a frame, so one of the two comes.
        if start % period == 0 || made.len() > budget {
            break;
        }
    }
    made.truncate(loop_end(&ends, period, budget));
    made
}

/// Push the piece or pieces of a frame `length` µs long starting `start` µs
/// in, which shows decoded frame `index` and is `delay` long: whole where it
/// holds no longer than a `step`, else cut at every step it would hold past.
fn cut(
    made: &mut Vec<(usize, u64, Delay)>,
    index: usize,
    start: u64,
    length: u64,
    delay: Delay,
    step: u64,
) {
    if length <= step {
        made.push((index, start, delay));
        return;
    }
    let end = start + length;
    let mut at = start;
    while at < end {
        let next = (at / step + 1) * step;
        let piece = next.min(end) - at;
        // A piece is at most a step, so its µs fit.
        made.push((index, at, Delay::from_numer_denom_ms(piece as u32, 1_000)));
        at += piece;
    }
}

/// How many frames the made loop keeps, of `ends`, the length and frame
/// count after each run of the emote's loop, in order. The first run always,
/// as the emote's own loop is never cut short; then only runs within
/// `budget` frames. Of those, the one whose end comes nearest the end of a
/// `period`, the fewest runs on a tie. A loop shorter than one period is as
/// far off as all of the period it leaves out, not as its length: one that
/// stops at 120 ms of 1.5 s is only 120 ms from the period's start, but it
/// never shows the rest of the wheel. So with no run long enough for a whole
/// period, the longest is kept; and a 1.4 s loop is kept as it is, 100 ms
/// short, rather than run twice to 2.8 s, 200 ms short.
fn loop_end(ends: &[(u64, usize)], period: u64, budget: usize) -> usize {
    let within = ends
        .iter()
        .take_while(|(_, count)| *count <= budget)
        .count()
        .max(1)
        .min(ends.len());
    let off = |length: u64| {
        if length < period {
            return period - length;
        }
        let over = length % period;
        over.min(period - over)
    };
    ends[..within]
        .iter()
        .min_by_key(|(length, _)| off(*length))
        .map_or(0, |(_, count)| *count)
}

/// How far round party's colour wheel it is `start` µs in, in degrees.
fn party_degrees(start: u64) -> f32 {
    ((start % PARTY_PERIOD) as f64 / PARTY_PERIOD as f64 * 360.0) as f32
}

/// Shake's offset `start` µs in, in px.
fn shake_at(start: u64) -> (i32, i32) {
    SHAKE[(start / SHAKE_STEP % SHAKE.len() as u64) as usize]
}

/// Shake, measured in the picture's own pixels: BetterTTV's px at the size
/// the picture is drawn, which across a wide box is not the size it is
/// down it.
#[derive(Debug, Clone, Copy)]
struct Shake {
    /// The picture's pixels to one of BetterTTV's px, across and down.
    scale: (f32, f32),
    /// The room added on each side, in the picture's pixels: the reach,
    /// rounded as each offset is, so no offset is more than it.
    margin: (u32, u32),
}

impl Shake {
    /// Shake for a picture `width` by `height` px drawn over `picture`.
    fn of(picture: Rect, width: u32, height: u32) -> Self {
        let scale = (
            width as f32 / (picture.width * EMOTE_PX),
            height as f32 / (picture.height * EMOTE_PX),
        );
        Self {
            scale,
            margin: (
                (SHAKE_REACH * scale.0).round() as u32,
                (SHAKE_REACH * scale.1).round() as u32,
            ),
        }
    }

    /// `buffer` on a canvas wider and taller by the margin on every side,
    /// moved from its middle by `offset`, in BetterTTV's px.
    fn moved(self, buffer: &RgbaImage, (dx, dy): (i32, i32)) -> RgbaImage {
        let (width, height) = buffer.dimensions();
        let (mx, my) = self.margin;
        let mut canvas = RgbaImage::new(width + 2 * mx, height + 2 * my);
        let x = i64::from(mx) + (dx as f32 * self.scale.0).round() as i64;
        let y = i64::from(my) + (dy as f32 * self.scale.1).round() as i64;
        imageops::replace(&mut canvas, buffer, x, y);
        canvas
    }

    /// `placement` with its picture grown by the margin on every side, so
    /// the canvas lands with the picture where it was and the shake hangs
    /// past the box, as a `translate` does.
    fn widened(self, placement: Placement) -> Placement {
        let across = self.margin.0 as f32 / self.scale.0 / EMOTE_PX;
        let down = self.margin.1 as f32 / self.scale.1 / EMOTE_PX;
        let picture = placement.picture;
        Placement {
            width: placement.width,
            picture: Rect {
                x: picture.x - across,
                y: picture.y - down,
                width: picture.width + 2.0 * across,
                height: picture.height + 2.0 * down,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use image::Rgba;

    use super::*;
    use crate::message_tokens;
    use crate::providers::Provider;
    use crate::tokenize::{Kind, Named, Token};

    const RED: Rgba<u8> = Rgba([255, 0, 0, 255]);
    const CLEAR: Rgba<u8> = Rgba([0, 0, 0, 0]);

    /// A still frame `width` by `height`, clear but for red at its top left.
    fn marked(width: u32, height: u32) -> Frame {
        let mut buffer = RgbaImage::from_pixel(width, height, CLEAR);
        buffer.put_pixel(0, 0, RED);
        Frame::new(buffer)
    }

    /// Where the red pixel is in `frame`.
    fn red_at(frame: &Frame) -> (u32, u32) {
        let found = frame
            .buffer()
            .enumerate_pixels()
            .find(|(_, _, pixel)| **pixel == RED)
            .map(|(x, y, _)| (x, y));
        found.expect("no red pixel")
    }

    fn settled(names: &[&str]) -> Effects {
        names
            .iter()
            .map(|name| Effect::named(name).expect("a known effect"))
            .collect()
    }

    fn ms(delay: Delay) -> Duration {
        Duration::from(delay)
    }

    #[test]
    fn effects_are_known_by_either_providers_name() {
        assert_eq!(Effect::named("w!"), Some(Effect::Wide));
        assert_eq!(Effect::named("ffzW"), Some(Effect::Wide));
        assert_eq!(Effect::named("ffzX"), Some(Effect::FlipX));
        assert_eq!(Effect::named("ffzY"), Some(Effect::FlipY));
        assert_eq!(Effect::named("ffzCursed"), Some(Effect::Cursed));
        for (name, effect) in [
            ("h!", Effect::FlipX),
            ("v!", Effect::FlipY),
            ("l!", Effect::TurnLeft),
            ("r!", Effect::TurnRight),
            ("z!", Effect::ZeroSpace),
            ("c!", Effect::Cursed),
            ("p!", Effect::Party),
            ("s!", Effect::Shake),
        ] {
            assert_eq!(Effect::named(name), Some(effect), "{name}");
        }
        assert_eq!(Effect::named("ffzSpin"), None);
        assert_eq!(Effect::named("KEKW"), None);
    }

    /// Two transforms keep the stylesheet's later one, whichever was
    /// written first; party's animation keeps its filter over cursed's;
    /// party and shake run together.
    #[test]
    fn the_stylesheet_keeps_one_transform_and_one_filter() {
        assert_eq!(settled(&["h!", "v!"]).turn, Some(Turn::FlipY));
        assert_eq!(settled(&["v!", "h!"]).turn, Some(Turn::FlipY));
        assert_eq!(settled(&["r!", "l!", "ffzX"]).turn, Some(Turn::Right));
        assert_eq!(settled(&["p!", "c!"]).tint, Some(Tint::Party));
        let both = settled(&["p!", "s!"]);
        assert_eq!((both.tint, both.shake), (Some(Tint::Party), true));
        assert_eq!(settled(&["w!", "z!", "w!"]), settled(&["z!", "ffzW"]));
    }

    /// Wide and zero space are only where the picture goes; the rest change
    /// its frames.
    #[test]
    fn only_some_effects_change_the_frames() {
        assert!(!settled(&["w!", "z!"]).changes_frames());
        for name in ["h!", "l!", "c!", "p!", "s!"] {
            assert!(settled(&[name]).changes_frames(), "{name}");
        }
    }

    /// An emote's effects are read off what is stacked on it, from either
    /// side; one nobody knows is left out.
    #[test]
    fn an_emotes_effects_are_the_known_ones_stacked_on_it() {
        let lookup = |name: &str| {
            let (provider, kind) = match name {
                "KEKW" => (Provider::Bttv, Kind::Normal),
                "w!" | "h!" => (Provider::Bttv, Kind::EffectOnNext),
                "ffzY" | "ffzFuture" => (Provider::Ffz, Kind::EffectOnPrevious),
                _ => return None,
            };
            Some(Named {
                url: format!("https://example.invalid/{name}"),
                provider,
                kind,
            })
        };
        let tokens = message_tokens(
            "w! KEKW ffzY ffzFuture",
            None,
            false,
            None,
            &|_| None,
            &lookup,
        );
        let [Token::Emote(kekw)] = tokens.as_slice() else {
            panic!("not one stack: {tokens:?}");
        };
        assert_eq!(
            Effects::of(kekw),
            Effects {
                wide: true,
                turn: Some(Turn::FlipY),
                ..Effects::default()
            }
        );
    }

    #[test]
    fn a_flip_or_a_turn_moves_a_known_pixel() {
        let flipped = apply(vec![marked(3, 2)], settled(&["h!"]));
        assert_eq!(red_at(&flipped.frames[0]), (2, 0));
        let flipped = apply(vec![marked(3, 2)], settled(&["v!"]));
        assert_eq!(red_at(&flipped.frames[0]), (0, 1));
        // A quarter turn clockwise takes the top left to the top right, and
        // the picture stands on its end.
        let right = apply(vec![marked(3, 2)], settled(&["r!"]));
        assert_eq!(right.frames[0].buffer().dimensions(), (2, 3));
        assert_eq!(red_at(&right.frames[0]), (1, 0));
        let left = apply(vec![marked(3, 2)], settled(&["l!"]));
        assert_eq!(red_at(&left.frames[0]), (0, 2));
    }

    #[test]
    fn wide_and_turns_are_placed_as_the_stylesheet_says() {
        // A picture twice as wide as tall, at its own shape.
        let plain = Effects::default().placement(56, 28);
        assert_eq!(plain.width, 2.0);
        assert_eq!(plain.picture, Rect::boxed(2.0));
        // Wide: four heights across, the picture stretched to it.
        let wide = settled(&["w!"]).placement(56, 28);
        assert_eq!((wide.width, wide.picture), (4.0, Rect::boxed(4.0)));
        // Turned, it stands 28 by 56 in a square, fitted and centred.
        let turned = settled(&["r!"]).placement(28, 56);
        assert_eq!(turned.width, 1.0);
        assert_eq!(
            turned.picture,
            Rect {
                x: 0.25,
                y: 0.0,
                width: 0.5,
                height: 1.0
            }
        );
        // Wide and turned: the turn's square, the picture stretched to it.
        let both = settled(&["w!", "l!"]).placement(28, 56);
        assert_eq!((both.width, both.picture), (1.0, Rect::boxed(1.0)));
    }

    /// Cursed on a still emote is that one frame, recoloured, its delay
    /// kept.
    #[test]
    fn cursed_recolours_each_frame_and_keeps_its_time() {
        let mut grey = Frame::from_parts(
            RgbaImage::from_pixel(2, 2, Rgba([128, 128, 128, 255])),
            0,
            0,
            Delay::from_numer_denom_ms(30, 1),
        );
        grey.buffer_mut()
            .put_pixel(1, 1, Rgba([255, 255, 255, 255]));
        let cursed = apply(vec![grey], settled(&["c!"]));
        assert_eq!(cursed.frames.len(), 1);
        assert_eq!(ms(cursed.frames[0].delay()), Duration::from_millis(30));
        assert_eq!(
            cursed.frames[0].buffer().get_pixel(0, 0).0,
            [33, 33, 33, 255]
        );
        assert_eq!(cursed.frames[0].buffer().get_pixel(1, 1).0, [255; 4]);
    }

    /// Party makes a still picture 24 frames over 1.5 s, the hue a step on
    /// at each, so the 13th is half way round the wheel.
    #[test]
    fn party_on_a_still_emote_is_24_frames_over_a_second_and_a_half() {
        let orange = Frame::new(RgbaImage::from_pixel(1, 1, Rgba([200, 60, 30, 255])));
        let party = apply(vec![orange.clone()], settled(&["p!"]));
        assert_eq!(party.frames.len(), 24);
        for frame in &party.frames {
            assert_eq!(ms(frame.delay()), Duration::from_micros(62_500));
        }
        let at = |degrees: f32| {
            let mut buffer = orange.buffer().clone();
            Filter::party(degrees).apply(&mut buffer);
            buffer.get_pixel(0, 0).0
        };
        assert_eq!(party.frames[0].buffer().get_pixel(0, 0).0, at(0.0));
        assert_eq!(party.frames[12].buffer().get_pixel(0, 0).0, at(180.0));
        assert_eq!(party.placement, Effects::default().placement(1, 1));
    }

    /// Shake makes a still picture ten 50 ms frames, each on a canvas 2 px
    /// larger on every side, the picture moved by that step's offset; its
    /// place grows by the same 2 px each side, so the picture itself stays
    /// where it was.
    #[test]
    fn shake_on_a_still_emote_is_ten_steps_on_a_canvas_2px_larger() {
        let shaken = apply(vec![marked(28, 28)], settled(&["s!"]));
        assert_eq!(shaken.frames.len(), 10);
        for (frame, (dx, dy)) in shaken.frames.iter().zip(SHAKE) {
            assert_eq!(ms(frame.delay()), Duration::from_millis(50));
            assert_eq!(frame.buffer().dimensions(), (32, 32));
            assert_eq!(red_at(frame), ((2 + dx) as u32, (2 + dy) as u32));
        }
        let reach = 2.0 / 28.0;
        assert_eq!(shaken.placement.width, 1.0);
        let picture = shaken.placement.picture;
        assert!((picture.x + reach).abs() < 1e-6, "{picture:?}");
        assert!((picture.y + reach).abs() < 1e-6, "{picture:?}");
        assert!((picture.width - (1.0 + 2.0 * reach)).abs() < 1e-6);
    }

    /// The reach is BetterTTV's 2 px at the size the picture is drawn: four
    /// pixels down a 56 px picture drawn 28 tall, and across a wide box one
    /// pixel, where 56 px are stretched over 112.
    #[test]
    fn a_shake_reaches_two_px_at_the_size_drawn() {
        let shaken = apply(vec![marked(56, 56)], settled(&["s!"]));
        assert_eq!(shaken.frames[0].buffer().dimensions(), (64, 64));
        assert_eq!(red_at(&shaken.frames[0]), (4 + 4, 4));
        let wide = apply(vec![marked(56, 56)], settled(&["s!", "w!"]));
        assert_eq!(wide.frames[0].buffer().dimensions(), (58, 64));
        assert_eq!(red_at(&wide.frames[0]), (1 + 1, 4));
    }

    /// Both on a still picture: shake's ten steps repeated to party's 1.5 s,
    /// 30 frames of 50 ms, each shaken and its hue taken where it starts.
    #[test]
    fn party_and_shake_together_run_both() {
        let both = apply(vec![marked(28, 28)], settled(&["p!", "s!"]));
        assert_eq!(both.frames.len(), 30);
        assert!(both
            .frames
            .iter()
            .all(|frame| ms(frame.delay()) == Duration::from_millis(50)));
        assert_eq!(both.frames[10].buffer().dimensions(), (32, 32));
        let mut red = RgbaImage::from_pixel(1, 1, RED);
        Filter::party(120.0).apply(&mut red);
        let (dx, dy) = SHAKE[0];
        let tenth = both.frames[10].buffer();
        assert_eq!(
            *tenth.get_pixel((2 + dx) as u32, (2 + dy) as u32),
            *red.get_pixel(0, 0)
        );
    }

    /// Turned and shaken, a picture twice as wide as tall stands in the
    /// middle of its square, the canvas grown by the reach on every side,
    /// which on a 56 px picture fitted to 28 is 4 pixels.
    #[test]
    fn a_turned_shake_stays_in_the_middle_of_its_square() {
        let shaken = apply(vec![marked(56, 28)], settled(&["r!", "s!"]));
        assert_eq!(shaken.frames.len(), 10);
        let first = &shaken.frames[0];
        assert_eq!(first.buffer().dimensions(), (28 + 8, 56 + 8));
        // Turned, the top left is at the top right, (27, 0); the first step
        // moves it 2 px right, 4 of these pixels.
        assert_eq!(red_at(first), (27 + 4 + 4, 4));
        let reach = 2.0 / 28.0;
        let placement = shaken.placement;
        assert_eq!(placement.width, 1.0);
        let picture = placement.picture;
        let close = |a: f32, b: f32| (a - b).abs() < 1e-6;
        assert!(close(picture.x, 0.25 - reach), "{picture:?}");
        assert!(close(picture.y, -reach), "{picture:?}");
        assert!(close(picture.width, 0.5 + 2.0 * reach), "{picture:?}");
        assert!(close(picture.height, 1.0 + 2.0 * reach), "{picture:?}");
        assert!(close(picture.x + picture.width / 2.0, 0.5));
        assert!(close(picture.y + picture.height / 2.0, 0.5));
    }

    /// Frames `ms` long, each a picture of its own: clear but for red at
    /// `(n, 0)` in the `n`th.
    fn animated(ms: &[u32]) -> Vec<Frame> {
        ms.iter()
            .enumerate()
            .map(|(n, &ms)| {
                let mut buffer = RgbaImage::from_pixel(28, 28, CLEAR);
                buffer.put_pixel(n as u32, 0, RED);
                Frame::from_parts(buffer, 0, 0, Delay::from_numer_denom_ms(ms, 1))
            })
            .collect()
    }

    /// How long `frames` run, all told.
    fn total(frames: &[Frame]) -> Duration {
        frames.iter().map(|frame| ms(frame.delay())).sum()
    }

    /// An animated emote keeps its own frames and delays where none holds
    /// past a step; each takes shake's offset from when it starts. Its
    /// 120 ms loop is run until it ends with shake's 500 ms, 25 times, so
    /// every offset is shown and the loop comes round on the first.
    #[test]
    fn an_animated_emote_takes_the_shake_by_its_time() {
        let shaken = apply(animated(&[30, 30, 30, 30]), settled(&["s!"]));
        assert_eq!(shaken.frames.len(), 100);
        assert_eq!(total(&shaken.frames), Duration::from_millis(3_000));
        // They start at 0, 30, 60 and 90 ms: steps 0, 0, 1 and 1; and so on,
        // each the emote's frame in turn, the clock running on.
        for (k, frame) in shaken.frames.iter().enumerate() {
            assert_eq!(ms(frame.delay()), Duration::from_millis(30));
            let (dx, dy) = SHAKE[k * 30 / 50 % 10];
            let n = (k % 4) as i32;
            assert_eq!(red_at(frame), ((2 + n + dx) as u32, (2 + dy) as u32), "{k}");
        }
        let steps: std::collections::HashSet<_> =
            (0..100).map(|k: usize| k * 30 / 50 % 10).collect();
        assert_eq!(steps.len(), SHAKE.len());
    }

    /// Party on a loop shorter than its 1.5 s goes all the way round the
    /// wheel: the loop run until the two end together, 3 s, the hue taken
    /// where each frame starts, so the 26th frame, 750 ms in, is half way.
    #[test]
    fn party_takes_a_short_loop_all_the_way_round() {
        let colours = [
            [200, 60, 30, 255],
            [40, 180, 90, 255],
            [30, 70, 210, 255],
            [180, 160, 40, 255],
        ];
        let frames = colours
            .map(|colour| {
                Frame::from_parts(
                    RgbaImage::from_pixel(1, 1, Rgba(colour)),
                    0,
                    0,
                    Delay::from_numer_denom_ms(30, 1),
                )
            })
            .to_vec();
        let party = apply(frames, settled(&["p!"]));
        assert_eq!(party.frames.len(), 100);
        assert_eq!(total(&party.frames), Duration::from_millis(3_000));
        let mut half = RgbaImage::from_pixel(1, 1, Rgba(colours[25 % 4]));
        Filter::party(180.0).apply(&mut half);
        assert_eq!(
            party.frames[25].buffer().get_pixel(0, 0),
            half.get_pixel(0, 0)
        );
    }

    /// Frames `ms` long, as `timeline` reads them.
    fn delays(ms: &[u32]) -> Vec<Frame> {
        ms.iter()
            .map(|&ms| {
                Frame::from_parts(
                    RgbaImage::new(1, 1),
                    0,
                    0,
                    Delay::from_numer_denom_ms(ms, 1),
                )
            })
            .collect()
    }

    /// Where `timeline` ends the made loop: how many frames, and how long.
    fn made_loop(frames: &[Frame], names: &[&str], budget: usize) -> (usize, u64) {
        let made = timeline(frames, settled(names), budget);
        let length = made
            .iter()
            .map(|(_, _, delay)| ms(*delay).as_micros() as u64);
        (made.len(), length.sum())
    }

    /// A loop that meets the period only far out is run as many times as
    /// the budget allows and ends nearest a period's end: 30 frames of
    /// 33 ms, 990 ms, meets 1.5 s at 49.5 s, so at 256 frames it is run three
    /// times, 2.97 s, the wheel 30 ms short as it comes round, where twice
    /// would be 480 ms short. A budget under two runs keeps the one, and
    /// under one still keeps the emote's own loop whole.
    #[test]
    fn past_the_budget_the_loop_ends_nearest_the_period() {
        let frames = delays(&[33; 30]);
        assert_eq!(made_loop(&frames, &["p!"], 256), (90, 2_970_000));
        assert_eq!(made_loop(&frames, &["p!"], 40), (30, 990_000));
        assert_eq!(made_loop(&frames, &["p!"], 10), (30, 990_000));
        // Within the budget it runs to where the two end together.
        assert_eq!(made_loop(&frames, &["p!"], 2_000), (1_500, 49_500_000));
    }

    /// A loop shorter than the period is as far off as all it leaves out.
    /// 35 frames of 40 ms, 1.4 s, meets 1.5 s only at 21 s, past 334 frames
    /// (a 56 px emote's with party), so it is kept as it is, 100 ms short,
    /// rather than run twice, 200 ms short of 3 s. And where the budget is
    /// spent before a whole period, the most of the wheel it can show: a
    /// 120 ms loop in 20 frames is run five times, 600 ms, not once, which is
    /// only 120 ms from the period's start but never shows the rest.
    #[test]
    fn a_short_loop_is_off_by_what_it_leaves_out() {
        assert_eq!(made_loop(&delays(&[40; 35]), &["p!"], 334), (35, 1_400_000));
        assert_eq!(made_loop(&delays(&[30; 4]), &["p!"], 20), (20, 600_000));
    }

    /// A loop longer than the period that is not a multiple of it is run
    /// again too: 2 s with party meets 1.5 s at 6 s, each 1 s frame cut at
    /// party's 62.5 ms steps. One that is a multiple, or a still, is made
    /// once.
    #[test]
    fn a_loop_ends_with_the_period() {
        assert_eq!(
            made_loop(&delays(&[1_000, 1_000]), &["p!"], 1_000),
            (96, 6_000_000)
        );
        assert_eq!(
            made_loop(&delays(&[250, 250]), &["s!"], 1_000),
            (10, 500_000)
        );
        assert_eq!(
            made_loop(&delays(&[40]), &["p!", "s!"], 1_000),
            (30, 1_500_000)
        );
        assert_eq!(made_loop(&delays(&[0, 0]), &["p!"], 1_000), (2, 0));
    }

    /// A frame that holds past shake's next step is cut there, each piece
    /// the same picture with the offset its time has, so an emote whose
    /// frames are 500 ms still shakes, at its own pace.
    #[test]
    fn a_frame_held_past_a_step_is_cut_at_each() {
        let shaken = apply(animated(&[500, 500]), settled(&["s!"]));
        assert_eq!(shaken.frames.len(), 20);
        for (k, frame) in shaken.frames.iter().enumerate() {
            assert_eq!(ms(frame.delay()), Duration::from_millis(50));
            let (dx, dy) = SHAKE[k % 10];
            let n = (k / 10) as i32;
            assert_eq!(red_at(frame), ((2 + n + dx) as u32, (2 + dy) as u32), "{k}");
        }
        // Off the steps: 30 ms, then 100 ms from 30 cut at 50 and 100; the
        // first run of the loop, which goes on to meet 500 ms.
        let shaken = apply(animated(&[30, 100]), settled(&["s!"]));
        let pieces: Vec<_> = shaken
            .frames
            .iter()
            .take(4)
            .map(|frame| (ms(frame.delay()).as_millis(), red_at(frame)))
            .collect();
        let at = |n: i32, step: usize| {
            let (dx, dy) = SHAKE[step];
            ((2 + n + dx) as u32, (2 + dy) as u32)
        };
        assert_eq!(
            pieces,
            [
                (30, at(0, 0)),
                (20, at(1, 0)),
                (50, at(1, 1)),
                (30, at(1, 2))
            ]
        );
    }

    /// Party on an animated emote: each frame takes its hue from when it
    /// starts, cut at party's 62.5 ms steps, so three frames of 375 ms are
    /// six pieces each, and each frame's first piece is 0, 90 and 180
    /// degrees round. Their 1.125 s loop is run four times, to 4.5 s, where
    /// it ends with the wheel.
    #[test]
    fn an_animated_emote_takes_party_hue_by_its_time() {
        let colours = [[200, 60, 30, 255], [40, 180, 90, 255], [30, 70, 210, 255]];
        let frames = colours
            .map(|colour| {
                Frame::from_parts(
                    RgbaImage::from_pixel(1, 1, Rgba(colour)),
                    0,
                    0,
                    Delay::from_numer_denom_ms(375, 1),
                )
            })
            .to_vec();
        let party = apply(frames, settled(&["p!"]));
        assert_eq!(party.frames.len(), 72);
        assert_eq!(total(&party.frames), Duration::from_millis(4_500));
        for frame in &party.frames {
            assert_eq!(ms(frame.delay()), Duration::from_micros(62_500));
        }
        let at = |colour: [u8; 4], degrees: f32| {
            let mut buffer = RgbaImage::from_pixel(1, 1, Rgba(colour));
            Filter::party(degrees).apply(&mut buffer);
            buffer.get_pixel(0, 0).0
        };
        for (n, (colour, degrees)) in colours.into_iter().zip([0.0, 90.0, 180.0]).enumerate() {
            let first = party.frames[n * 6].buffer().get_pixel(0, 0).0;
            assert_eq!(first, at(colour, degrees), "frame {n}");
        }
        // A piece on from the first is a step round, the picture the same.
        let second = party.frames[7].buffer().get_pixel(0, 0).0;
        assert_eq!(second, at(colours[1], 105.0));
    }

    /// Party's hue is where its 1.5 s turn is, coming round past it.
    #[test]
    fn party_and_shake_are_timed_by_their_periods() {
        assert_eq!(party_degrees(0), 0.0);
        assert_eq!(party_degrees(750_000), 180.0);
        assert_eq!(party_degrees(1_875_000), 90.0);
        assert_eq!(shake_at(0), SHAKE[0]);
        assert_eq!(shake_at(49_999), SHAKE[0]);
        assert_eq!(shake_at(450_000), SHAKE[9]);
        assert_eq!(shake_at(500_000), SHAKE[0]);
    }
}
