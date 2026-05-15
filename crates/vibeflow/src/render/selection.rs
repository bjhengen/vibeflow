//! Mouse-driven cell selection state machine. Pure logic — no GPU, no winit,
//! no PTY dependency.
//!
//! Each `PtySession` owns one `SelectionTracker`. Mouse events from
//! `window.rs` are translated to grid `Point`s before reaching the tracker;
//! the tracker emits `Selection` updates that the renderer reads each frame.
//!
//! Selection lives in *grid* coordinates (alacritty `Point` — line, column).
//! The tracker doesn't care about pixels.
//!
//! State transitions:
//!
//! ```text
//! Idle ── mouse_down ──► Dragging
//!  ▲                       │
//!  │                       │ mouse_drag (updates `current`, snaps to mode)
//!  │                       │
//!  │                       ▼
//!  └─ mouse_up (and ───── Selected
//!     start==end &       (final, visible)
//!     count==1) or
//!     clear()
//! ```
//!
//! Word/Line modes are entered by raising the click counter via successive
//! mouse_down calls within 500ms and 1 cell of each other. Click counter
//! resets after 500ms gap or movement > 1 cell.

use std::time::{Duration, Instant};

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::Term;

const MULTI_CLICK_WINDOW: Duration = Duration::from_millis(500);

/// Final or in-progress selection range. `start` and `end` are ordered such
/// that `start` is the visually-earlier point in reading order
/// (`start.line < end.line`, or same line with `start.column <= end.column`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub start: Point,
    pub end: Point,
    pub mode: SelectionMode,
}

/// What kind of region the selection covers. Affects how `mouse_drag` snaps
/// the endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMode {
    /// Cell-by-cell (single-click drag). Endpoints follow the mouse exactly.
    Cell,
    /// Word-bounded (double-click). Endpoints snap to word boundaries via
    /// `Term::semantic_search_left/right`.
    Word,
    /// Whole-line (triple-click). `start.column = 0`, `end.column = cols-1`.
    Line,
    /// Rectangular column selection (Alt+drag). Each row is clipped to the
    /// [min_col, max_col] span; rows are joined with `\n` on copy.
    Block,
}

#[derive(Debug, Clone, Copy)]
struct ClickHistory {
    last_at: Instant,
    last_point: Point,
    count: u8,
}

/// Per-session state tracker. Owns the in-flight drag and the current
/// finalized selection (if any).
pub struct SelectionTracker {
    selection: Option<Selection>,
    drag_anchor: Option<Point>,
    click: Option<ClickHistory>,
}

impl Default for SelectionTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl SelectionTracker {
    #[must_use]
    pub fn new() -> Self {
        Self {
            selection: None,
            drag_anchor: None,
            click: None,
        }
    }

    /// Returns the current selection (in-flight or finalized) or `None`.
    #[must_use]
    pub fn current(&self) -> Option<&Selection> {
        self.selection.as_ref()
    }

    /// Returns true while the mouse is held and a drag is being built.
    /// Used by `window.rs` to decide whether `CursorMoved` should call
    /// `mouse_drag` or be ignored.
    #[must_use]
    pub fn is_dragging(&self) -> bool {
        self.drag_anchor.is_some()
    }

    /// Mouse-down handler. Internal API takes an `Instant` for testability;
    /// in production callers pass `Instant::now()`.
    pub fn mouse_down(
        &mut self,
        point: Point,
        shift_held: bool,
        alt: bool,
        term: &Term<VoidListener>,
        now: Instant,
    ) {
        // Click counter — distinguishes single / double / triple clicks.
        let count = self.bump_click(point, now);

        if shift_held && self.selection.is_some() {
            // Shift-extend: keep the existing start, move end to the new
            // point. Don't change mode. Don't change drag_anchor.
            if let Some(sel) = &mut self.selection {
                let (start, end) = order(sel.start, point);
                sel.start = start;
                sel.end = end;
            }
            self.drag_anchor = Some(self.selection.unwrap().start);
            return;
        }

        // Fresh selection. Alt forces sticky Block mode; otherwise mode
        // follows the click count (single/double/triple = Cell/Word/Line).
        let mode = if alt && !shift_held {
            SelectionMode::Block
        } else {
            match count {
                1 => SelectionMode::Cell,
                2 => SelectionMode::Word,
                3 => SelectionMode::Line,
                _ => SelectionMode::Cell, // 4th click wraps back to single
            }
        };
        self.drag_anchor = Some(point);
        self.selection = Some(Selection {
            start: point,
            end: point,
            mode,
        });
        // For Word/Line, immediately snap so a click without drag selects
        // the word/line under the cursor. Block's snap arm is a no-op.
        self.snap_to_mode(term);
    }

