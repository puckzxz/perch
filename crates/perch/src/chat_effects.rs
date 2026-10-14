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
//! pane's plain emotes do (`ChatView::emote_images`), and its frames, which
//! it keeps as well. So the book is bounded, by count ([`LIMIT`]) and by
//! what its frames cost ([`BYTES`]): past either, the picture drawn longest
//! ago goes, and is handed back to be released at the start of the pane's
//! next frame ([`Pictures::start_frame`]); and when the pane goes, everything
//! is ([`Pictures::take_all`]). Never one the frame being drawn has used: its
//! tiles would be freed under a scene that still names them, so a frame that
//! asks for more than the bound overruns it, until an ask, or a picture
//! landing, in a later frame lets the extra go. The frame that does is
//! usually the list's measuring pass, which lays out every row of the
//! backlog at once (when the pane first measures, and when a new text size,
//! times setting or Shared Chat label resets the list while it follows
//! live): every distinct emote and effects in the backlog is made then,
//! which costs memory and background work but no atlas tiles, since nothing
//! measured is painted.

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

/// How many bytes of frames a pane keeps made, all told. Party and shake run
/// an animated emote's loop again until it ends with their period, up to
/// 4 MiB of frames for one picture (`emotes::effects`), so [`LIMIT`] alone
/// would let a pane keep 256 MiB of them. This holds eight of the largest,
/// more than a pane shows at once, and a flip, turn or cursed is only the
/// emote's own frames, a few hundred KiB for most.
pub const BYTES: usize = 32 << 20;

/// What a kept picture costs, which the book holds to [`BYTES`].
pub trait Weight {
    /// The bytes of its frames.
    fn bytes(&self) -> usize;
}

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
    /// What its frames cost: nothing until it is made, or if it could not be.
    bytes: usize,
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

impl<T: Clone + Weight> Pictures<T> {
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
        self.let_go(1);
        self.entries.push(Entry {
            url: url.to_string(),
            effects,
            picture: Picture::Making,
            bytes: 0,
            drawn: self.frame,
        });
    }

    /// `url` with `effects` is made, or could not be. Kept only where it is
    /// still being made: one let go meanwhile, or made twice, is dropped,
    /// which costs nothing, since a picture never drawn has no tiles. Its
    /// frames can take the book past [`BYTES`], so it lets go of what it must.
    pub fn land(&mut self, url: &str, effects: Effects, made: Option<T>) {
        let making = self.entries.iter_mut().find(|entry| {
            entry.url == url && entry.effects == effects && matches!(entry.picture, Picture::Making)
        });
        if let Some(entry) = making {
            entry.bytes = made.as_ref().map_or(0, Weight::bytes);
            entry.picture = made.map_or(Picture::Failed, Picture::Ready);
        }
        self.let_go(0);
    }

    /// Let the pictures drawn longest ago go, never one the frame being drawn
    /// used, until there is room for `room` more under [`LIMIT`] and the
    /// frames kept are within [`BYTES`]. Past the bytes alone, only a picture
    /// that holds frames goes: one being made, or that could not be, costs
    /// nothing, and letting it go would only have it asked for again.
    fn let_go(&mut self, room: usize) {
        loop {
            let crowded = self.entries.len() + room > LIMIT;
            let heavy = self.entries.iter().map(|entry| entry.bytes).sum::<usize>() > BYTES;
            if !crowded && !heavy {
                break;
            }
            let oldest = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry.drawn < self.frame && (crowded || entry.bytes > 0))
                .min_by_key(|(_, entry)| entry.drawn)
                .map(|(index, _)| index);
            let Some(index) = oldest else {
                break;
            };
            if let Picture::Ready(made) = self.entries.swap_remove(index).picture {
                self.gone.push(made);
            }
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

impl Weight for Made {
    fn bytes(&self) -> usize {
        (0..self.image.frame_count())
            .filter_map(|frame| self.image.as_bytes(frame))
            .map(<[u8]>::len)
            .sum()
    }
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

    /// A picture in these tests is a number, which is also what it costs.
    impl Weight for u32 {
        fn bytes(&self) -> usize {
            *self as usize
        }
    }

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

    /// Past [`BYTES`], the picture drawn longest ago goes as a heavy one
    /// lands, while one still being made, which costs nothing yet, stays;
    /// and nothing the frame being drawn has used goes, even past the bound.
    #[test]
    fn past_its_bytes_the_one_drawn_longest_ago_goes() {
        let mut book = Pictures::<u32>::default();
        let party = effects(&["p!"]);
        let half = (BYTES / 2) as u32;
        book.start_frame();
        book.start("making", party);
        for url in ["a", "b"] {
            book.start_frame();
            book.start(url, party);
            book.land(url, party, Some(half));
        }
        // Exactly at the bound, all of it is kept.
        assert_eq!(book.entries.len(), 3);
        book.start_frame();
        book.start("c", party);
        book.land("c", party, Some(1));
        assert_eq!(book.get("a", party), None);
        assert_eq!(book.get("making", party), Some(Picture::Making));
        assert_eq!(book.start_frame(), [half]);

        assert_eq!(book.get("b", party), Some(Picture::Ready(half)));
        assert_eq!(book.get("c", party), Some(Picture::Ready(1)));
        book.start("d", party);
        book.land("d", party, Some(half));
        assert_eq!(book.entries.len(), 4);
        assert_eq!(book.start_frame(), Vec::<u32>::new());
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
