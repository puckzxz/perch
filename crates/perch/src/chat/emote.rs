//! One emote in a chat line, as the chat view draws it: its picture, what
//! its effects make of it, the overlays laid over it, and what it says under
//! the pointer.
//!
//! What was written together and is drawn together is `emotes::stack`'s,
//! pure and tested; this draws the result. A stack is one piece of the
//! line, in the box its emote alone has always had: a line high,
//! `EMOTE_PAD_X` either side, the picture hanging past the line above and
//! below by `Metrics::overhang`. Each overlay is placed absolutely in that
//! same box and fitted to it, centred with its shape kept, rather than drawn
//! at its own size: wider, it would reach over the words beside it, and
//! taller, past the row's padding, where nothing may hang (HANDOFF, "Emotes
//! overhang their line"). So a stack never makes a row taller or moves a
//! word, and a wide overlay on a narrow emote is drawn smaller than 7TV's
//! own extension draws it.
//!
//! Overlays take no listener, tooltip or cursor, so gpui gives them no
//! hitbox and the pointer goes through them to the emote under them, whose
//! tooltip and right press answer for the whole stack.
//!
//! Effects (`w!`, `ffzX`) are drawn as BetterTTV draws them, which
//! `emotes::effects` works out. Wide and zero space only move the emote: its
//! box four emote heights across with the picture stretched to it, and the
//! box pulled back into the space before it, the two effects that change a
//! line's width, as they do in BetterTTV. The rest change the picture, and
//! gpui's image sprite has no transform (`PolychromeSprite`), so the picture
//! is made anew: the emote's file decoded and its frames flipped, turned,
//! recoloured or shaken, off the UI thread and once per pane for an emote and
//! its effects (`chat_effects`), then drawn as gpui draws any animated image,
//! where `emotes::effects::Placement` puts it — a turn in a square, a shake
//! hanging past its box by its reach. Until it is made, the emote's name
//! stands in, as it does while the file downloads; one that cannot be made is
//! drawn plain. Overlays on an emote with effects are laid over it plain, as
//! BetterTTV puts the class on the emote and not on what is laid over it. An
//! effect nobody knows yet is left out of the line, and the tooltip names it
//! as not drawn.

use std::path::{Path, PathBuf};

use emotes::effects::{WIDE, ZERO_SPACE};
use emotes::{Effects, Emote};
use gpui::{
    div, img, prelude::*, px, AnyElement, Context, MouseButton, ObjectFit, SharedString, Window,
};

use super::copy_menu::Target;
use super::ChatView;
use crate::chat_display::Metrics;
use crate::chat_effects::{self, Made, Picture};
use crate::chat_words;
use crate::theme;

/// What an emote is drawn from.
enum Drawn {
    /// Its own file, as its provider serves it.
    File(PathBuf),
    /// The picture its effects made of that file.
    Made(Made),
}

/// What a picture in a chat line says under the pointer: a line for each
/// picture or effect in it, its name and then what it is. Emote names are
/// worth showing on hover: half of chat is emotes, and knowing what one is
/// called is the difference between reading a message and guessing.
pub(super) struct EmoteTooltip {
    pub lines: Vec<(SharedString, SharedString)>,
}

impl EmoteTooltip {
    pub(super) fn new(lines: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            lines: lines
                .into_iter()
                .map(|(name, what)| (name.into(), what.into()))
                .collect(),
        }
    }
}

impl Render for EmoteTooltip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // At its natural width: a tooltip has no definite width to wrap
        // against, and a `max_w` on one does not help (HANDOFF, "Do not size
        // a tooltip with `max_w`").
        div()
            .px(px(theme::CONTROL_PAD_X))
            .py(px(theme::CONTROL_PAD_Y))
            .rounded(px(theme::RADIUS))
            .bg(theme::surface_raised())
            .flex()
            .flex_col()
            .gap(px(theme::GAP_WORD))
            .children(self.lines.iter().map(|(name, what)| {
                div()
                    .flex()
                    .flex_row()
                    .items_baseline()
                    .gap(px(theme::GAP_WORD))
                    .child(
                        div()
                            .text_size(px(theme::TEXT_LABEL))
                            .text_color(theme::text())
                            .child(name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(theme::TEXT_META))
                            .text_color(theme::text_dim())
                            .child(what.clone()),
                    )
            }))
    }
}