    /// Mouse-drag handler. Updates the current endpoint based on `point`
    /// and re-snaps according to the active mode.
    pub fn mouse_drag(&mut self, point: Point, term: &Term<VoidListener>) {
        let Some(anchor) = self.drag_anchor else {
            return;
        };
        let Some(sel) = self.selection.as_mut() else {
            return;
        };
        let (start, end) = order(anchor, point);
        sel.start = start;
        sel.end = end;
        // Re-snap to keep word/line bounds consistent with the new endpoints.
        self.snap_to_mode(term);
    }

    /// Mouse-up handler. Finalizes the selection. If the user clicked
    /// without dragging (single-click, start==end), clears the selection.
    pub fn mouse_up(&mut self) {
        self.drag_anchor = None;
        let click_count = self.click.map(|c| c.count).unwrap_or(0);
        if let Some(sel) = self.selection {
            if sel.start == sel.end && click_count == 1 {
                self.selection = None;
            }
        }
    }

    /// Force-clear the selection. Called by `window.rs` on window resize,
    /// any input keystroke, etc.
    pub fn clear(&mut self) {
        self.selection = None;
        self.drag_anchor = None;
        // Click counter persists — clearing for "input typed" must not reset
        // the double-click window. The 500ms / 1-cell rule still gates it.
    }

    /// Select the entire grid buffer including all available scrollback. The
    /// start line uses `-history_size as i32`; the end is the bottom-right
    /// cell of the viewport. `text()` and `cells()` already iterate the full
    /// range without filtering scrollback, so a subsequent copy retrieves the
    /// invisible history. Selection rectangles for scrollback rows are still
    /// filtered out of rendering by `build_selection_rects`.
    pub fn select_all(&mut self, term: &Term<VoidListener>) {
        let cols = term.columns();
        let lines = term.screen_lines();
        let history = term.history_size();
        let start = Point::new(Line(-(history as i32)), Column(0));
        let end = Point::new(Line(lines as i32 - 1), Column(cols.saturating_sub(1)));
        self.selection = Some(Selection {
            start,
            end,
            mode: SelectionMode::Cell,
        });
        self.drag_anchor = None;
    }

    /// Yield each cell in the current selection in row-major order.
    /// Returns an empty iterator if no selection.
    pub fn cells<'a>(
        &'a self,
        term: &'a Term<VoidListener>,
    ) -> Box<dyn Iterator<Item = Point> + 'a> {
        let Some(sel) = self.selection else {
            return Box::new(std::iter::empty());
        };
        match sel.mode {
            SelectionMode::Block => Box::new(cells_in_range_block(sel.start, sel.end)),
            _ => Box::new(cells_in_range(sel.start, sel.end, term.columns())),
        }
    }

    /// Materialize the selection as a `String`. Returns `None` if there's
    /// no selection. Used by `Ctrl+Shift+C`.
    pub fn text(&self, term: &Term<VoidListener>) -> Option<String> {
        let sel = self.selection?;
        let mut out = String::new();
        let cells_iter: Box<dyn Iterator<Item = Point>> = match sel.mode {
            SelectionMode::Block => Box::new(cells_in_range_block(sel.start, sel.end)),
            _ => Box::new(cells_in_range(sel.start, sel.end, term.columns())),
        };
        let mut current_line: Option<Line> = None;
        for p in cells_iter {
            if let Some(l) = current_line {
                if l != p.line {
                    out.push('\n');
                }
            }
            current_line = Some(p.line);
            // alacritty `Term::grid()[p].c` gives the character at the cell.
            // For empty cells it's typically `' '`.
            let cell = &term.grid()[p];
            out.push(cell.c);
        }
        Some(out)
    }

    fn bump_click(&mut self, point: Point, now: Instant) -> u8 {
        let count = match self.click {
            Some(prev)
                if now.duration_since(prev.last_at) <= MULTI_CLICK_WINDOW
                    && cell_distance(prev.last_point, point) <= 1 =>
            {
                prev.count.wrapping_add(1)
            }
            _ => 1,
        };
        self.click = Some(ClickHistory {
            last_at: now,
            last_point: point,
            count,
        });
        count
    }

    fn snap_to_mode(&mut self, term: &Term<VoidListener>) {
        let Some(sel) = self.selection.as_mut() else {
            return;
        };
        match sel.mode {
            SelectionMode::Cell => {} // no snap
            SelectionMode::Word => {
                sel.start = term.semantic_search_left(sel.start);
                sel.end = term.semantic_search_right(sel.end);
            }
            SelectionMode::Line => {
                sel.start.column = Column(0);
                let last_col = term.columns().saturating_sub(1);
                sel.end.column = Column(last_col);
            }
            SelectionMode::Block => {} // no snap — rectangular region follows the mouse
        }
    }
}

