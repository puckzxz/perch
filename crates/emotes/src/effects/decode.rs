//! An emote's file to frames, as gpui decodes one for `img`
//! (`ImageAssetLoader` in `vendor/gpui/src/elements/img.rs`): a GIF, and a
//! WebP that is animated, to every frame, each composited onto the whole
//! picture and carrying its delay; anything else to one still frame. So an
//! emote with an effect animates exactly as the same emote without one.
//!
//! RGBA, as the image crate gives it. gpui keeps BGRA, and turning one into
//! the other is the drawer's half, done once on the frames an effect made.

use std::io::Cursor;

use image::codecs::gif::GifDecoder;
use image::codecs::webp::WebPDecoder;
use image::{AnimationDecoder, DynamicImage, Frame, ImageFormat, Rgba};

/// Decode `bytes`, an emote's file, to its frames.
pub fn decode(bytes: &[u8]) -> Result<Vec<Frame>, String> {
    let format = image::guess_format(bytes).map_err(|e| e.to_string())?;
    let frames = match format {
        ImageFormat::Gif => GifDecoder::new(Cursor::new(bytes))
            .and_then(|decoder| decoder.into_frames().collect_frames())
            .map_err(|e| e.to_string())?,
        ImageFormat::WebP => {
            let mut decoder = WebPDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
            if decoder.has_animation() {
                // What gpui sets: a frame's uncovered pixels are clear, not
                // the file's background colour.
                let _ = decoder.set_background_color(Rgba([0, 0, 0, 0]));
                decoder
                    .into_frames()
                    .collect_frames()
                    .map_err(|e| e.to_string())?
            } else {
                let still = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
                vec![Frame::new(still.into_rgba8())]
            }
        }
        _ => {
            let still =
                image::load_from_memory_with_format(bytes, format).map_err(|e| e.to_string())?;
            vec![Frame::new(still.into_rgba8())]
        }
    };
    if frames.is_empty() {
        return Err("no frames".to_string());
    }
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Delay, ImageEncoder, RgbaImage};

    use super::*;

    #[test]
    fn a_gif_decodes_to_every_frame_with_its_delay() {
        let mut bytes = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut bytes);
            encoder.set_repeat(Repeat::Infinite).unwrap();
            for (colour, ms) in [([255, 0, 0, 255], 40), ([0, 0, 255, 255], 70)] {
                let frame = Frame::from_parts(
                    RgbaImage::from_pixel(3, 2, Rgba(colour)),
                    0,
                    0,
                    Delay::from_numer_denom_ms(ms, 1),
                );
                encoder.encode_frame(frame).unwrap();
            }
        }
        let frames = decode(&bytes).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].buffer().dimensions(), (3, 2));
        assert_eq!(frames[1].buffer().get_pixel(0, 0).0, [0, 0, 255, 255]);
        assert_eq!(Duration::from(frames[0].delay()), Duration::from_millis(40));
        assert_eq!(Duration::from(frames[1].delay()), Duration::from_millis(70));
    }

    #[test]
    fn a_png_is_one_still_frame() {
        let picture = RgbaImage::from_pixel(4, 4, Rgba([10, 20, 30, 128]));
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&picture, 4, 4, image::ExtendedColorType::Rgba8)
            .unwrap();
        let frames = decode(&bytes).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].buffer().get_pixel(3, 3).0, [10, 20, 30, 128]);
    }

    #[test]
    fn what_is_not_a_picture_is_an_error() {
        assert!(decode(b"<html>not found</html>").is_err());
        assert!(decode(&[]).is_err());
    }
}
