//! Choosing how to arrange N players in a window.
//!
//! Rather than a lookup table of "2 streams means side by side", the shape is
//! derived: try every column count, and pick the one whose resulting cells come
//! closest to the shape a video pane actually wants. That falls out correctly
//! for an ultrawide, a square window and a vertical monitor without any of them
//! being special-cased.

use gpui::{Pixels, Size};

/// The room a page has, which is not the window's once the rail is open.
///
/// One type, built in one place, so that nothing laying out a page can be
/// handed the viewport by mistake. The watch grid was, once: with the rail
/// out, every stacked pane carried a black band under its picture, because
/// its 16:9 box had been derived from a cell wider than the one it was drawn
/// in - by exactly the rail's share of the width. Every consumer now takes a
/// `Body`, and the only way to make one is from the viewport and the rail.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub width: f32,
    pub height: f32,
}

impl Body {
    /// The viewport less `rail_width`, which is zero when the rail is folded.
    pub fn of(viewport: Size<Pixels>, rail_width: f32) -> Self {
        Self {
            width: (f32::from(viewport.width) - rail_width).max(0.0),
            height: f32::from(viewport.height),
        }
    }

    /// Width over height, guarding against a zero-height window during a
    /// minimise.
    pub fn aspect(&self) -> f32 {
        self.width / self.height.max(1.0)
    }
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

        let body = Body::of(viewport, rail);
        assert_eq!(body.width, 1600.0 - rail);
        assert_eq!(body.height, 921.0);
        assert!(body.aspect() < Body::of(viewport, 0.0).aspect());

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
            grid_shape(2, Body::of(viewport, 0.0).aspect()),
            "the rail should change the grid for this window"
        );
    }

    #[test]
    fn a_body_never_goes_negative() {
        let narrow = Body::of(gpui::size(gpui::px(100.), gpui::px(0.)), 236.0);
        assert_eq!(narrow.width, 0.0);
        assert!(narrow.aspect().is_finite());
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
