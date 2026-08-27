//! Choosing how to arrange N players in a window.
//!
//! Rather than a lookup table of "2 streams means side by side", the shape is
//! derived: try every column count, and pick the one whose resulting cells come
//! closest to the shape a video pane actually wants. That falls out correctly
//! for an ultrawide, a square window and a vertical monitor without any of them
//! being special-cased.

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

/// The shape of the video box when chat sits below it.
///
/// Fixed rather than taken from the stream: render size follows the pane, so
/// sizing the pane from the frame would be a feedback loop. Practically every
/// Twitch stream is 16:9, and anything else letterboxes inside the box exactly
/// as it did when the box was the whole pane.
const VIDEO_ASPECT: f32 = 16.0 / 9.0;

/// How tall the video is in a cell that stacks, leaving chat the rest.
///
/// The two arrangements are deliberate opposites: beside the video, chat gets a
/// fixed width and the video takes what is left; below it, the video gets a
/// fixed height and chat takes what is left. A window is tall because you want
/// more chat, not more letterboxing.
pub fn video_box_height(cell_width: f32) -> f32 {
    cell_width / VIDEO_ASPECT
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDE: f32 = 16.0 / 9.0; // 1.78, an ordinary monitor
    const ULTRAWIDE: f32 = 32.0 / 9.0; // 3.55
    const PORTRAIT: f32 = 9.0 / 16.0; // 0.56, a rotated monitor

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
