//! Colour and spacing tokens.
//!
//! One place for every colour in the app. Scattered hex literals are how a UI
//! ends up looking incidental — a slightly different grey per file, borders that
//! do not agree, and no way to change the mood without a search-and-replace.
//!
//! The direction is deliberately quiet: this is a window you leave open for
//! hours, so the chrome should recede and the video should be the only bright
//! thing on screen.

use std::time::Duration;

use gpui::{ease_in_out, ease_out_quint, rgb, rgba, FontWeight, Hsla, Rgba};

/// Behind the video, and nothing else. Pure black rather than near-black: any
/// lift here shows as a grey halo around letterboxed content, which is exactly
/// where the eye is least forgiving.
pub fn player_bg() -> Hsla {
    rgb(0x000000).into()
}

// ── Surfaces, darkest to lightest ────────────────────────────────────

/// The window's base layer.
pub fn bg() -> Hsla {
    rgb(0x0d0d10).into()
}

/// Panels sitting on the base: chat, browse cards, the sidebar.
pub fn surface() -> Hsla {
    rgb(0x131317).into()
}

/// Raised things: hovered rows, popovers, the settings sheet.
pub fn surface_raised() -> Hsla {
    rgb(0x1a1a21).into()
}

/// Hover wash over an existing surface, kept translucent so it works on top of
/// whatever is beneath rather than needing a variant per background.
pub fn hover() -> Hsla {
    rgba(0xffffff0a).into()
}

/// Held down. Stronger than `hover` rather than a different colour, so a press
/// reads as more of the same gesture instead of a separate state.
pub fn pressed() -> Hsla {
    rgba(0xffffff1a).into()
}

/// The contrast every piece of text in this app is held to, against whatever
/// it sits on. WCAG AA for text below 18.66px bold, which is all of it.
pub const MIN_CONTRAST: f32 = 4.5;

/// sRGB relative luminance, per WCAG 2.
///
/// Here rather than in the tests because [`readable`] needs it at runtime: a
/// username is lifted until it *measures* legible, not until it reaches a
/// lightness that usually is.
fn luminance(color: Hsla) -> f32 {
    let rgba: Rgba = color.into();
    let channel = |c: f32| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(rgba.r) + 0.7152 * channel(rgba.g) + 0.0722 * channel(rgba.b)
}

/// WCAG contrast ratio between two colours, 1:1 to 21:1.
pub fn contrast(a: Hsla, b: Hsla) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// How many halving steps [`readable`] spends finding the lightness it needs.
/// Ten puts it within 0.1% of the true floor, which is finer than the eye or
/// the 8-bit colour it ends up as.
const READABLE_STEPS: u32 = 10;

/// A username colour that can be read on this background.
///
/// Hue and saturation are kept and only lightness is lifted, so people stay
/// recognisable by colour rather than being flattened to one. The lift stops
/// at a *measured* [`MIN_CONTRAST`] rather than at a fixed lightness, because
/// the two are not the same thing: blue reads far darker than yellow at equal
/// lightness, so the flat floor this replaces left pure blue at 2.7:1 — under
/// the bar the whole function exists to clear.
///
/// Luminance rises monotonically with lightness at fixed hue and saturation,
/// so bisection finds the floor; the alternative is stepping, which costs a
/// hundred times as much for every name on screen every frame.
pub fn readable(color: u32) -> Hsla {
    let mut color: Hsla = rgb(color).into();
    if contrast(color, surface()) >= MIN_CONTRAST {
        return color;
    }

    let (mut low, mut high) = (color.l, 1.0);
    for _ in 0..READABLE_STEPS {
        let mid = (low + high) / 2.0;
        color.l = mid;
        if contrast(color, surface()) >= MIN_CONTRAST {
            high = mid;
        } else {
            low = mid;
        }
    }
    // Land on the light side of the bracket: the low end is the last value
    // known to be *under* the bar.
    color.l = high;
    color
}

/// Every other chat row, for the same reason ledgers have ruled lines.
pub fn stripe() -> Hsla {
    rgba(0xffffff05).into()
}

