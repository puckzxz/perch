//! Choosing how to arrange N players in a window.
//!
//! Rather than a lookup table of "2 streams means side by side", the shape is
//! derived: try every column count, and pick the one whose resulting cells come
//! closest to the shape a video pane actually wants. That falls out correctly
//! for an ultrawide, a square window and a vertical monitor without any of them
//! being special-cased.
//!
//! It also owns the few numbers more than one part of the window has to agree
//! on: how much of the window the title bar takes, whether the rail is drawn,
//! and where on the bar a drag begins; how the mini player is cut into tiles,
//! how far in from the page's edge it floats, and how much room a browse list
//! leaves at its foot so nothing ends up stuck under the player. Each is read
//! from here rather than recomputed where it is used.

use gpui::{Pixels, Size};

/// The room a page has: the window less the rail and the title bar.
///
/// One type, built in one place, so that nothing laying out a page can be
/// handed the viewport by mistake. The watch grid was, once: with the rail
/// out, every stacked pane carried a black band under its picture, because
/// its 16:9 box had been derived from a cell wider than the one it was drawn
/// in - by exactly the rail's share of the width. Every consumer now takes a
/// `Body`, and the only way to make one is from the viewport, the rail and
/// the bar above the page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub width: f32,
    pub height: f32,
}

impl Body {
    /// The viewport less `rail_width`, which is zero when the rail is not
    /// drawn (see [`rail_shown`]), and less `top_inset`, the title bar's
    /// [`title_bar_height`].
    pub fn of(viewport: Size<Pixels>, rail_width: f32, top_inset: f32) -> Self {
        Self {
            width: (f32::from(viewport.width) - rail_width).max(0.0),
            height: (f32::from(viewport.height) - top_inset).max(0.0),
        }
    }

    /// Width over height, guarding against a zero-height window during a
    /// minimise.
    pub fn aspect(&self) -> f32 {
        self.width / self.height.max(1.0)
    }
}

/// How much of the window's height the title bar takes: all of
/// `TITLE_BAR_HEIGHT`, or none in fullscreen, where the bar is not drawn and
/// the picture gets the whole screen.
pub fn title_bar_height(fullscreen: bool) -> f32 {
    if fullscreen {
        0.0
    } else {
        crate::theme::TITLE_BAR_HEIGHT
    }
}

/// Whether the rail is drawn: not while it is folded away, and not in
/// fullscreen either, whichever way it was left.
///
/// Fullscreen is the picture alone, and the rail's own control is the
/// title-bar button, which goes with the bar. Left on screen there, the rail
/// was a column the mouse could neither fold nor bring back.
/// `RootView::body` reads this as well as the render that draws the rail, so
/// the room a page is laid out in matches what is beside it.
pub fn rail_shown(collapsed: bool, fullscreen: bool) -> bool {
    !collapsed && !fullscreen
}

/// How far below the window's top edge the title bar starts answering as a
/// drag handle or a caption button.
///
/// Windowed, it leaves `TITLE_BAR_RESIZE_BAND` to the platform: gpui asks for
/// a window control before it asks Windows about the frame, so a drag area
/// reaching the very top would take the top resize edge with it. Maximised,
/// it is nothing, because there is no edge to resize — gpui skips that band
/// there — and a pointer flung to the top of the screen has to land on the
/// bar rather than on a strip that does nothing.
pub fn drag_top(maximized: bool) -> f32 {
    if maximized {
        0.0
    } else {
        crate::theme::TITLE_BAR_RESIZE_BAND
    }
}

/// What a browse list has to work with: how wide it is, which sizes its
/// cards, and how much it leaves free at its foot, which is what keeps its
/// last row from ending up under the mini player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Room {
    pub width: f32,
    pub bottom: f32,
}

/// How the mini player arranges what is playing: its grid, the size of each
/// tile, and how tall the whole player is, bar included.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MiniLayout {
    pub cols: usize,
    pub rows: usize,
    /// Each tile, in definite pixels: a player inside a box with no definite
    /// size lets the frame's own aspect decide the box (see the `img` trap in
    /// the handoff), and a tile is exactly such a box.
    pub tile_w: f32,
    pub tile_h: f32,
    /// The whole player inside its border: the tiles, the seams between their
    /// rows and the bar under them, inset from the edge all round.
    pub height: f32,
}

