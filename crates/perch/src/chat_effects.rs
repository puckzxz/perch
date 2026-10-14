//! The pictures FrankerFaceZ's and BetterTTV's effects make of an emote, as
//! a chat pane keeps them.
//!
//! What an effect does is `emotes::effects`', pure and tested; this is what
//! is kept of it. A flipped, turned, recoloured or shaken emote is a picture
//! of its own: its file decoded again, its frames changed, and handed to gpui
//! as one `RenderImage`, which `img` animates as it does any image ([`make`],
//! run off the UI thread). Each is asked for by its emote's URL and the
//! effects on it, so the same stack in a hundred rows is made once.
//!
//! A `RenderImage` that has been drawn holds atlas tiles, one per frame,
//! which gpui gives back only when told (`App::drop_image`), exactly as the
//! pane's plain emotes do (`ChatView::emote_images`). So the book is bounded
//! ([`LIMIT`]): past it, the picture drawn longest ago goes, and is handed
//! back to be released at the start of the pane's next frame
//! ([`Pictures::start_frame`]); and when the pane goes, everything is
//! ([`Pictures::take_all`]). Never one the frame being drawn has used: its
//! tiles would be freed under a scene that still names them, so a frame that
//! asks for more than the bound overruns it, until an ask in a later frame
//! lets the extra go. The frame that does is usually the list's measuring
//! pass, which lays out every row of the backlog at once (when the pane first
//! measures, and when a new text size, times setting or Shared Chat label
//! resets the list while it follows live): every distinct emote and effects
//! in the backlog is made then, which costs memory and background work but
//! no atlas tiles, since nothing measured is painted.

use std::sync::Arc;

use emotes::effects::{self, Placement};
use emotes::Effects;
use gpui::RenderImage;
use image::Frame;
use smallvec::SmallVec;

/// How many pictures a pane keeps made. A pane shows a few dozen rows, and
/// an effect is on a few of them; past this, a picture made has not been
/// drawn for a while.
pub const LIMIT: usize = 64;

/// What there is of one picture.
#[derive(Debug, Clone, PartialEq)]
pub enum Picture<T> {
    /// Asked for, and being made off the UI thread.
    Making,
    Ready(T),
    /// Could not be made (a file the decoder does not read): the emote is
    /// drawn plain.
    Failed,
}

#[derive(Debug)]
struct Entry<T> {
    url: String,
    effects: Effects,
    picture: Picture<T>,
    /// The frame that last asked for it.
    drawn: u64,
}

/// One pane's pictures, by emote URL and effects. Generic over what is
/// made, which is [`Made`] in the app, so the bookkeeping is tested alone.
#[derive(Debug)]
pub struct Pictures<T> {
    entries: Vec<Entry<T>>,
    /// The frame being drawn, counted from the pane's first.
    frame: u64,
    /// Let go while a frame was drawn, to be released before the next.
    gone: Vec<T>,
}

impl<T> Default for Pictures<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            frame: 0,
            gone: Vec::new(),
        }
    }
}

impl<T: Clone> Pictures<T> {
    /// A new frame is being drawn: what the last one let go, to be released
    /// now that no scene being built can name it.
    pub fn start_frame(&mut self) -> Vec<T> {
        self.frame += 1;
        std::mem::take(&mut self.gone)
    }

    /// What there is of `url` with `effects`, marked as drawn in this frame;
    /// `None` when it has not been asked for, or was let go.
    pub fn get(&mut self, url: &str, effects: Effects) -> Option<Picture<T>> {
        let frame = self.frame;
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.url == url && entry.effects == effects)?;
        entry.drawn = frame;
        Some(entry.picture.clone())
    }

    /// Note `url` with `effects` as being made, letting the picture drawn
    /// longest ago go when the book is full.
    pub fn start(&mut self, url: &str, effects: Effects) {
        while self.entries.len() >= LIMIT {
            let oldest = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry.drawn < self.frame)
                .min_by_key(|(_, entry)| entry.drawn)
                .map(|(index, _)| index);
            let Some(index) = oldest else {
                break;
            };
            if let Picture::Ready(made) = self.entries.swap_remove(index).picture {
                self.gone.push(made);
            }
        }
        self.entries.push(Entry {
            url: url.to_string(),
            effects,
            picture: Picture::Making,
            drawn: self.frame,
        });
    }

    /// `url` with `effects` is made, or could not be. Kept only where it is
    /// still being made: one let go meanwhile, or made twice, is dropped,
    /// which costs nothing, since a picture never drawn has no tiles.
    pub fn land(&mut self, url: &str, effects: Effects, made: Option<T>) {
        let making = self.entries.iter_mut().find(|entry| {
            entry.url == url && entry.effects == effects && matches!(entry.picture, Picture::Making)
        });
        if let Some(entry) = making {
            entry.picture = made.map_or(Picture::Failed, Picture::Ready);
        }
    }

    /// Everything held, for the pane's going: all of it is released.
    pub fn take_all(&mut self) -> Vec<T> {
        let mut all = std::mem::take(&mut self.gone);
        all.extend(
            self.entries
                .drain(..)
                .filter_map(|entry| match entry.picture {
                    Picture::Ready(made) => Some(made),
                    Picture::Making | Picture::Failed => None,
                }),
        );
        all
    }
}

/// A picture an emote's effects made: its frames as gpui draws them, and
/// where it goes in the line.
#[derive(Debug, Clone)]
pub struct Made {
    pub image: Arc<RenderImage>,
    pub placement: Placement,
}