/// Behind a chat row that is an event rather than a message — a sub, a raid,
/// an announcement.
///
/// A wash rather than a rule or a left bar: the row still has to sit inside the
/// ruler of timestamps you scan down, and anything that changes its geometry
/// puts a jog in that column for the sake of one row.
pub fn event_wash() -> Hsla {
    rgba((ACCENT << 8) | 0x14).into()
}

/// The louder wash, for the two events worth interrupting a read: a raid
/// changes who is in the room, and an announcement is the broadcaster rather
/// than the chat. Everything else Twitch invents gets the quiet one.
pub fn event_wash_loud() -> Hsla {
    rgba((ACCENT << 8) | 0x33).into()
}

/// Scrim behind a modal.
pub fn scrim() -> Hsla {
    rgba(0x00000099).into()
}

/// Behind a badge on a picture: a viewer count on a thumbnail, a recording's
/// length, the seek bar's time under the pointer.
///
/// Darker than [`scrim`], because this one is not dimming what is behind it —
/// it is making its own contrast on top of an image that could be any colour at
/// all in the next frame. But not dark enough for every tier: over the worst
/// picture, a white frame, only [`text`] passes on it (10.2:1). `text_muted`
/// (4.4), `text_dim` (3.6) and the accent (4.0) all fall under the bar there,
/// so nothing but full-strength text is ever drawn on this wash. Controls and
/// quieter tiers go on [`video_chrome`], which carries them all.
pub fn overlay() -> Hsla {
    rgba(0x000000cc).into()
}

/// Behind controls and pane facts drawn over a playing picture: the player's
/// control bar, and anything else that has to carry more than one tier of text
/// on top of video.
///
/// Denser than [`overlay`] because it carries every tier, not just full text,
/// and each one is measured against the worst picture there is — a white frame
/// under the wash: text 13.3, text_muted 5.8, text_dim 4.7, accent 5.2 and
/// danger 6.8:1. `every_tier_drawn_on_a_picture_reads_over_a_white_frame` holds
/// it there. One wash for everything on a picture, so two strips over the same
/// frame never disagree about how dark the video behind them is.
pub fn video_chrome() -> Hsla {
    rgba(0x000000e0).into()
}

/// The brightest a picture can be under this wash: a white frame with the wash
/// drawn over it. The one spelling of "the worst case for text on video", for
/// every test that measures something drawn on a picture. `blend` draws its
/// argument over the colour it is called on.
#[cfg(test)]
pub(crate) fn over_white(wash: Hsla) -> Hsla {
    Hsla::from(rgb(0xffffff)).blend(wash)
}

// ── Lines ────────────────────────────────────────────────────────────

/// Structural borders between panes.
pub fn border() -> Hsla {
    rgb(0x24242c).into()
}

/// The seek bar's rail: the part of a recording not yet played. It sits on
/// the control bar's [`video_chrome`], over live video, so it is a wash rather
/// than a colour of its own, and a stronger one than a divider because a
/// four-pixel line has to survive whatever the picture is doing behind it.
pub fn seek_rail() -> Hsla {
    rgba(0xffffff40).into()
}

/// The stretch of that rail between the playhead and the pointer: where a
/// press would skip to. The same wash, a step stronger, so it reads as more of
/// the rail rather than as a second bar competing with the played part.
pub fn seek_hover() -> Hsla {
    rgba(0xffffff80).into()
}

/// Hairlines within a pane, e.g. between chat messages. Deliberately fainter
/// than `border`, since separating peers needs less weight than separating
/// regions.
pub fn divider() -> Hsla {
    rgba(0xffffff0d).into()
}

// ── Text ─────────────────────────────────────────────────────────────

pub fn text() -> Hsla {
    rgb(0xe8e6ed).into()
}

/// Labels, metadata, anything supporting.
pub fn text_muted() -> Hsla {
    rgb(0x9a97a5).into()
}

/// The quietest tier: timestamps, help text, a game name under a title.
///
/// Quiet is a job; invisible is not. The value this replaced measured 3.16:1
/// on [`surface`] and 2.95:1 on [`surface_raised`], both under AA — and it was
/// not carrying placeholders, it was carrying the game name you pick a stream
/// by, the names of every offline channel you follow, and every line of help
/// in the settings sheet. `every_text_tier_is_legible` holds it to the bar now.
pub fn text_dim() -> Hsla {
    rgb(0x8a8794).into()
}