/// How many tiles sit side by side once there is more than one: two, so four
/// streams make a square and the player stays [`theme::MINI_PLAYER_WIDTH`]
/// wide whatever is playing.
///
/// [`theme::MINI_PLAYER_WIDTH`]: crate::theme::MINI_PLAYER_WIDTH
const MINI_COLUMNS: usize = 2;

/// The mini player for `panes` streams: one gets the whole width, and more
/// share it two to a row, so the player grows by rows rather than reaching
/// further across the cards. Tiles are 16:9, with the watch grid's seams
/// between them.
pub fn mini_player(panes: usize) -> MiniLayout {
    let panes = panes.max(1);
    let cols = panes.min(MINI_COLUMNS);
    let rows = panes.div_ceil(cols);
    let tile_w = cell_extent(crate::theme::MINI_PLAYER_WIDTH, cols);
    let tile_h = tile_w / VIDEO_ASPECT;
    let seams = crate::theme::PANE_GAP * (rows - 1) as f32;
    MiniLayout {
        cols,
        rows,
        tile_w,
        tile_h,
        height: tile_h * rows as f32
            + seams
            + crate::theme::MINI_BAR_HEIGHT
            + 2.0 * crate::theme::MINI_PLAYER_INSET,
    }
}

/// How far in from the page's right edge the mini player floats: its gap
/// from the edge, measured from the inside of the browse list's scrollbar,
/// which runs down that edge. At the gap alone the player sat over the foot
/// of the track, and over the thumb whenever the thumb came down that far,
/// so the list could not be dragged by its last stretch.
pub fn mini_player_right() -> f32 {
    crate::theme::SCROLLBAR_WIDTH + crate::theme::GAP
}

/// How much a browse list leaves free at its foot while the mini player is
/// up over it: the player, and a gap either side of it — the one it floats
/// at above the window's edge and one above it — so the last row can be
/// scrolled clear. Nothing with nothing playing. The list's own padding comes
/// on top of this, which more than covers the player's one-pixel border.
pub fn mini_reserve(panes: usize) -> f32 {
    if panes == 0 {
        return 0.0;
    }
    mini_player(panes).height + 2.0 * crate::theme::GAP
}

/// A cell holding 16:9 video with chat *beside* it, so the cell is wider than
/// the video.
const TARGET_CHAT_BESIDE: f32 = 16.0 / 9.0 * 1.25;

/// A cell holding 16:9 video with chat *underneath*, so the cell is taller than
/// the video: 16 wide by roughly 9 + 4.5 tall.
const TARGET_CHAT_BELOW: f32 = 16.0 / 13.5;

/// How badly a cell of this aspect fits either arrangement.
///
/// Two targets rather than one, because both arrangements are legitimate: the
/// question is only which one a given cell is closer to. Measuring against a
/// single ideal made side-by-side panes look wrong on an ordinary monitor,
/// since splitting 16:9 in two columns gives distinctly tall cells - which is
/// fine, they just put their chat underneath.
///
/// Compared in log space so being twice too wide and half too wide are
/// penalised equally.
fn cell_penalty(cell_aspect: f32) -> f32 {
    let beside = (cell_aspect / TARGET_CHAT_BESIDE).ln().abs();
    let below = (cell_aspect / TARGET_CHAT_BELOW).ln().abs();
    beside.min(below)
}

/// Rows and columns for `count` panes in a window of `aspect` (width / height).
pub fn grid_shape(count: usize, aspect: f32) -> (usize, usize) {
    let count = count.max(1);
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        16.0 / 9.0
    };

    let mut best = (1usize, 1usize);
    let mut best_penalty = f32::INFINITY;

    for cols in 1..=count {
        let rows = count.div_ceil(cols);
        // Reject shapes with enough spare cells to drop a whole row or column.
        // Four panes in 2x3 leaves two holes and still passes a naive
        // "no entirely empty row" check, yet 2x2 wastes nothing.
        let cells = rows * cols;
        if cells >= count + rows || cells >= count + cols {
            continue;
        }

        let penalty = cell_penalty(cell_aspect(aspect, rows, cols));
        if penalty < best_penalty {
            best_penalty = penalty;
            best = (rows, cols);
        }
    }
    best
}

/// Whether a cell of this aspect should stack chat under the video rather than
/// beside it.
pub fn cell_is_portrait(cell_aspect: f32) -> bool {
    cell_aspect < crate::theme::PORTRAIT_ASPECT
}

