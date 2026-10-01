//! Choosing how to arrange N players in a window.
//!
//! Rather than a lookup table of "2 streams means side by side", the shape is
//! derived: try every column count, and pick the one whose resulting cells come
//! closest to the shape a video pane actually wants. That falls out correctly
//! for an ultrawide, a square window and a vertical monitor without any of them
//! being special-cased. The answer is a [`Grid`], made by [`Grid::of`] and
//! nothing else, so the page, the divider drag and the quality a pane is
//! chosen for all read one grid; [`quality_height`] turns a cell of it into
//! the height a rendition is chosen for.
//!
//! It also owns the few numbers more than one part of the window has to agree
//! on: how much of the window the title bar takes, whether the rail is drawn,
//! and where on the bar a drag begins; how the mini player is cut into tiles,
//! how far in from the page's edge it floats, and how much room a browse list
//! leaves at its foot so nothing ends up stuck under the player. And where a
//! pane popped into a window of its own opens. Each is read from here rather
//! than recomputed where it is used.

use gpui::{point, px, size, Bounds, Pixels, Size};

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

/// Where a pane popped into a window of its own opens, on a display whose
/// bounds are `display`, for a stream `aspect` wide to its height, clear of
/// the pop-outs already `open`, and with `last` the bounds the last pop-out
/// closed at this session, if one has.
///
/// It goes back to where the last one was left while the display still
/// holds all of it and no pop-out open covers any of it: somebody who moved
/// a pop-out out of the way meant that place. Otherwise pop-outs stack
/// upwards from the display's bottom-right corner, [`theme::POP_OUT_EDGE`]
/// in from its edges, and a column that has run out of height starts
/// another to its left, so four fit a 1080p display. Each is
/// [`theme::POP_OUT_WIDTH`] wide in the stream's shape — 16:9 for a stream
/// that has not said — never smaller than its window may be resized to, and
/// a vertical stream no taller than half the display. And always inside the
/// display, which Windows asks of a window it opens: one that is not is put
/// somewhere of the platform's choosing (gpui's windows/window.rs:1316-1321).
///
/// Stacked against where the open ones really are, not against a count of
/// them. Pop-outs come in every shape and may have been dragged, resized or
/// sent back to the last place, so a place worked out from a number — the
/// third up the stack is two heights up — lands on one taller than this, or
/// on one moved there: a 4:3 at the bottom and a 16:9 above it overlapped
/// by 80 px. So the places tried are the corner and the spots just above
/// and just left of each open pop-out, lowest of the rightmost column
/// first, and the first that covers none of them wins. A pop-out brought
/// home leaves its place for the next one that way too. Only on a display
/// with no room left anywhere does a new one go to the corner over the
/// others.
///
/// [`theme::POP_OUT_EDGE`]: crate::theme::POP_OUT_EDGE
/// [`theme::POP_OUT_WIDTH`]: crate::theme::POP_OUT_WIDTH
pub fn pop_out_bounds(
    display: Bounds<Pixels>,
    aspect: f32,
    open: &[Bounds<Pixels>],
    last: Option<Bounds<Pixels>>,
) -> Bounds<Pixels> {
    let clear = |bounds: &Bounds<Pixels>| !open.iter().any(|other| overlaps(*bounds, *other));
    if let Some(last) = last.filter(|last| holds(display, *last) && clear(last)) {
        return last;
    }
    let (width, height) = pop_out_size(display, aspect);
    let edge = crate::theme::POP_OUT_EDGE;
    let gap = crate::theme::GAP;
    let left = f32::from(display.origin.x) + edge;
    let top = f32::from(display.origin.y) + edge;
    let corner_x = f32::from(display.right()) - edge - width;
    let corner_y = f32::from(display.bottom()) - edge - height;
    // Rightmost column first, and in each the lowest place first. A hair of
    // slack, so the corner itself is never lost to rounding.
    let mut xs: Vec<f32> = std::iter::once(corner_x)
        .chain(
            open.iter()
                .map(|other| f32::from(other.origin.x) - gap - width),
        )
        .filter(|x| *x >= left - 0.5 && *x <= corner_x + 0.5)
        .collect();
    let mut ys: Vec<f32> = std::iter::once(corner_y)
        .chain(
            open.iter()
                .map(|other| f32::from(other.origin.y) - gap - height),
        )
        .filter(|y| *y >= top - 0.5 && *y <= corner_y + 0.5)
        .collect();
    xs.sort_by(|a, b| b.total_cmp(a));
    ys.sort_by(|a, b| b.total_cmp(a));
    xs.iter()
        .flat_map(|x| ys.iter().map(move |y| (*x, *y)))
        .map(|(x, y)| inside(display, x, y, width, height))
        .find(clear)
        .unwrap_or_else(|| inside(display, corner_x, corner_y, width, height))
}