// ── Accent and status ────────────────────────────────────────────────

/// The accent, as a bare RGB so the washes and the dim variant cannot drift
/// from it. See [`accent`] for how the value was chosen.
const ACCENT: u32 = 0x35a094;

/// Used sparingly: selection, focus, the one control that matters in a view.
///
/// Teal, sharing the icon's hue, and picked by measurement rather than by eye.
/// Its relative luminance is within 2% of the `#9d7bff` it replaces, which is
/// the property that matters: an accent appears on every focus ring and every
/// chat link, so a brighter one would quietly undo the opening paragraph of
/// this file. The obvious brighter teal was 11% up and did exactly that.
///
/// It measures 5.8:1 against [`surface`], past AA for the smallest thing it is
/// used on, which is a chat link at [`TEXT_BODY`]. Both claims are checked by
/// `accent_carries_its_weight` rather than left as assertions.
///
/// The purple it replaces sat a few degrees off Twitch's own. A fair joke while
/// the app was called nativetwitch; a trademark question once it was not.
pub fn accent() -> Hsla {
    rgb(ACCENT).into()
}

pub fn accent_dim() -> Hsla {
    rgba((ACCENT << 8) | 0x33).into()
}

/// The live dot. The only saturated red in the app, so it reads as status
/// rather than decoration — with one exception, [`caption_close`], which is
/// the platform's red rather than the app's and only shows under the pointer.
pub fn live() -> Hsla {
    rgb(0xe5534b).into()
}

pub fn danger() -> Hsla {
    rgb(0xf08a80).into()
}

// ── Window frame ─────────────────────────────────────────────────────

/// The close button under the pointer, on a title bar Perch draws itself.
///
/// Windows' own red rather than [`live`] or [`danger`]: every other window on
/// the desktop turns this colour under the pointer, and a close that turns a
/// different one reads as a different control. Its own token because it is
/// the platform's and not the app's, and it is never at rest on screen.
pub fn caption_close() -> Hsla {
    rgb(0xe81123).into()
}

/// The close button held down. Darker rather than lighter, the way the
/// platform's own does it, so a press reads as the same red pushed in.
pub fn caption_close_pressed() -> Hsla {
    rgb(0xc50f1f).into()
}

/// The close glyph on either red. White, not [`text`]: the app's off-white
/// measures about 3.7:1 on [`caption_close`], under [`MIN_CONTRAST`], and
/// white clears it — `the_caption_close_glyph_reads_on_its_red` holds it there.
pub fn caption_close_glyph() -> Hsla {
    rgb(0xffffff).into()
}

// ── Type ─────────────────────────────────────────────────────────────
//
// Four roles rather than four sizes. The app previously used one size,
// text_xs, for nineteen different jobs - button labels, chat notices, stream
// metadata, settings labels - so nothing had rank. Weight was the same story:
// BOLD was the only weight in the codebase, which means emphasis had no
// degrees, only on and off.
//
// There were five. `TEXT_MICRO` and `weight_shout` existed for one element, the
// `LIVE` badge on a browse card, and went when it did - a card in a live-only
// list was wearing a badge that said the same thing on every card in every
// list. A role nothing plays is not a role.

/// Page and panel titles.
pub const TEXT_TITLE: f32 = 15.0;
/// Chat messages and card names: the content you actually read.
pub const TEXT_BODY: f32 = 13.0;
/// Interactive control labels. Same size as meta but a heavier weight, so a
/// thing you can click never looks like a thing you can only read.
pub const TEXT_LABEL: f32 = 11.5;
/// Supporting information: viewers, uptime, status, help text.
pub const TEXT_META: f32 = 11.5;
/// Leading for running text. Chat is dense and repetitive; default leading
/// makes consecutive lines hard to separate.
pub const LINE_BODY: f32 = 19.0;
/// Leading for single-line labels, where extra space just inflates the row.
pub const LINE_TIGHT: f32 = 15.0;

