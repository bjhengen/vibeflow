//! Stage 10 integration tests against a real PTY. Verify menu open/dispatch
//! flow end-to-end. No display required — these run headless.

use std::time::{Duration, Instant};

use vibeflow::app::App;
use vibeflow::render::context_menu;
use vibeflow::session::SessionEvent;

/// Spawn an `App` with one tab running `bash`, mirroring the pattern in
/// `pty_integration.rs`: `App::new()` + `new_tab`.
fn spawn_app_with_one_tab() -> App {
    let mut app = App::new();
    app.new_tab(&["bash"]).expect("spawn bash");
    app
}

/// Poll the app for up to `deadline`, driving `poll_all` on each tick.
/// Matches the driver loop pattern from `pty_integration.rs`.
fn drive_until(app: &mut App, deadline: Instant) {
    while Instant::now() < deadline {
        let _events = app.poll_all(Instant::now());
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Drive until `SessionEvent::Died` is observed on any tab, or timeout.
/// Returns `true` if a `Died` event was seen.
fn drive_until_died(app: &mut App, max: Duration) -> bool {
    let deadline = Instant::now() + max;
    while Instant::now() < deadline {
        for (_, ev) in app.poll_all(Instant::now()) {
            if matches!(ev, SessionEvent::Died) {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

// ---- select_all tests -------------------------------------------------------

#[test]
fn select_all_then_text_returns_full_buffer() {
    let mut app = spawn_app_with_one_tab();
    // Wait for the shell to print at least one prompt.
    drive_until(&mut app, Instant::now() + Duration::from_millis(300));
    // `select_all_active` resolves the split-borrow between `selection`
    // (needs `&mut`) and `term()` (needs `&`) on the same `PtySession`
    // internally, so the integration test stays safe and straightforward.
    app.select_all_active();
    let active = app.active();
    // Both borrows below are `&self`, so they're fine on the same session.
    let s = &app.tabs()[active];
    let text = s.selection.text(s.term()).unwrap_or_default();
    assert!(
        !text.is_empty(),
        "select_all → text should produce shell output"
    );
}

// ---- grid_menu tests --------------------------------------------------------

#[test]
fn grid_menu_disables_copy_when_no_selection() {
    let app = spawn_app_with_one_tab();
    let active = app.active();
    let has_sel = app.tabs()[active].selection.current().is_some();
    let items = context_menu::grid_menu(has_sel);
    let copy = items.iter().find(|i| i.label == "Copy").unwrap();
    assert!(
        !copy.enabled,
        "Copy must be disabled when there is no selection"
    );
}

// ---- tab_menu / tab-death tests ---------------------------------------------

#[test]
fn tab_menu_for_dead_tab_includes_restart() {
    // Spawn a tab that exits immediately.
    let mut app = App::new();
    app.new_tab(&["true"]).expect("spawn true");
    // Drive until the child exits and the session marks dead.
    let died = drive_until_died(&mut app, Duration::from_secs(5));
    assert!(died, "expected SessionEvent::Died for `true`");
    let is_dead = !app.tabs()[0].is_alive();
    assert!(is_dead, "expected `true` child to have exited");
    let items = context_menu::tab_menu(0, is_dead, app.tabs().len(), &[]);
    assert!(
        items.iter().any(|i| i.label == "Restart Tab"),
        "tab_menu(is_dead=true) must include a 'Restart Tab' item"
    );
}