/// How large a pop-out opens on `display` for a stream `aspect` wide to its
/// height; see [`pop_out_bounds`].
fn pop_out_size(display: Bounds<Pixels>, aspect: f32) -> (f32, f32) {
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        VIDEO_ASPECT
    };
    let mut width = crate::theme::POP_OUT_WIDTH;
    let mut height = width / aspect;
    let tallest = f32::from(display.size.height) / 2.0;
    if height > tallest {
        height = tallest;
        width = height * aspect;
    }
    (
        width.max(crate::theme::POP_OUT_MIN_WIDTH),
        height.max(crate::theme::POP_OUT_MIN_HEIGHT),
    )
}

/// A `width` by `height` box at `x`, `y`, moved as little as it takes to sit
/// inside `display` — against its top-left edge where it is the larger.
fn inside(display: Bounds<Pixels>, x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
    let left = f32::from(display.origin.x);
    let top = f32::from(display.origin.y);
    let width = width.min(f32::from(display.size.width));
    let height = height.min(f32::from(display.size.height));
    let x = x.clamp(left, f32::from(display.right()) - width);
    let y = y.clamp(top, f32::from(display.bottom()) - height);
    Bounds {
        origin: point(px(x), px(y)),
        size: size(px(width), px(height)),
    }
}

/// Whether all of `inner` is on `outer`.
fn holds(outer: Bounds<Pixels>, inner: Bounds<Pixels>) -> bool {
    inner.origin.x >= outer.origin.x
        && inner.origin.y >= outer.origin.y
        && inner.right() <= outer.right()
        && inner.bottom() <= outer.bottom()
}

/// Whether two boxes share any area. Touching edges do not.
fn overlaps(a: Bounds<Pixels>, b: Bounds<Pixels>) -> bool {
    a.origin.x < b.right()
        && b.origin.x < a.right()
        && a.origin.y < b.bottom()
        && b.origin.y < a.bottom()
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

/// The watch grid: how many rows and columns of cells, how large each cell
/// is, and whether a cell stacks its chat under the picture.
///
/// One answer, made in one place ([`Grid::of`]), for everything that has to
/// agree about a pane's size: the watch page that draws the cells, the
/// divider drag measured against them, and the quality each pane is chosen
/// for (`RootView::grid`, `RootView::pane_height_for`). Each of those used
/// to work the shape out again from the body and the number of panes, four
/// copies of one sum that had to be kept in step by hand.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub rows: usize,
    pub cols: usize,
    /// One cell's width and height, with the seams between panes taken out.
    pub cell_width: f32,
    pub cell_height: f32,
    /// A cell's share of the body's height before the seams are taken out:
    /// what its quality is chosen against (`RootView::pane_height_for`,
    /// through [`quality_height`]), as a pane always has been.
    ///
    /// Not `cell_height`, which is shorter by its part of the seams in a
    /// grid of more than one row. A rendition is picked by exact heights
    /// (720p is 1:1 in a pane 720 pixels tall, and one 721 tall already
    /// wants 1080p), so a pixel or two less would move a pane at the edge to
    /// a softer rendition than it plays today, for no saving anybody asked
    /// for.
    pub share_height: f32,
    /// Chat under the picture rather than beside it: a cell's share of the
    /// body is narrower than [`theme::PORTRAIT_ASPECT`].
    ///
    /// Judged on the share before the seams are taken out — the body's
    /// aspect over the columns, times the rows — not on
    /// `cell_width / cell_height`. That share is what [`grid_shape`] scores
    /// each shape by, and what the page has always stacked by, so chat moves
    /// under the picture at the window width it always did. The two differ
    /// only within a few pixels of the threshold, where a seam can tip the
    /// cell as cut to the other side of it.
    ///
    /// [`theme::PORTRAIT_ASPECT`]: crate::theme::PORTRAIT_ASPECT
    pub portrait: bool,
}