/// Order two points so the result is `(earlier, later)` in reading order.
fn order(a: Point, b: Point) -> (Point, Point) {
    if (a.line, a.column.0) <= (b.line, b.column.0) {
        (a, b)
    } else {
        (b, a)
    }
}

/// Manhattan-ish distance in cells. Used by the click-counter to decide if
/// two clicks are "close enough" to count as a double-click.
fn cell_distance(a: Point, b: Point) -> u32 {
    let line_diff = (a.line.0 - b.line.0).unsigned_abs();
    let col_diff = a.column.0.abs_diff(b.column.0) as u32;
    line_diff + col_diff
}

/// Stage 13: rectangular cell iteration for block-selection mode.
/// Normalizes start/end so any pair of corners works. Row-major order.
fn cells_in_range_block(start: Point, end: Point) -> impl Iterator<Item = Point> {
    let (top, bottom) = if start.line.0 <= end.line.0 {
        (start.line.0, end.line.0)
    } else {
        (end.line.0, start.line.0)
    };
    let (left, right) = if start.column.0 <= end.column.0 {
        (start.column.0, end.column.0)
    } else {
        (end.column.0, start.column.0)
    };
    (top..=bottom)
        .flat_map(move |line| (left..=right).map(move |col| Point::new(Line(line), Column(col))))
}