/// The aspect of one cell in the given grid.
pub fn cell_aspect(window_aspect: f32, rows: usize, cols: usize) -> f32 {
    (window_aspect / cols.max(1) as f32) * rows.max(1) as f32
}

/// One cell's width or height along an axis of `total` pixels split `count`
/// ways, with the seams between panes taken out first.
pub fn cell_extent(total: f32, count: usize) -> f32 {
    let count = count.max(1);
    let seams = crate::theme::PANE_GAP * (count - 1) as f32;
    ((total - seams) / count as f32).max(0.0)
}

/// The shape a video box is given before the stream has said what shape it
/// is. Practically every Twitch stream is 16:9, so this is right for the few
/// seconds it is used and for any stream that never reports a size.
pub const VIDEO_ASPECT: f32 = 16.0 / 9.0;

/// How tall the video box is in a stacked cell, for a stream of `aspect`,
/// leaving chat the rest.
///
/// The two arrangements are deliberate opposites: beside the video, chat gets a
/// fixed width and the video takes what is left; below it, the video gets a
/// fixed height and chat takes what is left. A window is tall because you want
/// more chat, not more letterboxing.
///
/// Sized from the *stream's* shape rather than a fixed 16:9, so the box is
/// exactly the video and chat starts where the picture stops. A 16:9 box
/// around a 4:3 stream left a band of black between the two, which read as a
/// gap nobody had asked for. The stream's aspect is safe to size from where
/// its frame size is not: render size follows the pane, so a pane sized from
/// the frame would be a feedback loop, but a broadcast's shape does not change
/// with the window.
///
/// A dragged `share` overrides all of it, as it always did. Otherwise the box
/// is capped at `VIDEO_SHARE_MAX` of the cell, so a vertical stream pillarboxes
/// rather than pushing chat off the bottom.
pub fn stacked_video_height(cell_width: f32, cell_height: f32, aspect: f32, share: f32) -> f32 {
    if share > 0.0 {
        return cell_height
            * share.clamp(crate::theme::VIDEO_SHARE_MIN, crate::theme::VIDEO_SHARE_MAX);
    }
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        VIDEO_ASPECT
    };
    (cell_width / aspect).min(cell_height * crate::theme::VIDEO_SHARE_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDE: f32 = 16.0 / 9.0; // 1.78, an ordinary monitor
    const ULTRAWIDE: f32 = 32.0 / 9.0; // 3.55
    const PORTRAIT: f32 = 9.0 / 16.0; // 0.56, a rotated monitor

    /// The box a 16:9 stream gets in a cell tall enough not to cap it.
    fn video_box_height(cell_width: f32) -> f32 {
        stacked_video_height(cell_width, f32::MAX, VIDEO_ASPECT, 0.0)
    }

    #[test]
    fn one_pane_fills_the_window() {
        assert_eq!(grid_shape(1, WIDE), (1, 1));
        assert_eq!(grid_shape(1, PORTRAIT), (1, 1));
    }

    #[test]
    fn two_panes_split_along_the_long_axis() {
        // Side by side on a normal monitor...
        assert_eq!(grid_shape(2, WIDE), (1, 2));
        // ...and stacked on a rotated one, without either being special-cased.
        assert_eq!(grid_shape(2, PORTRAIT), (2, 1));
    }

    #[test]
    fn four_panes_form_a_square_on_a_normal_monitor() {
        assert_eq!(grid_shape(4, WIDE), (2, 2));
    }

    #[test]
    fn an_ultrawide_prefers_a_single_row() {
        // 32:9 split four ways gives 8:9 cells if stacked 2x2, but a clean
        // 8:9-per-cell row of four is closer to what a pane wants.
        let (rows, cols) = grid_shape(4, ULTRAWIDE);
        assert_eq!(
            rows, 1,
            "expected one row on an ultrawide, got {rows}x{cols}"
        );
        assert_eq!(cols, 4);
    }

    #[test]
    fn a_tall_window_stacks_four_panes_vertically() {
        let (rows, cols) = grid_shape(4, PORTRAIT);
        assert!(rows > cols, "expected a tall grid, got {rows}x{cols}");
    }

    /// A shape that leaves an entire empty row wastes space no matter how good
    /// its cell aspect looks.
    #[test]
    fn never_leaves_a_wholly_empty_row_or_column() {
        for count in 1..=4 {
            for aspect in [PORTRAIT, 1.0, WIDE, ULTRAWIDE] {
                let (rows, cols) = grid_shape(count, aspect);
                assert!(
                    rows * cols >= count,
                    "{count} panes do not fit in {rows}x{cols}"
                );
                assert!(
                    (rows - 1) * cols < count,
                    "{rows}x{cols} leaves an empty row for {count} panes"
                );
            }
        }
    }

    #[test]
    fn degenerate_aspects_fall_back_rather_than_panicking() {
        assert_eq!(grid_shape(2, 0.0), grid_shape(2, 16.0 / 9.0));
        assert_eq!(grid_shape(2, f32::NAN), grid_shape(2, 16.0 / 9.0));
        assert_eq!(grid_shape(0, WIDE), (1, 1));
    }

    /// The video box must never be able to fill a cell it stacks in, or chat
    /// would be squeezed to nothing on some window shape nobody tried. This
    /// holds because a cell only stacks when it is at least
    /// `1 / PORTRAIT_ASPECT` as tall as it is wide, which is taller than
    /// `1 / VIDEO_ASPECT`. Breaking either constant breaks this.
    #[test]
    fn a_stacked_video_always_leaves_room_for_chat() {
        const CELL_WIDTH: f32 = 900.0;
        // The *widest* cell that still stacks: a wider cell is a shorter one,
        // so this is the worst case for chat.
        let cell_aspect = crate::theme::PORTRAIT_ASPECT - 0.001;
        assert!(cell_is_portrait(cell_aspect));
        let cell_height = CELL_WIDTH / cell_aspect;

        let video = video_box_height(CELL_WIDTH);
        let chat = cell_height - video;
        assert!(
            video < cell_height,
            "video {video} filled a {cell_height} cell"
        );
        assert!(
            chat > cell_height * 0.25,
            "chat got {chat} of {cell_height}, which is not a chat pane"
        );
    }

    /// The bug this type exists to prevent, as a number. Two panes in a
    /// window with the rail open: derived from the viewport, the 16:9 box for
    /// a side-by-side cell was 66px taller than the video that fit in it.
    /// Derived from the body, the box is the video - and the grid itself comes
    /// out differently, because a body with the rail taken off it is nearer
    /// square than the window, and two panes stack in a square.
    #[test]
    fn a_body_is_the_viewport_less_the_rail() {
        let viewport = gpui::size(gpui::px(1600.), gpui::px(921.));
        let rail = 236.0;

        let body = Body::of(viewport, rail, 0.0);
        assert_eq!(body.width, 1600.0 - rail);
        assert_eq!(body.height, 921.0);
        assert!(body.aspect() < Body::of(viewport, 0.0, 0.0).aspect());

        // Side by side, which is what the viewport's aspect chose: the box
        // must be sized from the body's half, not the window's.
        let cols = 2;
        let right = video_box_height(body.width / cols as f32);
        let wrong = video_box_height(f32::from(viewport.width) / cols as f32);
        assert!(
            (wrong - right - rail / cols as f32 / VIDEO_ASPECT).abs() < 0.01,
            "the viewport-derived box is {wrong}, the body-derived one {right}"
        );

        // And the shape is the body's to choose, not the window's.
        assert_ne!(
            grid_shape(2, body.aspect()),
            grid_shape(2, Body::of(viewport, 0.0, 0.0).aspect()),
            "the rail should change the grid for this window"
        );
    }

    /// The title bar takes its height off the page the way the rail takes its
    /// width. A grid laid out from the window's height would make every row
    /// taller by its share of the bar, and so run the bottom one off the
    /// window by the whole of it.
    #[test]
    fn a_body_is_also_less_the_title_bar() {
        let viewport = gpui::size(gpui::px(1600.), gpui::px(921.));
        let rail = 236.0;
        let bar = title_bar_height(false);

        let body = Body::of(viewport, rail, bar);
        assert_eq!(body.width, 1600.0 - rail);
        assert_eq!(body.height, 921.0 - crate::theme::TITLE_BAR_HEIGHT);

        // Four panes stack here, so the bar is split between rows: each one
        // sized from the window would be its share of the bar too tall.
        let (rows, _) = grid_shape(4, body.aspect());
        assert!(rows > 1, "four panes in this window should stack");
        let right = cell_extent(body.height, rows);
        let wrong = cell_extent(f32::from(viewport.height), rows);
        assert!(
            (wrong - right - bar / rows as f32).abs() < 0.01,
            "the window-derived row is {wrong}, the body-derived one {right}"
        );
    }

    /// Fullscreen is the picture and nothing else, so the bar goes with the
    /// rest of the window's chrome.
    #[test]
    fn fullscreen_gives_the_title_bar_back() {
        assert_eq!(title_bar_height(true), 0.0);
        assert_eq!(title_bar_height(false), crate::theme::TITLE_BAR_HEIGHT);
    }

    /// The rail goes with the bar: fullscreen hides it whether or not it was
    /// folded, and windowed it is there exactly when it is not folded.
    #[test]
    fn fullscreen_hides_the_rail_too() {
        assert!(rail_shown(false, false));
        assert!(!rail_shown(true, false), "a folded rail is not drawn");
        assert!(!rail_shown(false, true), "fullscreen is the picture alone");
        assert!(!rail_shown(true, true));
    }

    /// A maximised window has no top edge to resize, so the bar is a drag
    /// handle right up to the screen's edge.
    #[test]
    fn a_maximised_bar_drags_from_the_very_top() {
        assert_eq!(drag_top(true), 0.0);
    }

    /// Windowed, the top few pixels are the platform's resize edge, and the
    /// drag area starts under them rather than taking them over.
    #[test]
    fn a_windowed_bar_leaves_the_top_edge_to_resize() {
        let top = drag_top(false);
        assert_eq!(top, crate::theme::TITLE_BAR_RESIZE_BAND);
        assert!(
            top > 0.0,
            "a windowed bar would swallow the top resize edge"
        );
    }

    #[test]
    fn a_body_never_goes_negative() {
        let narrow = Body::of(gpui::size(gpui::px(100.), gpui::px(0.)), 236.0, 0.0);
        assert_eq!(narrow.width, 0.0);
        assert!(narrow.aspect().is_finite());

        // A window shorter than its own title bar, mid-minimise.
        let short = Body::of(gpui::size(gpui::px(800.), gpui::px(30.)), 0.0, 40.0);
        assert_eq!(short.height, 0.0);
        assert!(short.aspect().is_finite());
    }

    /// A 16:9 stream gets the box it always got; anything else gets its own
    /// shape, within the room chat has to keep.
    #[test]
    fn a_stacked_box_is_the_shape_of_its_stream() {
        let (width, height) = (900.0, 1200.0);
        assert_eq!(
            stacked_video_height(width, height, VIDEO_ASPECT, 0.0),
            video_box_height(width)
        );
        let four_three = stacked_video_height(width, height, 4.0 / 3.0, 0.0);
        assert!((four_three - 675.0).abs() < 0.01, "4:3 gave {four_three}");

        let vertical = stacked_video_height(width, height, 9.0 / 16.0, 0.0);
        assert_eq!(vertical, height * crate::theme::VIDEO_SHARE_MAX);

        // A dragged share wins over any shape, clamped like the setting is.
        assert_eq!(stacked_video_height(width, height, 4.0 / 3.0, 0.5), 600.0);
        assert_eq!(
            stacked_video_height(width, height, 4.0 / 3.0, 0.99),
            height * crate::theme::VIDEO_SHARE_MAX
        );

        // Nonsense aspects fall back rather than dividing by zero.
        assert_eq!(
            stacked_video_height(width, height, 0.0, 0.0),
            video_box_height(width)
        );
        assert_eq!(
            stacked_video_height(width, height, f32::NAN, 0.0),
            video_box_height(width)
        );
    }

    /// Why a stacked pane with chat hidden keeps the box a pane with chat
    /// has, rather than giving the picture the whole cell (`watch::chat_or_why`):
    /// with the divider where it is derived, the whole cell has no room to
    /// make a landscape picture any wider.
    ///
    /// A cell stacks only below `PORTRAIT_ASPECT`, and that is narrower than
    /// any landscape stream, so in the whole cell a landscape picture would
    /// still be as wide as the cell and no taller than it — width-limited,
    /// with nothing gained but black above and below. A 16:9 stream gets its
    /// whole width in the derived box already: the box's cap never reaches it
    /// in a cell narrow enough to stack. (A 4:3 one, in a cell just narrow
    /// enough to stack, is capped a few percent short of the cell's width,
    /// with chat or without.) Only the derived share, zero, is checked: a
    /// dragged divider sizes the box as it was dragged, which can be smaller,
    /// and is the user's choice rather than this one. Raising
    /// `PORTRAIT_ASPECT` past a stream's shape fails this, and reopens the
    /// decision.
    #[test]
    fn a_stacked_cell_never_has_room_to_widen_a_landscape_picture() {
        const CELL_WIDTH: f32 = 900.0;
        let threshold = crate::theme::PORTRAIT_ASPECT;
        let stacking = (1..=100).map(|step| threshold * step as f32 / 100.0 - 0.001);
        for cell_aspect in stacking {
            assert!(
                cell_is_portrait(cell_aspect),
                "{cell_aspect} does not stack"
            );
            let cell_height = CELL_WIDTH / cell_aspect;
            for stream in [VIDEO_ASPECT, 4.0 / 3.0] {
                let full_width = CELL_WIDTH / stream;
                assert!(
                    full_width <= cell_height,
                    "a {stream:.2} picture in a {cell_aspect:.3} cell would be taller than \
                     the cell at its full width, so the whole cell would widen it"
                );
            }
            assert_eq!(
                stacked_video_height(CELL_WIDTH, cell_height, VIDEO_ASPECT, 0.0),
                CELL_WIDTH / VIDEO_ASPECT,
                "a 16:9 picture in a {cell_aspect:.3} cell is narrower than the cell in its box"
            );
        }
    }

    #[test]
    fn one_pane_gets_the_whole_mini_player() {
        let mini = mini_player(1);
        assert_eq!((mini.rows, mini.cols), (1, 1));
        assert_eq!(mini.tile_w, crate::theme::MINI_PLAYER_WIDTH);
    }

    /// The player is as wide with four streams as with two: a fixed width is
    /// what keeps it from reaching across the cards, so more streams make it
    /// taller instead.
    #[test]
    fn the_mini_player_grows_by_rows_not_width() {
        let shapes: Vec<(usize, usize)> = (1..=4)
            .map(|panes| {
                let mini = mini_player(panes);
                (mini.rows, mini.cols)
            })
            .collect();
        assert_eq!(shapes, [(1, 1), (1, 2), (2, 2), (2, 2)]);

        for panes in 1..=4 {
            let mini = mini_player(panes);
            let across =
                mini.tile_w * mini.cols as f32 + crate::theme::PANE_GAP * (mini.cols - 1) as f32;
            assert!(
                (across - crate::theme::MINI_PLAYER_WIDTH).abs() < 0.01,
                "{panes} panes spread {across}px across"
            );
        }
        assert!(mini_player(4).height > mini_player(2).height);
    }

    /// Tiles are the shape of the streams in them, so a picture fills its
    /// tile rather than sitting in black bars.
    #[test]
    fn mini_tiles_stay_sixteen_by_nine() {
        for panes in 1..=4 {
            let mini = mini_player(panes);
            assert!(
                (mini.tile_w / mini.tile_h - VIDEO_ASPECT).abs() < 0.001,
                "{panes} panes gave a {}x{} tile",
                mini.tile_w,
                mini.tile_h
            );
        }
    }

    #[test]
    fn nothing_is_reserved_when_nothing_plays() {
        assert_eq!(mini_reserve(0), 0.0);
    }

    /// The last row of a list has to be able to scroll out from under the
    /// player: the room left is at least the player and the gap it floats at.
    #[test]
    fn the_reserve_clears_the_player() {
        for panes in 1..=4 {
            let height = mini_player(panes).height;
            let reserve = mini_reserve(panes);
            assert!(reserve >= height + crate::theme::GAP, "{panes} panes");
        }
    }

    /// The player stands off the page's right edge by more than the list's
    /// scrollbar, so the whole track is the list's to drag, down to its foot,
    /// with the player's usual gap between them.
    #[test]
    fn the_mini_player_leaves_the_scrollbar_clear() {
        let right = mini_player_right();
        assert!(right > crate::theme::SCROLLBAR_WIDTH, "over the track");
        assert_eq!(right - crate::theme::SCROLLBAR_WIDTH, crate::theme::GAP);
    }

    #[test]
    fn cells_report_their_own_aspect() {
        // Half the width, same height: half the aspect.
        assert!((cell_aspect(WIDE, 1, 2) - WIDE / 2.0).abs() < 0.001);
        // Two rows and two columns of a 16:9 window is 16:9 again.
        assert!((cell_aspect(WIDE, 2, 2) - WIDE).abs() < 0.001);
    }

    #[test]
    fn narrow_cells_stack_their_chat() {
        assert!(cell_is_portrait(0.6));
        assert!(!cell_is_portrait(WIDE));
    }
}
