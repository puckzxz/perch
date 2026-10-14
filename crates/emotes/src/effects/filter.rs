//! The CSS filter functions two of BetterTTV's effects are made of, done
//! to a frame's pixels.
//!
//! Each is defined by the Filter Effects spec in terms of an SVG primitive
//! ("Shorthands defined in terms of the filter element",
//! <https://drafts.fxtf.org/filter-effects/#ShorthandEquivalents>; its source
//! read on 14 October 2026): `grayscale`, `sepia`, `saturate` and
//! `hue-rotate` are colour matrices, `brightness` multiplies, and `contrast`
//! is `(c - 0.5) * k + 0.5`. The spec has them work in sRGB, on colour that is
//! not premultiplied, so a pixel's 8-bit values are used as they are and its
//! alpha is left alone; and it clamps every primitive's result before the
//! next one sees it, which is why a list is applied one function at a time
//! rather than folded into one matrix. `saturate` and `hue-rotate` use the
//! coefficients the spec prints (0.213, 0.715, 0.072), which are three-digit
//! where `grayscale`'s are four.

use image::RgbaImage;

/// A colour as three channels from 0 to 1.
type Rgb = [f32; 3];

/// One filter function, as the primitive the spec defines it by.
#[derive(Debug, Clone, Copy)]
enum Step {
    /// A colour matrix, row by row (`feColorMatrix`).
    Matrix([[f32; 3]; 3]),
    /// The same straight line on every channel (`feComponentTransfer`,
    /// `type="linear"`).
    Linear { slope: f32, intercept: f32 },
}

impl Step {
    fn apply(self, [r, g, b]: Rgb) -> Rgb {
        let out = match self {
            Step::Matrix(m) => [
                m[0][0] * r + m[0][1] * g + m[0][2] * b,
                m[1][0] * r + m[1][1] * g + m[1][2] * b,
                m[2][0] * r + m[2][1] * g + m[2][2] * b,
            ],
            Step::Linear { slope, intercept } => {
                [r, g, b].map(|channel| slope * channel + intercept)
            }
        };
        out.map(|channel| channel.clamp(0.0, 1.0))
    }
}

/// `grayscale(amount)`.
fn grayscale(amount: f32) -> Step {
    let a = 1.0 - amount.min(1.0);
    Step::Matrix([
        [
            0.2126 + 0.7874 * a,
            0.7152 - 0.7152 * a,
            0.0722 - 0.0722 * a,
        ],
        [
            0.2126 - 0.2126 * a,
            0.7152 + 0.2848 * a,
            0.0722 - 0.0722 * a,
        ],
        [
            0.2126 - 0.2126 * a,
            0.7152 - 0.7152 * a,
            0.0722 + 0.9278 * a,
        ],
    ])
}

/// `sepia(amount)`.
fn sepia(amount: f32) -> Step {
    let a = 1.0 - amount.min(1.0);
    Step::Matrix([
        [0.393 + 0.607 * a, 0.769 - 0.769 * a, 0.189 - 0.189 * a],
        [0.349 - 0.349 * a, 0.686 + 0.314 * a, 0.168 - 0.168 * a],
        [0.272 - 0.272 * a, 0.534 - 0.534 * a, 0.131 + 0.869 * a],
    ])
}

/// `saturate(amount)`: `feColorMatrix type="saturate"`.
fn saturate(s: f32) -> Step {
    Step::Matrix([
        [0.213 + 0.787 * s, 0.715 - 0.715 * s, 0.072 - 0.072 * s],
        [0.213 - 0.213 * s, 0.715 + 0.285 * s, 0.072 - 0.072 * s],
        [0.213 - 0.213 * s, 0.715 - 0.715 * s, 0.072 + 0.928 * s],
    ])
}

/// `hue-rotate(degrees)`: `feColorMatrix type="hueRotate"`.
fn hue_rotate(degrees: f32) -> Step {
    let (sin, cos) = degrees.to_radians().sin_cos();
    Step::Matrix([
        [
            0.213 + cos * 0.787 - sin * 0.213,
            0.715 - cos * 0.715 - sin * 0.715,
            0.072 - cos * 0.072 + sin * 0.928,
        ],
        [
            0.213 - cos * 0.213 + sin * 0.143,
            0.715 + cos * 0.285 + sin * 0.140,
            0.072 - cos * 0.072 - sin * 0.283,
        ],
        [
            0.213 - cos * 0.213 - sin * 0.787,
            0.715 - cos * 0.715 + sin * 0.715,
            0.072 + cos * 0.928 + sin * 0.072,
        ],
    ])
}

