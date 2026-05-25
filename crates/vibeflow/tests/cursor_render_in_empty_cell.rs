//! Integration test: drive a `Term` to a state with the cursor on an empty
//! cell (the Claude Code TUI input-box bug), and verify the alacritty
//! `display_iter` + cursor contract that the unit tests in
//! `crates/vibeflow/src/render/quad.rs::tests` build on.
//!
//! The unit tests cover the actual quad-emission path (gated `#[ignore]`
//! because they need a real `TextEngine` which requires Mesa software GL).
//! This integration test guards the *premise* of those tests without
//! constructing any wgpu state — so it runs on every CI sweep.

#[test]
fn cursor_position_and_empty_cell_premise_holds() {
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::term::{Config, Term};
    use alacritty_terminal::vte::ansi::Handler;

    let mut term = Term::new(
        Config::default(),
        &TermSize::new(20, 5),
        alacritty_terminal::event::VoidListener,
    );
    // Write 'X' at (0, 0), then move cursor to (line 0, col 10) — an empty cell.
    term.input('X');
    term.goto(0i32, 10);

    let content = term.renderable_content();
    let occupied_cols: Vec<usize> = content
        .display_iter
        .filter_map(|cell| {
            if cell.point.line.0 == 0 && cell.c != ' ' {
                Some(cell.point.column.0)
            } else {
                None
            }
        })
        .collect();
    assert!(
        occupied_cols.contains(&0),
        "expected content at col 0; got {:?}",
        occupied_cols
    );
    assert!(
        !occupied_cols.contains(&10),
        "expected NO non-space content at cursor col 10 (empty cell); got {:?}",
        occupied_cols
    );

    let cursor_state = content.cursor;
    assert_eq!(cursor_state.point.line.0, 0);
    assert_eq!(cursor_state.point.column.0, 10);
}