impl Grid {
    /// The grid `cells` panes are drawn in, in the room the `body` says the
    /// page has: the shape whose cells come closest to what a pane wants
    /// (see [`grid_shape`]), cut from the cells it is given rather than from
    /// whatever else the caller might count. No cells is the shape of one.
    pub fn of(body: Body, cells: usize) -> Self {
        let (rows, cols) = grid_shape(cells, body.aspect());
        Self {
            rows,
            cols,
            cell_width: cell_extent(body.width, cols),
            cell_height: cell_extent(body.height, rows),
            share_height: body.height / rows as f32,
            portrait: cell_is_portrait(cell_aspect(body.aspect(), rows, cols)),
        }
    }
}

/// The least height a pane's quality is chosen for: under it, every rendition
/// is too big already, and a minimised window's zero would ask for nothing.
const QUALITY_HEIGHT_MIN: f32 = 180.0;

/// The height a pane `physical_px` tall asks its stream's rendition for:
/// whole pixels, no less than [`QUALITY_HEIGHT_MIN`] and no more than the
/// render thread will ever draw (`video::MAX_RENDER_HEIGHT`).
///
/// Physical pixels, not logical ones: on a display at 150% a 720 px pane is
/// 1080 pixels tall, and a rendition chosen for 720 would be stretched to
/// fill it. `RootView::pane_height_for` is the one reader, with the pane's
/// [`Grid::share_height`] multiplied by its window's scale.
pub fn quality_height(physical_px: f32) -> u32 {
    physical_px
        .round()
        .clamp(QUALITY_HEIGHT_MIN, crate::video::MAX_RENDER_HEIGHT as f32) as u32
}