/// `brightness(amount)`.
fn brightness(amount: f32) -> Step {
    Step::Linear {
        slope: amount,
        intercept: 0.0,
    }
}

/// `contrast(amount)`.
fn contrast(amount: f32) -> Step {
    Step::Linear {
        slope: amount,
        intercept: -(0.5 * amount) + 0.5,
    }
}

/// A `filter` list, applied in the order written.
#[derive(Debug, Clone, Copy)]
pub(super) struct Filter([Step; 3]);

impl Filter {
    /// `c!` and `ffzCursed`: `grayscale(1) brightness(0.7) contrast(2.5)`.
    pub(super) fn cursed() -> Self {
        Self([grayscale(1.0), brightness(0.7), contrast(2.5)])
    }

    /// `p!` at `degrees` into its turn of the colour wheel:
    /// `sepia(0.5) hue-rotate(degrees) saturate(2.5)`.
    pub(super) fn party(degrees: f32) -> Self {
        Self([sepia(0.5), hue_rotate(degrees), saturate(2.5)])
    }

    /// Filter every pixel of `image` in place, its alpha kept.
    pub(super) fn apply(&self, image: &mut RgbaImage) {
        for pixel in image.pixels_mut() {
            let mut rgb = [pixel[0], pixel[1], pixel[2]].map(|channel| f32::from(channel) / 255.0);
            for step in self.0 {
                rgb = step.apply(rgb);
            }
            for (channel, value) in pixel.0.iter_mut().zip(rgb) {
                // Clamped to 0..=1 by the last step, so this is in range.
                *channel = (value * 255.0).round() as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    /// `filter` applied to one pixel of `rgb`, fully opaque.
    fn filtered(filter: Filter, rgb: [u8; 3]) -> [u8; 4] {
        let mut image = RgbaImage::from_pixel(1, 1, Rgba([rgb[0], rgb[1], rgb[2], 255]));
        filter.apply(&mut image);
        image.get_pixel(0, 0).0
    }

    /// Cursed, worked by hand from the spec: grey by luminance, times 0.7,
    /// then `(c - 0.5) * 2.5 + 0.5`, clamped. White stays white (0.7 is
    /// 0.2 under the middle, 0.5 over it after the contrast); mid grey goes
    /// to 0.1284, 33 of 255; red's luminance (0.2126) ends below zero.
    #[test]
    fn cursed_maps_known_colours_to_the_specs_values() {
        let cursed = Filter::cursed();
        assert_eq!(filtered(cursed, [255, 255, 255]), [255, 255, 255, 255]);
        assert_eq!(filtered(cursed, [128, 128, 128]), [33, 33, 33, 255]);
        assert_eq!(filtered(cursed, [255, 0, 0]), [0, 0, 0, 255]);
        assert_eq!(filtered(cursed, [0, 0, 0]), [0, 0, 0, 255]);
        // Green's luminance, 0.7152: 0.5006 after brightness, 0.5016 after
        // contrast, 128 of 255.
        assert_eq!(filtered(cursed, [0, 255, 0]), [128, 128, 128, 255]);
    }

    /// Party at no turn is sepia half way then saturated: worked by hand,
    /// mid grey (0.502) is sepia'd to (0.5901, 0.5529, 0.4861), and
    /// `saturate(2.5)` pushes that apart to (0.6411, 0.5483, 0.3813).
    #[test]
    fn party_at_no_turn_is_sepia_then_saturate() {
        let [r, g, b, a] = filtered(Filter::party(0.0), [128, 128, 128]);
        let expected = [163, 140, 97];
        for (got, want) in [r, g, b].into_iter().zip(expected) {
            assert!(
                got.abs_diff(want) <= 1,
                "{:?} against {expected:?}",
                [r, g, b]
            );
        }
        assert_eq!(a, 255);
    }

    /// A whole turn of the colour wheel is no turn at all, and half a turn
    /// is not.
    #[test]
    fn party_comes_round_in_a_full_turn() {
        let colour = [200, 60, 30];
        let start = filtered(Filter::party(0.0), colour);
        let round = filtered(Filter::party(360.0), colour);
        for (a, b) in start.into_iter().zip(round) {
            assert!(a.abs_diff(b) <= 1, "{start:?} against {round:?}");
        }
        assert_ne!(filtered(Filter::party(180.0), colour), start);
    }

    /// Alpha is not a colour: a filter leaves it as it was.
    #[test]
    fn a_filter_keeps_alpha() {
        let mut image = RgbaImage::from_pixel(1, 1, Rgba([10, 200, 90, 77]));
        Filter::cursed().apply(&mut image);
        assert_eq!(image.get_pixel(0, 0)[3], 77);
    }
}
