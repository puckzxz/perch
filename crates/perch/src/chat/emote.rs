//! One emote in a chat line, as the chat view draws it: its picture, the
//! overlays laid over it, and what it says under the pointer.
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
//! Effects (`w!`, `ffzX`) are recognised and not drawn: their word is left
//! out of the line, the emote under them is drawn plain, and the tooltip
//! names each one as not drawn. gpui's image sprite has no transform
//! (`PolychromeSprite`, `vendor/gpui/src/scene.rs`), so drawing one means
//! changing the decoded frames before gpui sees them; the stack keeps every
//! effect by name (`Emote::effects`) for whatever draws them.

use emotes::Emote;
use gpui::{div, img, prelude::*, px, AnyElement, Context, MouseButton, SharedString, Window};

use super::copy_menu::Target;
use super::ChatView;
use crate::chat_display::Metrics;
use crate::chat_words;
use crate::theme;

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
        let Some(path) = self.cache.get_or_request(&emote.url) else {
            // Until the image lands, show the emote's name so the message
            // still reads correctly. Its own name only: the stack's words
            // joined would be one run that cannot wrap.
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
        // emote's words. Each overlay is keyed on its own URL, which `stack`
        // keeps distinct within one stack, behind an `over ` that no
        // `id_key` can start with. These ids are the pane's, not the row's
        // (nothing between `chat-pane` and a picture has an id), so the same
        // picture standing alone at the same index in another row would
        // otherwise share the overlay's state; and an element with no tooltip
        // builder takes down whatever tooltip it finds in its state
        // (`div.rs`), so the overlay would put out the lone emote's.
        let picture = img(path)
            // Before `.id(..)`: `image_cache` is on `Img`, and `.id(..)`
            // yields a `Stateful<Img>`.
            .image_cache(&self.emote_images)
            .id((SharedString::from(emote.id_key()), index))
            .h(px(metrics.emote))
            .mt(px(-metrics.overhang))
            // The right button and the tooltip listen on the picture, not
            // the wrapper: the picture overhangs the wrapper above and below,
            // and a press on the overhang is on the emote as much as one in
            // the middle. The tooltip only while the pointer is in the
            // window, since a tooltip outlives a pointer that leaves it
            // (HANDOFF, GPUI traps); its words are worked out only when it
            // comes up.
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
                    .h(px(metrics.emote))
            }))
            .into_any_element()
    }
}