/// Make what `effects` make of an emote whose file is `bytes`. The slow
/// half — a decode, then a pass over every pixel of every frame — so it is
/// never run on the UI thread.
pub fn make(bytes: &[u8], effects: Effects) -> Result<Made, String> {
    let changed = effects::apply(effects::decode(bytes)?, effects);
    let frames: SmallVec<[Frame; 1]> = changed
        .frames
        .into_iter()
        .map(|mut frame| {
            // gpui keeps BGRA: `img.rs` swaps every frame it decodes.
            for pixel in frame.buffer_mut().chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
            frame
        })
        .collect();
    Ok(Made {
        image: Arc::new(RenderImage::new(frames)),
        placement: changed.placement,
    })
}

#[cfg(test)]
mod tests {
    use emotes::Effect;
    use image::{ImageEncoder, Rgba, RgbaImage};

    use super::*;

    fn effects(names: &[&str]) -> Effects {
        names
            .iter()
            .map(|name| Effect::named(name).expect("a known effect"))
            .collect()
    }

    /// The book's state, picture by picture, for the URL each is kept under.
    fn states(book: &Pictures<u32>) -> Vec<(&str, &Picture<u32>)> {
        book.entries
            .iter()
            .map(|entry| (entry.url.as_str(), &entry.picture))
            .collect()
    }

    #[test]
    fn a_picture_is_asked_for_once_and_kept() {
        let mut book = Pictures::<u32>::default();
        book.start_frame();
        let flip = effects(&["h!"]);
        assert_eq!(book.get("kekw", flip), None);
        book.start("kekw", flip);
        assert_eq!(book.get("kekw", flip), Some(Picture::Making));
        // The same emote with other effects is another picture.
        assert_eq!(book.get("kekw", effects(&["v!"])), None);
        book.land("kekw", flip, Some(7));
        assert_eq!(book.get("kekw", flip), Some(Picture::Ready(7)));
        book.land("kekw", effects(&["c!"]), None);
        assert_eq!(states(&book), [("kekw", &Picture::Ready(7))]);
    }

    /// One that could not be made is kept as such, so it is not made again
    /// every frame; one let go before it landed, or made twice, is dropped.
    #[test]
    fn a_picture_lands_only_where_it_is_being_made() {
        let mut book = Pictures::<u32>::default();
        book.start_frame();
        let party = effects(&["p!"]);
        book.start("bad", party);
        book.land("bad", party, None);
        assert_eq!(book.get("bad", party), Some(Picture::Failed));
        book.land("bad", party, Some(1));
        assert_eq!(book.get("bad", party), Some(Picture::Failed));
        book.land("never asked", party, Some(2));
        assert_eq!(book.get("never asked", party), None);
    }

    /// Full, the book lets go of the picture drawn longest ago, and hands it
    /// back to be released at the start of the next frame.
    #[test]
    fn past_the_limit_the_one_drawn_longest_ago_goes() {
        let mut book = Pictures::<u32>::default();
        let flip = effects(&["h!"]);
        for n in 0..LIMIT {
            book.start_frame();
            let url = n.to_string();
            book.start(&url, flip);
            book.land(&url, flip, Some(n as u32));
        }
        // The first is drawn again, so the second is now the oldest.
        book.start_frame();
        assert_eq!(book.get("0", flip), Some(Picture::Ready(0)));
        book.start_frame();
        book.start("new", flip);
        assert_eq!(book.entries.len(), LIMIT);
        assert_eq!(book.get("1", flip), None);
        assert_eq!(book.get("0", flip), Some(Picture::Ready(0)));
        assert_eq!(book.start_frame(), [1]);
        assert_eq!(book.start_frame(), Vec::<u32>::new());
    }

    /// Nothing the frame being drawn has used goes, even past the limit: its
    /// tiles are in the scene being built.
    #[test]
    fn nothing_this_frame_drew_goes() {
        let mut book = Pictures::<u32>::default();
        let flip = effects(&["h!"]);
        book.start_frame();
        for n in 0..=LIMIT {
            let url = n.to_string();
            book.start(&url, flip);
            book.land(&url, flip, Some(n as u32));
        }
        assert_eq!(book.entries.len(), LIMIT + 1);
        assert!(book.start_frame().is_empty());
        // The next frame asks for one more: the first frame's go, oldest
        // first, down to the limit.
        book.start("one more", flip);
        assert_eq!(book.entries.len(), LIMIT);
        assert_eq!(book.start_frame().len(), 2);
    }

    #[test]
    fn the_pane_going_gives_everything_back() {
        let mut book = Pictures::<u32>::default();
        let flip = effects(&["h!"]);
        book.start_frame();
        for n in 0..LIMIT {
            let url = n.to_string();
            book.start(&url, flip);
            book.land(&url, flip, Some(n as u32));
        }
        book.start_frame();
        book.start("making", flip);
        let mut all = book.take_all();
        all.sort_unstable();
        assert_eq!(all, (0..LIMIT as u32).collect::<Vec<_>>());
        assert!(book.entries.is_empty());
    }

    /// A made picture is in gpui's colour order, with the frames the effects
    /// made and the place they go.
    #[test]
    fn a_made_picture_is_bgra_and_placed() {
        let mut red_left = RgbaImage::from_pixel(2, 1, Rgba([0, 0, 0, 0]));
        red_left.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&red_left, 2, 1, image::ExtendedColorType::Rgba8)
            .unwrap();

        let flipped = make(&bytes, effects(&["h!"])).unwrap();
        assert_eq!(flipped.image.frame_count(), 1);
        // Mirrored, red is on the right, written blue first.
        assert_eq!(
            flipped.image.as_bytes(0).unwrap(),
            [0, 0, 0, 0, 0, 0, 255, 255]
        );
        assert_eq!(flipped.placement.width, 2.0);

        let party = make(&bytes, effects(&["p!"])).unwrap();
        assert_eq!(party.image.frame_count(), 24);
        assert!(make(b"not a picture", effects(&["h!"])).is_err());
    }
}