/// Titles and names. Semibold rather than bold, leaving bold for the one
/// element that genuinely has to shout.
pub fn weight_title() -> FontWeight {
    FontWeight::SEMIBOLD
}

/// Control labels: enough weight to read as interactive, not enough to compete
/// with a title.
pub fn weight_label() -> FontWeight {
    FontWeight::MEDIUM
}

// ── Spacing ──────────────────────────────────────────────────────────
//
// Named by role rather than by size. The point is not the numbers but that
// two things playing the same role get the same value: the app previously
// mixed px_2/px_3/px_4 for the same kind of padding in different files, which
// is what made it read as unconsidered rather than any single gap being wrong.

/// Corner radius for a control: a pill, a button, a text field, a dropdown.
/// Shared with the widget library through [`crate::widget_theme`], so a button
/// this app draws and one `gpui-component` draws are the same shape.
pub const RADIUS: f32 = 4.0;
/// Corner radius for a surface: a card, a panel, a sheet.
pub const RADIUS_LG: f32 = 8.0;

/// Outer margin of a page.
pub const PAGE_PAD: f32 = 20.0;
/// The height of the browse page's tab strip: the tabs, then Refresh.
///
/// Fixed rather than falling out of its padding, because the toast stack on
/// the browse page is offset by it. Toasts are anchored to the top-right of
/// the content area under the title bar, and the strip is left-aligned, so at
/// a comfortable width the two never meet — but in a narrow window Refresh
/// reaches the right-hand side, where a "went live" toast arriving on top of
/// it would take the click meant for it. One constant read by both is what
/// makes them agree by construction rather than by a number nudged until it
/// looked right.
pub const TAB_STRIP_HEIGHT: f32 = 52.0;
/// How wide the title bar's search box is when the bar has the room: wide
/// enough for a channel's name and a few words of a game.
pub const SEARCH_WIDTH: f32 = 260.0;
/// The least the search box shrinks to in a narrow window: still room for a
/// name to be typed and seen. The bar's drag strip never shrinks below
/// [`TITLE_BAR_DRAG_MIN`], so in a window narrower still the box does not
/// take the strip's room. It is cut off at its right end instead; see
/// `root::title_bar`.
pub const SEARCH_MIN_WIDTH: f32 = 160.0;

/// The title bar Perch draws in place of the platform's.
///
/// `layout::title_bar_height` and `layout::Body` take it off the window, so
/// the pages, the watch grid and the divider maths all measure what is left
/// under the bar; nothing else may assume the bar's height. Forty so the macOS
/// traffic lights can sit centred in it — a choice still to be checked on a
/// Mac, like the [`TRAFFIC_LIGHT_X`] numbers it is meant to agree with.
pub const TITLE_BAR_HEIGHT: f32 = 40.0;
/// The strip along the top of a windowed title bar left to the platform's
/// resize edge rather than to dragging — about one logical `SM_CYFRAME`, which
/// is the band gpui answers `HTTOP` in on Windows. `layout::drag_top` is the
/// only reader, and it gives the strip back when the window is maximised.
pub const TITLE_BAR_RESIZE_BAND: f32 = 4.0;
/// The least of the title bar that stays empty for dragging, however much
/// else the bar comes to hold. A window with nothing left to grab is one that
/// can no longer be moved.
pub const TITLE_BAR_DRAG_MIN: f32 = 48.0;
/// One of the minimise, maximise and close buttons Perch draws on Windows:
/// the platform's own width, so the three sit where a hand expects them.
/// From memory rather than a measurement; check it against a native window.
pub const CAPTION_BUTTON_WIDTH: f32 = 46.0;
/// The box a caption button's glyph is drawn in, which is not the glyph's
/// size: the icons leave about a quarter of their box empty on each side, so
/// this draws the cross and the dash at about eight pixels with a stroke of a
/// pixel and a third. Meant to sit near the platform's own glyphs, which is a
/// recollection rather than a measurement; check it against a native window.
pub const CAPTION_GLYPH: f32 = 16.0;
/// An icon on a control, and the square control it sits in. The square is a
/// comfortable target at the bar's height without filling it edge to edge.
pub const ICON: f32 = 16.0;
pub const ICON_BUTTON: f32 = 28.0;
/// Where macOS puts the traffic lights, measured from the window's top-left,
/// and how much of the bar's left end is kept clear for them. All three are
/// unverified guesses until someone tunes them on a Mac; gpui measures the
/// lights against AppKit's own title strip, not against this bar.
pub const TRAFFIC_LIGHT_X: f32 = 13.0;
pub const TRAFFIC_LIGHT_Y: f32 = 13.0;
pub const TRAFFIC_LIGHT_INSET: f32 = 80.0;