/// Rows and columns for `count` panes in a window of `aspect` (width / height).
/// Read through [`Grid::of`], the one grid, and nowhere else.
fn grid_shape(count: usize, aspect: f32) -> (usize, usize) {
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
fn cell_is_portrait(cell_aspect: f32) -> bool {
    cell_aspect < crate::theme::PORTRAIT_ASPECT
}

/// The aspect of one cell in the given grid.
fn cell_aspect(window_aspect: f32, rows: usize, cols: usize) -> f32 {
    (window_aspect / cols.max(1) as f32) * rows.max(1) as f32
}

/// One cell's width or height along an axis of `total` pixels split `count`
/// ways, with the seams between panes taken out first.
fn cell_extent(total: f32, count: usize) -> f32 {
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

    /// The room a page has in a window of `aspect`, 900 px tall, with no rail
    /// and no bar.
    fn body_of(aspect: f32) -> Body {
        Body::of(gpui::size(px(900.0 * aspect), px(900.0)), 0.0, 0.0)
    }

    /// One pane is the whole page, and so is none: a page with nothing on it
    /// is one empty cell, not a grid divided by zero.
    #[test]
    fn one_cell_gets_the_whole_body() {
        for aspect in [WIDE, ULTRAWIDE, PORTRAIT] {
            let body = body_of(aspect);
            for cells in [0, 1] {
                let grid = Grid::of(body, cells);
                assert_eq!((grid.rows, grid.cols), (1, 1), "{cells} at {aspect}");
                assert_eq!(grid.cell_width, body.width);
                assert_eq!(grid.cell_height, body.height);
            }
        }
    }

    /// The shape is cut from the number of cells handed in, and each cell is
    /// its share of the body less the seams between them.
    #[test]
    fn the_grid_is_cut_from_the_cells_it_is_given() {
        let body = body_of(WIDE);
        let four = Grid::of(body, 4);
        assert_eq!((four.rows, four.cols), (2, 2));
        assert_eq!(four.cell_width, (body.width - crate::theme::PANE_GAP) / 2.0);
        assert_eq!(
            four.cell_height,
            (body.height - crate::theme::PANE_GAP) / 2.0
        );

        let body = body_of(PORTRAIT);
        let two = Grid::of(body, 2);
        assert_eq!((two.rows, two.cols), (2, 1));
        assert_eq!(two.cell_width, body.width);
        assert_eq!(
            two.cell_height,
            (body.height - crate::theme::PANE_GAP) / 2.0
        );
    }

    /// Two panes side by side in a body 1981 px wide and 900 tall, within
    /// a few pixels of where chat moves under the picture.
    fn two_beside_at_the_threshold() -> Body {
        Body::of(gpui::size(px(1981.0), px(900.0)), 0.0, 0.0)
    }

    /// Whether a cell stacks its chat is judged on the cell's share of the
    /// body before the seams come out: the body's aspect over the columns,
    /// times the rows, the measure `grid_shape` scores each shape by. For
    /// every count of panes on every window shape tried here, and at the
    /// threshold, where the cell as cut would answer differently (see
    /// `a_seam_does_not_tip_a_cell_into_stacking`).
    #[test]
    fn a_grids_portrait_flag_is_its_cells_share() {
        let bodies = [WIDE, ULTRAWIDE, PORTRAIT]
            .map(body_of)
            .into_iter()
            .chain([two_beside_at_the_threshold()]);
        for body in bodies {
            for cells in 1..=4 {
                let grid = Grid::of(body, cells);
                let share = body.aspect() / grid.cols as f32 * grid.rows as f32;
                assert_eq!(
                    grid.portrait,
                    share < crate::theme::PORTRAIT_ASPECT,
                    "{cells} cells in {} x {}: a {share:.4} share",
                    body.width,
                    body.height
                );
            }
        }
    }

    /// At the threshold the share decides, not the cell as cut. Each of two
    /// panes beside each other in 1981 x 900 has a share a hair wider than
    /// `PORTRAIT_ASPECT`, and the seam between them makes the cell as cut a
    /// hair narrower. Their chat stays beside, as it did before there was a
    /// `Grid`: the switch to stacking stays at the window width it was.
    #[test]
    fn a_seam_does_not_tip_a_cell_into_stacking() {
        let grid = Grid::of(two_beside_at_the_threshold(), 2);
        assert_eq!((grid.rows, grid.cols), (1, 2));
        let cut = grid.cell_width / grid.cell_height;
        assert!(
            cut < crate::theme::PORTRAIT_ASPECT,
            "the cell as cut is {cut:.4}, under the threshold"
        );
        assert!(!grid.portrait, "but its share is over it, and stays beside");
    }

    /// Never under the floor, never over what the render thread draws, and
    /// whole pixels between.
    #[test]
    fn quality_height_is_clamped_to_what_can_be_rendered() {
        assert_eq!(quality_height(0.0), 180, "a minimised window");
        assert_eq!(quality_height(-40.0), 180);
        assert_eq!(quality_height(120.0), 180);
        assert_eq!(quality_height(1080.0), 1080);
        assert_eq!(quality_height(720.4), 720);
        assert_eq!(quality_height(720.6), 721);
        assert_eq!(
            quality_height(4320.0),
            crate::video::MAX_RENDER_HEIGHT,
            "an 8K display asks for no more than is ever rendered"
        );
    }

    /// A pane is measured in the pixels its picture is drawn in: the same
    /// cell asks half again as much on a display at 150%, so it is not given
    /// a rendition that is then stretched to fill it.
    #[test]
    fn quality_height_counts_physical_pixels() {
        let cell = Grid::of(body_of(WIDE), 1).share_height;
        assert_eq!(quality_height(cell), 900);
        assert_eq!(quality_height(cell * 1.5), 1350);
        let shared = Grid::of(body_of(WIDE), 4).share_height;
        assert_eq!(
            quality_height(shared * 2.0),
            quality_height(shared * 2.0 - 0.4),
            "rounded to whole pixels"
        );
        assert!(quality_height(shared * 2.0) > quality_height(shared));
    }

    /// A pane's quality is chosen against its share of the body's height
    /// with the seams left in, as it always has been: the body's height over
    /// the rows, at any scale. In two rows of a body 1442 px tall each pane
    /// asks for 721, so a stream offering 720p and 1080p plays 1080p there;
    /// the cell as cut, 719.5, would round to 720 and be handed 720p. One
    /// row has no seams to differ by.
    #[test]
    fn a_panes_quality_is_measured_with_the_seams_left_in() {
        let tall = Body::of(gpui::size(px(800.0), px(1442.0)), 0.0, 0.0);
        let two = Grid::of(tall, 2);
        assert_eq!((two.rows, two.cols), (2, 1));
        assert_eq!(quality_height(two.share_height), 721);
        assert_eq!(quality_height(two.cell_height), 720, "the cell as cut");

        let grids = [(tall, 2), (body_of(PORTRAIT), 2), (body_of(WIDE), 4)];
        for (body, cells) in grids {
            let grid = Grid::of(body, cells);
            for scale in [1.0, 1.25, 1.5, 2.0] {
                assert_eq!(
                    quality_height(grid.share_height * scale),
                    quality_height(body.height * scale / grid.rows as f32),
                    "{cells} cells in {} x {} at {scale}",
                    body.width,
                    body.height
                );
            }
        }
        for aspect in [WIDE, ULTRAWIDE, PORTRAIT] {
            let one = Grid::of(body_of(aspect), 1);
            assert_eq!(one.share_height, one.cell_height, "one row at {aspect}");
        }
    }

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(x), px(y)),
            size: size(px(width), px(height)),
        }
    }

    /// A 1080p primary, a 1440p monitor to its right sitting lower, and one
    /// to the left of the primary, at negative coordinates.
    fn displays() -> [Bounds<Pixels>; 3] {
        [
            rect(0., 0., 1920., 1080.),
            rect(1920., 312., 2560., 1440.),
            rect(-1920., 0., 1920., 1080.),
        ]
    }

    /// The shapes a stream comes in: widescreen, the old 4:3, a little
    /// taller than widescreen, vertical, and wider than any monitor.
    const SHAPES: [f32; 5] = [VIDEO_ASPECT, 4.0 / 3.0, 16.0 / 10.0, 9.0 / 16.0, 32.0 / 9.0];

    /// `aspects` popped out one after another on `display`, nothing brought
    /// home in between: each placed clear of those before it.
    fn pop_out_all(display: Bounds<Pixels>, aspects: &[f32]) -> Vec<Bounds<Pixels>> {
        let mut open = Vec::new();
        for aspect in aspects {
            let bounds = pop_out_bounds(display, *aspect, &open, None);
            open.push(bounds);
        }
        open
    }

    /// No two of `stack` share any area.
    fn assert_apart(stack: &[Bounds<Pixels>]) {
        for (i, a) in stack.iter().enumerate() {
            for b in &stack[i + 1..] {
                assert!(!overlaps(*a, *b), "{a:?} covers {b:?}");
            }
        }
    }

    /// Windows puts a window whose bounds are off its display wherever it
    /// likes, so every pop-out, the first and the tenth, opens on the display.
    #[test]
    fn a_pop_out_sits_inside_the_display() {
        for display in displays() {
            for aspect in SHAPES {
                for bounds in pop_out_all(display, &[aspect; 10]) {
                    assert!(
                        holds(display, bounds),
                        "{aspect} left {display:?}: {bounds:?}"
                    );
                }
            }
        }
    }

    /// The stack starts at the bottom-right, clear of a taskbar, goes up,
    /// and no two pop-outs in it cover each other, whatever shapes they are
    /// and in whichever order they come: a 4:3 under a 16:9 overlapped it by
    /// 80 px when each place was worked out from its own height alone.
    #[test]
    fn pop_outs_stack_without_overlapping() {
        let display = displays()[0];
        let stack = pop_out_all(display, &[VIDEO_ASPECT; 4]);
        let edge = crate::theme::POP_OUT_EDGE;
        assert_eq!(f32::from(stack[0].right()), 1920.0 - edge);
        assert_eq!(f32::from(stack[0].bottom()), 1080.0 - edge);
        assert!(
            stack[1].origin.y < stack[0].origin.y,
            "the second goes up the stack"
        );
        assert_apart(&stack);
        for display in displays() {
            for a in SHAPES {
                for b in SHAPES {
                    for c in SHAPES {
                        for d in SHAPES {
                            let stack = pop_out_all(display, &[a, b, c, d]);
                            assert_apart(&stack);
                            for bounds in &stack {
                                assert!(holds(display, *bounds), "{bounds:?}");
                            }
                        }
                    }
                }
            }
        }
    }

    /// A full stack of four on an ordinary monitor: the column runs out of
    /// height at three, and the fourth starts a column to the left.
    #[test]
    fn four_pop_outs_fit_a_1080p_display() {
        let display = displays()[0];
        let stack = pop_out_all(display, &[VIDEO_ASPECT; 4]);
        for bounds in &stack {
            assert!(holds(display, *bounds), "{bounds:?}");
        }
        assert!(
            stack[3].right() <= stack[0].origin.x,
            "the fourth should start a column left of the first"
        );
        assert_eq!(stack[3].bottom(), stack[0].bottom());
    }

    /// The window is the stream's shape at the width it opens at, 16:9 until
    /// the stream says, and a vertical stream is held to half the display.
    #[test]
    fn a_pop_out_keeps_the_streams_shape() {
        let display = displays()[0];
        let shape = |aspect: f32| {
            let bounds = pop_out_bounds(display, aspect, &[], None);
            (f32::from(bounds.size.width), f32::from(bounds.size.height))
        };
        let (width, height) = shape(VIDEO_ASPECT);
        assert_eq!(width, crate::theme::POP_OUT_WIDTH);
        assert!((width / height - VIDEO_ASPECT).abs() < 0.01);
        let (width, height) = shape(4.0 / 3.0);
        assert!((width / height - 4.0 / 3.0).abs() < 0.01);
        for unknown in [0.0, f32::NAN, -1.0] {
            assert_eq!(shape(unknown), shape(VIDEO_ASPECT), "aspect {unknown}");
        }
        let (width, height) = shape(9.0 / 16.0);
        assert_eq!(height, 540.0, "a vertical stream at half the display");
        assert!((width / height - 9.0 / 16.0).abs() < 0.01);
        let (width, height) = shape(32.0 / 9.0);
        assert!(width >= crate::theme::POP_OUT_MIN_WIDTH);
        assert!(
            height >= crate::theme::POP_OUT_MIN_HEIGHT,
            "never under its least size"
        );
    }

    /// A pop-out goes back to where the last one was left, while the display
    /// still holds all of it and no pop-out open covers it.
    #[test]
    fn the_last_place_is_reused_while_a_display_holds_it() {
        let display = displays()[0];
        let moved = rect(200., 150., 640., 360.);
        assert_eq!(
            pop_out_bounds(display, VIDEO_ASPECT, &[], Some(moved)),
            moved
        );
        let half_off = rect(1700., 150., 640., 360.);
        assert_eq!(
            pop_out_bounds(display, VIDEO_ASPECT, &[], Some(half_off)),
            pop_out_bounds(display, VIDEO_ASPECT, &[], None),
            "a place the display no longer holds is not reused"
        );
        let there = rect(400., 300., 480., 270.);
        assert_eq!(
            pop_out_bounds(display, VIDEO_ASPECT, &[there], Some(moved)),
            pop_out_bounds(display, VIDEO_ASPECT, &[there], None),
            "a place a pop-out open covers is not reused"
        );
    }

    /// The last place can be one up the stack. Two out, both brought home,
    /// the second last: the next goes where the second was, and the one
    /// after it must not open over it — where the second went when the
    /// first was in the corner.
    #[test]
    fn a_pop_out_never_opens_over_one_sent_back_up_the_stack() {
        let display = displays()[0];
        let pair = pop_out_all(display, &[VIDEO_ASPECT; 2]);
        let third = pop_out_bounds(display, VIDEO_ASPECT, &[], Some(pair[1]));
        assert_eq!(third, pair[1]);
        let fourth = pop_out_bounds(display, VIDEO_ASPECT, &[third], Some(pair[1]));
        assert!(!overlaps(third, fourth), "{fourth:?} covers {third:?}");
        assert_eq!(fourth, pair[0], "the corner is free, so it goes there");
    }

    /// A pop-out brought home leaves its place for the next one, and one
    /// dragged away leaves its place too.
    #[test]
    fn a_place_left_is_taken_again() {
        let display = displays()[0];
        let stack = pop_out_all(display, &[VIDEO_ASPECT; 3]);
        assert_eq!(
            pop_out_bounds(display, VIDEO_ASPECT, &[stack[1], stack[2]], None),
            stack[0],
            "the corner, once the first came home"
        );
        assert_eq!(
            pop_out_bounds(display, VIDEO_ASPECT, &[stack[0], stack[2]], None),
            stack[1],
            "the gap in the middle"
        );
        let dragged = rect(100., 100., 480., 270.);
        assert_eq!(
            pop_out_bounds(display, VIDEO_ASPECT, &[dragged], None),
            stack[0],
            "the corner, once the one there moved"
        );
    }
}