/// Iterate the cells covered by a selection from `start` to `end` (inclusive)
/// in linear text-flow order: end of `start.line` → all of `start.line+1` →
/// ... → start of `end.line`.
fn cells_in_range(start: Point, end: Point, cols: usize) -> impl Iterator<Item = Point> {
    // Single-line case
    if start.line == end.line {
        let line = start.line;
        let s = start.column.0;
        let e = end.column.0;
        return Box::new((s..=e).map(move |c| Point::new(line, Column(c))))
            as Box<dyn Iterator<Item = Point>>;
    }
    // Multi-line case: a chain of three iterators (head, middle lines, tail).
    let last_col = cols.saturating_sub(1);
    let head = (start.column.0..=last_col).map(move |c| Point::new(start.line, Column(c)));
    let middle_lines: Vec<Point> = ((start.line.0 + 1)..end.line.0)
        .flat_map(move |l| (0..=last_col).map(move |c| Point::new(Line(l), Column(c))))
        .collect();
    let tail = (0..=end.column.0).map(move |c| Point::new(end.line, Column(c)));
    Box::new(head.chain(middle_lines).chain(tail)) as Box<dyn Iterator<Item = Point>>
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::term::{Config as TermConfig, Term};
    use std::time::Instant;

    fn make_term(cols: usize, lines: usize) -> Term<VoidListener> {
        let size = TermSize::new(cols, lines);
        Term::new(TermConfig::default(), &size, VoidListener)
    }

    fn pt(line: i32, col: usize) -> Point {
        Point::new(Line(line), Column(col))
    }

    // Construction

    #[test]
    fn new_tracker_has_no_selection() {
        let t = SelectionTracker::new();
        assert!(t.current().is_none());
        assert!(!t.is_dragging());
    }

    // Single click (no drag)

    #[test]
    fn single_click_no_drag_clears_on_mouse_up() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        let now = Instant::now();
        t.mouse_down(pt(5, 10), false, false, &term, now);
        // Drag-anchor set; selection points to the single cell.
        assert!(t.is_dragging());
        assert_eq!(t.current().map(|s| s.start), Some(pt(5, 10)));
        t.mouse_up();
        // Click without drag → cleared.
        assert!(t.current().is_none());
        assert!(!t.is_dragging());
    }

    // Drag

    #[test]
    fn mouse_down_then_drag_then_up_finalizes_selection() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        let now = Instant::now();
        t.mouse_down(pt(2, 3), false, false, &term, now);
        t.mouse_drag(pt(2, 8), &term);
        t.mouse_up();
        let s = t.current().expect("selection finalized");
        assert_eq!(s.start, pt(2, 3));
        assert_eq!(s.end, pt(2, 8));
        assert_eq!(s.mode, SelectionMode::Cell);
    }

    #[test]
    fn drag_endpoints_are_ordered_smaller_first() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        let now = Instant::now();
        // Drag from (5, 20) to (2, 5) — backward.
        t.mouse_down(pt(5, 20), false, false, &term, now);
        t.mouse_drag(pt(2, 5), &term);
        let s = t.current().unwrap();
        assert_eq!(s.start, pt(2, 5));
        assert_eq!(s.end, pt(5, 20));
    }

    #[test]
    fn mouse_drag_without_prior_mouse_down_is_noop() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        t.mouse_drag(pt(0, 0), &term);
        assert!(t.current().is_none());
    }

    // Multi-click (word / line)

    #[test]
    fn double_click_within_window_extends_to_word_mode() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        let t0 = Instant::now();
        t.mouse_down(pt(0, 5), false, false, &term, t0);
        t.mouse_up();
        // Within 500ms and same point → count = 2 → Word mode.
        let t1 = t0 + Duration::from_millis(100);
        t.mouse_down(pt(0, 5), false, false, &term, t1);
        let s = t.current().unwrap();
        assert_eq!(s.mode, SelectionMode::Word);
    }

    #[test]
    fn triple_click_within_window_extends_to_line_mode() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        let t0 = Instant::now();
        t.mouse_down(pt(0, 5), false, false, &term, t0);
        t.mouse_up();
        t.mouse_down(
            pt(0, 5),
            false,
            false,
            &term,
            t0 + Duration::from_millis(50),
        );
        t.mouse_up();
        t.mouse_down(
            pt(0, 5),
            false,
            false,
            &term,
            t0 + Duration::from_millis(100),
        );
        let s = t.current().unwrap();
        assert_eq!(s.mode, SelectionMode::Line);
    }

    #[test]
    fn click_counter_resets_after_500ms_gap() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        let t0 = Instant::now();
        t.mouse_down(pt(0, 5), false, false, &term, t0);
        t.mouse_up();
        // 600ms later — gap exceeds window.
        let t1 = t0 + Duration::from_millis(600);
        t.mouse_down(pt(0, 5), false, false, &term, t1);
        let s = t.current().unwrap();
        // Counter reset → count=1 → Cell mode.
        assert_eq!(s.mode, SelectionMode::Cell);
    }

    #[test]
    fn click_counter_resets_when_point_moves() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        let t0 = Instant::now();
        t.mouse_down(pt(0, 5), false, false, &term, t0);
        t.mouse_up();
        // Same time, different cell (>1 away).
        t.mouse_down(
            pt(0, 50),
            false,
            false,
            &term,
            t0 + Duration::from_millis(50),
        );
        let s = t.current().unwrap();
        assert_eq!(s.mode, SelectionMode::Cell);
    }

    // Shift-extend

    #[test]
    fn shift_click_extends_existing_selection() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        let now = Instant::now();
        t.mouse_down(pt(2, 5), false, false, &term, now);
        t.mouse_drag(pt(2, 10), &term);
        t.mouse_up();
        // Shift-click further out — should extend the end.
        t.mouse_down(
            pt(2, 20),
            true,
            false,
            &term,
            now + Duration::from_millis(50),
        );
        let s = t.current().unwrap();
        assert_eq!(s.start, pt(2, 5));
        assert_eq!(s.end, pt(2, 20));
    }

    // Clear

    #[test]
    fn clear_drops_selection_and_drag_anchor() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        let now = Instant::now();
        t.mouse_down(pt(0, 5), false, false, &term, now);
        t.mouse_drag(pt(0, 10), &term);
        // Mid-drag clear (e.g. user typed something).
        t.clear();
        assert!(t.current().is_none());
        assert!(!t.is_dragging());
    }

    // Cells iteration

    #[test]
    fn single_line_selection_yields_left_to_right_cells() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        let now = Instant::now();
        t.mouse_down(pt(0, 3), false, false, &term, now);
        t.mouse_drag(pt(0, 6), &term);
        t.mouse_up();
        let cells: Vec<Point> = t.cells(&term).collect();
        assert_eq!(cells, vec![pt(0, 3), pt(0, 4), pt(0, 5), pt(0, 6)]);
    }

    #[test]
    fn multi_line_selection_wraps_around_end_of_line() {
        let mut t = SelectionTracker::new();
        let term = make_term(5, 24); // 5 cols
        let now = Instant::now();
        t.mouse_down(pt(0, 3), false, false, &term, now);
        t.mouse_drag(pt(2, 1), &term);
        t.mouse_up();
        let cells: Vec<Point> = t.cells(&term).collect();
        assert_eq!(
            cells,
            vec![
                pt(0, 3),
                pt(0, 4), // tail of line 0
                pt(1, 0),
                pt(1, 1),
                pt(1, 2),
                pt(1, 3),
                pt(1, 4), // all of line 1
                pt(2, 0),
                pt(2, 1), // head of line 2
            ]
        );
    }

    #[test]
    fn empty_tracker_yields_no_cells() {
        let t = SelectionTracker::new();
        let term = make_term(80, 24);
        assert_eq!(t.cells(&term).count(), 0);
    }

    // ---- select_all (Stage 10) -------------------------------------------

    #[test]
    fn select_all_covers_visible_grid_when_no_history() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        t.select_all(&term);
        let s = t.current().expect("selection set after select_all");
        // start at (line: -history_size, col: 0); end at (line: 23, col: 79) for an 80x24 grid.
        assert_eq!(s.start.column.0, 0);
        assert_eq!(s.end.line.0, 23);
        assert_eq!(s.end.column.0, 79);
        // Default TermConfig has scrolling_history = 10000 → start.line = -10000.
        // Don't pin the exact value (config-dependent); assert it's <= 0.
        assert!(s.start.line.0 <= 0, "start.line should reach into history");
    }

    #[test]
    fn select_all_uses_cell_mode() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        t.select_all(&term);
        let s = t.current().expect("selection set");
        assert_eq!(s.mode, SelectionMode::Cell);
    }

    #[test]
    fn select_all_replaces_existing_selection() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        // Establish a small selection first.
        let now = Instant::now();
        t.mouse_down(pt(2, 3), false, false, &term, now);
        t.mouse_drag(pt(2, 8), &term);
        t.mouse_up();
        assert!(t.current().is_some());
        // Now select_all replaces it.
        t.select_all(&term);
        let s = t.current().expect("replaced");
        assert_eq!(s.end.line.0, 23);
        assert_eq!(s.end.column.0, 79);
    }

    // ---- Block (column) selection (Stage 13, Task 7) ----------------------

    #[test]
    fn cells_in_range_block_yields_rectangle() {
        // pt(1, 2) → pt(3, 4): rows 1..=3, cols 2..=4 → 3×3 = 9 cells
        let cells: Vec<_> = cells_in_range_block(pt(1, 2), pt(3, 4)).collect();
        assert_eq!(cells.len(), 9);
        assert_eq!(cells[0], pt(1, 2));
        assert_eq!(cells[2], pt(1, 4));
        assert_eq!(cells[3], pt(2, 2));
        assert_eq!(cells[8], pt(3, 4));
    }

    #[test]
    fn cells_in_range_block_handles_reverse_order() {
        // Dragging from bottom-right to top-left should normalize.
        let cells: Vec<_> = cells_in_range_block(pt(3, 4), pt(1, 2)).collect();
        assert_eq!(cells.len(), 9);
        assert_eq!(cells[0], pt(1, 2));
    }

    #[test]
    fn cells_in_range_block_single_cell() {
        let cells: Vec<_> = cells_in_range_block(pt(5, 5), pt(5, 5)).collect();
        assert_eq!(cells, vec![pt(5, 5)]);
    }

    #[test]
    fn alt_drag_sets_block_mode() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        let now = Instant::now();
        t.mouse_down(pt(2, 3), false, true, &term, now); // shift=false, alt=true
        t.mouse_drag(pt(5, 7), &term);
        t.mouse_up();
        let sel = t.current().expect("selection");
        assert_eq!(sel.mode, SelectionMode::Block);
    }

    #[test]
    fn alt_drag_block_yields_block_shaped_cells() {
        let mut t = SelectionTracker::new();
        let term = make_term(80, 24);
        let now = Instant::now();
        t.mouse_down(pt(0, 0), false, true, &term, now);
        t.mouse_drag(pt(1, 2), &term); // 2 rows × 3 cols = 6 cells
        t.mouse_up();
        let collected: Vec<_> = t.cells(&term).collect();
        assert_eq!(collected.len(), 6);
    }
}