/// Inside a card, panel or sheet.
pub const PANEL_PAD: f32 = 12.0;
/// Inside a pill or button.
pub const CONTROL_PAD_X: f32 = 10.0;
pub const CONTROL_PAD_Y: f32 = 5.0;
/// Above and below the text of a badge on a picture — a thumbnail's viewer
/// count, the seek bar's time. More than a tag gets, because it has an image
/// to stand out from rather than a panel.
pub const BADGE_PAD_Y: f32 = 3.0;
/// Above and below a tag's word in a pane header. Slim, so `muted` does not
/// make its row taller than the channel's name.
pub const TAG_PAD_Y: f32 = 1.0;
/// Between words in a sentence. Narrower than `GAP_TIGHT`, which was doing
/// this job and is wider than a real word space at `TEXT_BODY`.
pub const GAP_WORD: f32 = 4.0;
/// Between a label and the thing it labels.
pub const GAP_TIGHT: f32 = 6.0;
/// Between peers in a row or column.
pub const GAP: f32 = 10.0;
/// Between distinct sections.
pub const GAP_SECTION: f32 = 18.0;
/// Seam between panes in the watch grid. Deliberately thin: it separates
/// pictures, and anything wider reads as a border around each one.
pub const PANE_GAP: f32 = 3.0;
/// Vertical rhythm inside a chat row.
pub const ROW_PAD_X: f32 = 12.0;
pub const ROW_PAD_Y: f32 = 5.0;
/// Breathing room either side of an emote. Emotes need more air than words
/// do, and `GAP_WORD` alone crowds them.
pub const EMOTE_PAD_X: f32 = 2.0;

// ── Motion ────────────────────────────────────────────────────────────
//
// Named by what the movement is *for*, like spacing. Motion here has one job:
// to say that a thing changed, rather than that a different thing is now on
// screen. Anything long enough to wait for is too long.

/// Revealing or hiding something under the pointer. Short enough that the
/// control feels attached to the cursor rather than chasing it.
pub const MOTION_HOVER: Duration = Duration::from_millis(120);
/// Something arriving or leaving of its own accord: a menu, a toast, a page.
/// These get slightly longer because you did not ask for them at a precise
/// moment, so there is nothing for them to feel behind.
pub const MOTION_ENTER: Duration = Duration::from_millis(200);
/// First frames coming up from black. Deliberately the slowest thing in the
/// app: it is a picture resolving, not a control responding, and cutting
/// straight to video reads as a glitch.
pub const MOTION_VIDEO: Duration = Duration::from_millis(300);
/// One breath of a waiting indicator. Slow on purpose — a fast pulse reads as
/// alarm, and this only ever means "still working".
pub const PULSE_PERIOD: Duration = Duration::from_millis(1600);
/// How faint a waiting indicator gets at the bottom of its breath. Never zero:
/// something that vanishes entirely looks broken rather than busy.
pub const PULSE_FLOOR: f32 = 0.45;

/// For a two-way change — visible to hidden and back. Symmetric, because the
/// reveal and the hide are one event in opposite directions.
pub fn ease_fade() -> impl Fn(f32) -> f32 {
    ease_in_out
}

/// For a one-way arrival. Decelerating, so the thing leaves the mark
/// immediately and settles, rather than sliding to a stop.
pub fn ease_enter() -> impl Fn(f32) -> f32 {
    ease_out_quint()
}

// ── Metrics ──────────────────────────────────────────────────────────