impl ChatView {
    /// One emote of a message line, with whatever is stacked on it; the
    /// `index`-th emote in the line, which keys its pictures' ids.
    pub(super) fn emote(
        &self,
        emote: Emote,
        index: usize,
        metrics: &Metrics,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // The right button says which emote it was, for the row's copy
        // menu, picture or not: the stack as it was written, so pasting it
        // draws the same picture.
        let pressed = Self::press_target(Target::Emote(emote.typed()), cx);
        let effects = Effects::of(&emote);
        // Its own file, or the picture its effects make of it when they
        // change the frames and not only where they go.
        let drawn = match self.cache.get_or_request(&emote.url) {
            Some(path) if effects.changes_frames() => {
                match self.effect_picture(&emote.url, &path, effects, cx) {
                    Picture::Making => None,
                    Picture::Ready(made) => Some(Drawn::Made(made)),
                    // Could not be made: the emote, plain.
                    Picture::Failed => Some(Drawn::File(path)),
                }
            }
            path => path.map(Drawn::File),
        };
        let Some(drawn) = drawn else {
            // Until the picture is there, show the emote's name so the
            // message still reads correctly. Its own name only: the stack's
            // words joined would be one run that cannot wrap.
            return div()
                .text_color(theme::text_dim())
                .on_mouse_down(MouseButton::Right, pressed)
                .child(SharedString::from(emote.name))
                .into_any_element();
        };
        // An overlay still on its way is simply not there until it lands.
        let overlays: Vec<_> = emote
            .overlays()
            .filter_map(|layer| {
                let path = self.cache.get_or_request(&layer.url)?;
                Some((SharedString::from(format!("over {}", layer.url)), path))
            })
            .collect();

        // The id is what makes animated emotes animate: GPUI keys per-frame
        // state on an element's global id, and only requests an animation
        // frame when one exists. An img without an id is pinned to frame 0
        // forever.
        //
        // The id must identify the *image*, not the slot. Keyed on position
        // alone, a 40-frame GIF in one row and a 1-frame PNG in another share
        // state, and GPUI indexes the PNG with the GIF's frame number and
        // panics. The emote is keyed on `Emote::id_key`: its URL when it
        // stands alone, and every picture's in a stack, whose tooltip says
        // more than the emote's alone and would otherwise keep the lone
        // emote's words. A picture its effects made has frames of its own (a
        // still emote shaken is ten), so its key is behind an `fx `, and the
        // emote drawn plain when that picture cannot be made never shares its
        // frame number. Each overlay is keyed on its own URL,
        // which `stack` keeps distinct within one stack, behind an `over `
        // that no `id_key` can start with. These ids are the pane's, not the
        // row's (nothing between `chat-pane` and a picture has an id), so the
        // same picture standing alone at the same index in another row would
        // otherwise share the overlay's state; and an element with no tooltip
        // builder takes down whatever tooltip it finds in its state
        // (`div.rs`), so the overlay would put out the lone emote's.
        let height = metrics.emote;
        // The box a made picture takes in the line, which the wrapper is
        // given below; a file's is its own shape at the emote height.
        let made_box = match &drawn {
            Drawn::Made(made) => Some(made.placement.width * height),
            Drawn::File(_) => None,
        };
        let picture = match drawn {
            // Where the effects put it in its box, stretched to that: its
            // margins from the box's top left, so a shake's reach hangs past
            // the box without moving a word.
            Drawn::Made(made) => {
                let at = made.placement.picture;
                img(made.image)
                    .id((SharedString::from(format!("fx {}", emote.id_key())), index))
                    .object_fit(ObjectFit::Fill)
                    .w(px(at.width * height))
                    .h(px(at.height * height))
                    .ml(px(at.x * height))
                    .mt(px(at.y * height - metrics.overhang))
            }
            Drawn::File(path) => img(path)
                // Before `.id(..)`: `image_cache` is on `Img`, and `.id(..)`
                // yields a `Stateful<Img>`.
                .image_cache(&self.emote_images)
                .id((SharedString::from(emote.id_key()), index))
                .h(px(height))
                .mt(px(-metrics.overhang))
                .when(effects.wide, |picture| {
                    picture.w(px(WIDE * height)).object_fit(ObjectFit::Fill)
                }),
        }
        // The right button and the tooltip listen on the picture, not the
        // wrapper: the picture overhangs the wrapper above and below, and a
        // press on the overhang is on the emote as much as one in the
        // middle. The tooltip only while the pointer is in the window, since
        // a tooltip outlives a pointer that leaves it (HANDOFF, GPUI traps);
        // its words are worked out only when it comes up.
        .on_mouse_down(MouseButton::Right, pressed)
        .when(self.window_hovered, move |picture| {
            picture.tooltip(move |_window, cx| {
                cx.new(|_| EmoteTooltip::new(chat_words::emote_tip(&emote)))
                    .into()
            })
        });

        // Emotes are taller than the text they sit in. Give the wrapper a
        // line's worth of height and let the image overhang it, so a row
        // with emotes is no taller than one without and the list keeps a
        // single vertical rhythm to scan down.
        div()
            .flex_none()
            .h(px(metrics.line))
            .px(px(theme::EMOTE_PAD_X))
            // A made picture's box is given, not left to the picture: taffy
            // sizes a block by a child's own width when it has one and leaves
            // its margins out (`determine_content_based_container_width`,
            // compute/block.rs), so a shaken emote would widen its line by
            // its reach, and a turned one wider than tall sit off its square,
            // over the next word. The width is the box and its padding, as
            // gpui sizes the border box.
            .when_some(made_box, |wrapper, width| {
                wrapper.w(px(width + 2.0 * theme::EMOTE_PAD_X))
            })
            .when(effects.zero_space, |wrapper| {
                wrapper.ml(px(-ZERO_SPACE * height))
            })
            .when(!overlays.is_empty(), |wrapper| wrapper.relative())
            .child(picture)
            .children(overlays.into_iter().map(|(url, path)| {
                // `w_full` is needed: `img` gives an auto width its own
                // shape's width at its height (`img.rs`), which insets on
                // both sides would lose to.
                img(path)
                    .image_cache(&self.emote_images)
                    .id((url, index))
                    .absolute()
                    .top(px(-metrics.overhang))
                    .left_0()
                    .w_full()
                    .h(px(height))
            }))
            .into_any_element()
    }

    /// What `effects` make of the emote at `url`, whose file is at `path`,
    /// as far as it has got. The first time it is wanted it is made from the
    /// file on a background thread, since a decode and a pass over every
    /// pixel of every frame are never the UI thread's, and handed to this
    /// pane, which draws again when it lands.
    fn effect_picture(
        &self,
        url: &str,
        path: &Path,
        effects: Effects,
        cx: &mut Context<Self>,
    ) -> Picture<Made> {
        let mut pictures = self.effect_pictures.borrow_mut();
        if let Some(picture) = pictures.get(url, effects) {
            return picture;
        }
        pictures.start(url, effects);
        drop(pictures);

        let (url, path) = (url.to_string(), path.to_path_buf());
        cx.spawn(async move |this, cx| {
            let made = cx
                .background_spawn(async move {
                    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
                    chat_effects::make(&bytes, effects)
                })
                .await
                .inspect_err(|e| eprintln!("emote effects: {url}: {e}"))
                .ok();
            this.update(cx, |this: &mut ChatView, cx| {
                this.effect_pictures.get_mut().land(&url, effects, made);
                cx.notify();
            })
            .ok();
        })
        .detach();
        Picture::Making
    }
}