/// What a hand-edited `chat_width` is clamped to. There is no height pair:
/// stacked chat takes whatever the video leaves rather than a stored size.
pub const CHAT_WIDTH_MIN: f32 = 260.0;
pub const CHAT_WIDTH_MAX: f32 = 640.0;

/// What a hand-dragged `video_share` is clamped to.
///
/// Both ends leave the other half of the cell usable: a video squeezed under a
/// third of the pane is not worth watching, and one over four fifths leaves
/// chat too short to read a sentence in.
pub const VIDEO_SHARE_MIN: f32 = 0.3;
pub const VIDEO_SHARE_MAX: f32 = 0.8;

/// How wide the mini player's picture is: one tile with one stream playing,
/// two side by side with more. Fixed, so the player grows by rows and never
/// reaches further across the cards than this. `layout::mini_player` divides
/// it into tiles.
///
/// Larger than the old 96px thumbnail on purpose — big enough to follow a
/// game in — and render size follows the element, so a tile this wide is
/// scaled into a buffer this wide. The rendition is not: see
/// `RootView::go_browse`. The player is wider than this by its
/// [`MINI_PLAYER_INSET`] either side and its border.
pub const MINI_PLAYER_WIDTH: f32 = 320.0;
/// The strip under the mini player's tiles: what is playing, and the three
/// controls for all of it. Tall enough for an `ICON_BUTTON` with room round it.
pub const MINI_BAR_HEIGHT: f32 = 36.0;
/// How far inside the mini player's edge its tiles and bar sit. Enough to keep
/// a tile's square corners inside the player's [`RADIUS_LG`] curve: gpui clips
/// to rectangles, never to rounded corners, so a tile flush with the edge
/// would square the player off with the picture's black.
pub const MINI_PLAYER_INSET: f32 = 4.0;

/// How wide a vertical scrollbar's track is, down the right edge of the list
/// it scrolls: the thumb at its widest, eight pixels, with four either side.
/// gpui-component draws it and keeps the number to itself (`WIDTH` in its
/// `scroll/scrollbar.rs`), so it is written out here for what floats at a
/// list's right edge and has to leave the track clear — the mini player; see
/// `layout::mini_player_right`. Read from gpui-component 0.5.1, the version
/// `Cargo.lock` holds, and `the_scrollbar_width_is_read_from_the_locked_library`
/// fails once that version moves, so it is read again rather than trusted.
pub const SCROLLBAR_WIDTH: f32 = 16.0;

/// The grab area of a pane divider. Wider than the line it draws, because a
/// 1px target is a game rather than a control.
pub const DIVIDER_GRAB: f32 = 6.0;

/// The seek bar's rail, its thumb, and the strip that takes the pointer. A
/// four-pixel line is not a target, so the hit area is taller than the bar it
/// holds, with the rail centred in it.
pub const SEEK_RAIL: f32 = 4.0;
pub const SEEK_THUMB: f32 = 12.0;
pub const SEEK_HIT: f32 = 18.0;

/// The player's menus (`video_view::menu`): the least width one takes,
/// however short its rows; where it rests above the bar's right-hand
/// cluster, an icon button's height and a word's gap up from the cluster's
/// foot so it clears the button it came from; and how much lower than that
/// it starts as it rises into place.
pub const MENU_MIN_WIDTH: f32 = 120.0;
pub const MENU_BOTTOM: f32 = ICON_BUTTON + GAP_WORD;
pub const MENU_RISE: f32 = 6.0;

/// The live indicator's diameter. Small enough to read as a status mark
/// beside a number rather than as a control.
pub const LIVE_DOT: f32 = 6.0;

/// Below this window aspect ratio the window is treated as portrait and chat
/// moves under the video instead of beside it.
pub const PORTRAIT_ASPECT: f32 = 1.1;

#[cfg(test)]
mod tests {
    use super::*;

    /// Every surface a piece of text is ever drawn on.
    fn surfaces() -> [(&'static str, Hsla); 3] {
        [
            ("bg", bg()),
            ("surface", surface()),
            ("surface_raised", surface_raised()),
        ]
    }

    /// The two things [`accent`] claims about itself.
    ///
    /// Both were asserted in a comment first and turned out to be worth
    /// checking: the colour originally chosen by eye read 11% brighter than the
    /// one it replaced, which is exactly the kind of drift that turns a quiet
    /// UI into a loud one an accent at a time.
    #[test]
    fn accent_carries_its_weight() {
        /// The purple this replaced. Kept as the reference weight, not because
        /// anyone wants it back.
        const PREVIOUS: u32 = 0x9d7bff;

        let previous = luminance(rgb(PREVIOUS).into());
        let drift = (luminance(accent()) - previous).abs() / previous;
        assert!(
            drift < 0.05,
            "the accent is {:.1}% off the weight it replaced; a brighter one              makes every focus ring and link louder",
            drift * 100.0
        );

        let ratio = contrast(accent(), surface());
        assert!(
            ratio >= MIN_CONTRAST,
            "the accent reads {ratio:.2}:1 on surface(), under AA for a chat link"
        );
    }

    /// A tier can be quiet without being unreadable, and the difference is a
    /// number rather than a matter of taste.
    ///
    /// `text_dim` used to fail this on two of the three surfaces while carrying
    /// game names, offline channel names and the whole of the settings help —
    /// which is how a token named for placeholders ends up holding content.
    #[test]
    fn every_text_tier_is_legible() {
        let tiers: [(&str, Hsla); 4] = [
            ("text", text()),
            ("text_muted", text_muted()),
            ("text_dim", text_dim()),
            ("danger", danger()),
        ];

        for (tier, color) in tiers {
            for (surface_name, surface) in surfaces() {
                let ratio = contrast(color, surface);
                assert!(
                    ratio >= MIN_CONTRAST,
                    "{tier} reads {ratio:.2}:1 on {surface_name}, under AA at every size this app uses"
                );
            }
        }
    }

    /// Anything drawn on a picture is held to the brightest picture there is:
    /// a white frame, under the wash it is drawn on.
    ///
    /// Every tier passes on [`video_chrome`], which is why the control bar and
    /// anything else carrying quieter text over video use it. On [`overlay`]
    /// only full-strength text passes; `text_muted`, `text_dim` and the accent
    /// all fall under the bar there, which is why badges carry `text()` and
    /// nothing else.
    #[test]
    fn every_tier_drawn_on_a_picture_reads_over_a_white_frame() {
        let chrome = over_white(video_chrome());
        let tiers: [(&str, Hsla); 5] = [
            ("text", text()),
            ("text_muted", text_muted()),
            ("text_dim", text_dim()),
            ("accent", accent()),
            ("danger", danger()),
        ];
        for (tier, color) in tiers {
            let ratio = contrast(color, chrome);
            assert!(
                ratio >= MIN_CONTRAST,
                "{tier} reads {ratio:.2}:1 on video_chrome over a white frame"
            );
        }

        let ratio = contrast(text(), over_white(overlay()));
        assert!(
            ratio >= MIN_CONTRAST,
            "text reads {ratio:.2}:1 on overlay over a white frame"
        );
    }

    /// The tiers have to stay *apart*, or three names for one grey is all they
    /// are. Ranked by contrast rather than by hex, since that is what the eye
    /// sorts them by.
    #[test]
    fn the_text_tiers_are_ordered_and_distinct() {
        let ladder = [text(), text_muted(), text_dim()];
        for pair in ladder.windows(2) {
            let (louder, quieter) = (contrast(pair[0], surface()), contrast(pair[1], surface()));
            assert!(
                louder > quieter * 1.15,
                "{louder:.2}:1 and {quieter:.2}:1 are too close to read as different tiers"
            );
        }
    }

    /// The close button's glyph has to read on both of its reds. Checked
    /// against the reds rather than `surfaces()`, because those are the only
    /// things it is ever drawn on — at rest it is an ordinary muted glyph.
    ///
    /// The app's own `text()` is deliberately not the glyph, nor `live()` the
    /// red: `text()` measures about 3.7:1 on the hover red and about 3.0:1 on
    /// `live()`, both under the bar.
    #[test]
    fn the_caption_close_glyph_reads_on_its_red() {
        for (state, red) in [
            ("hovered", caption_close()),
            ("pressed", caption_close_pressed()),
        ] {
            let ratio = contrast(caption_close_glyph(), red);
            assert!(
                ratio >= MIN_CONTRAST,
                "the close glyph reads {ratio:.2}:1 on its {state} red"
            );
        }
    }

    /// [`SCROLLBAR_WIDTH`] copies a number gpui-component keeps private, so
    /// nothing can check it against the library directly. What can be checked
    /// is that the library is still the version it was read from: when
    /// `Cargo.lock` moves gpui-component on, this fails until somebody has
    /// read the new `scroll/scrollbar.rs`, set the width to what it says, and
    /// moved the version here to match.
    #[test]
    fn the_scrollbar_width_is_read_from_the_locked_library() {
        const LOCK: &str = include_str!("../../../Cargo.lock");
        let version = LOCK
            .lines()
            .skip_while(|line| *line != r#"name = "gpui-component""#)
            .nth(1);
        assert_eq!(
            version,
            Some(r#"version = "0.5.1""#),
            "gpui-component has moved; read SCROLLBAR_WIDTH from it again"
        );
    }

    /// Everything tinted with the accent has to come from the same value, or a
    /// retheme leaves a stray hue behind in the one place nobody looks.
    #[test]
    fn the_accent_tints_all_share_one_source() {
        assert_eq!(accent(), rgb(ACCENT).into());
        assert_eq!(accent_dim(), rgba((ACCENT << 8) | 0x33).into());
        assert_eq!(event_wash(), rgba((ACCENT << 8) | 0x14).into());
        assert_eq!(event_wash_loud(), rgba((ACCENT << 8) | 0x33).into());
    }

    /// The palette Twitch itself hands out is the worst case: it contains pure
    /// blue, firebrick and seagreen, all darker than the surface they land on.
    /// Tested against the real array rather than a copy, so the two cannot
    /// drift apart.
    ///
    /// Measured, not assumed: the lightness floor this replaced passed every
    /// one of these while leaving pure blue at 2.7:1.
    #[test]
    fn every_default_username_colour_clears_the_background() {
        for color in twitch_chat::message::DEFAULT_COLORS {
            let ratio = contrast(readable(color), surface());
            assert!(
                ratio >= MIN_CONTRAST,
                "{color:#08x} came out at {ratio:.2}:1"
            );
        }
    }

    /// The one the lightness floor missed, kept as its own case because it is
    /// the reason the floor became a measurement.
    #[test]
    fn pure_blue_is_lifted_until_it_is_actually_readable() {
        let raw: Hsla = rgb(0x0000ff).into();
        assert!(
            contrast(raw, surface()) < MIN_CONTRAST,
            "pure blue should need lifting"
        );

        let lifted = readable(0x0000ff);
        assert!(contrast(lifted, surface()) >= MIN_CONTRAST);
        assert_eq!(lifted.h, raw.h, "the hue is the person");
        assert!(lifted.l > raw.l);
    }

    /// A colour that is already legible must be left alone. Lifting everything
    /// to the same brightness would stop the colour identifying anyone.
    #[test]
    fn colours_that_are_already_light_enough_are_untouched() {
        let raw: Hsla = rgb(0xff7f50).into();
        assert!(
            contrast(raw, surface()) > MIN_CONTRAST,
            "coral should not need lifting"
        );
        assert_eq!(readable(0xff7f50), raw);
    }

    /// Lifting further than necessary is the same flattening the floor caused,
    /// arrived at politely: a colour taken well past the bar stops being that
    /// person's colour. Anything that *was* lifted has to sit within a hair of
    /// where it became readable.
    #[test]
    fn the_lift_stops_as_soon_as_it_clears_the_bar() {
        for color in twitch_chat::message::DEFAULT_COLORS {
            let raw: Hsla = rgb(color).into();
            let lifted = readable(color);
            if lifted.l == raw.l {
                continue; // Already legible, and returned untouched.
            }

            let mut under = lifted;
            under.l -= 0.02;
            assert!(
                contrast(under, surface()) < MIN_CONTRAST,
                "{color:#08x} was lifted to {:.3} when {:.3} would have done",
                lifted.l,
                under.l
            );
        }
    }
}
