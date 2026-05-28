//! v0.1.3 integration tests for the confirm-on-close dialog. State-machine
//! only — no real winit window, so we exercise via App's public surface
//! and the `render::confirm_close` state-struct directly.

use vibeflow::app::App;
use vibeflow::config::Ui;
use vibeflow::render::confirm_close::{ConfirmCloseState, FocusedButton};
use std::time::{Duration, Instant};

/// Poll-with-deadline helper for PTY-driven integration tests. Mirrors the
/// `wait_until` in `session::session::tests` — parallel `cargo test` load
/// makes fixed `thread::sleep` windows unreliable for bash-fork-subprocess
/// scenarios.
fn wait_until<F: FnMut() -> bool>(timeout: Duration, mut cond: F) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    cond()
}

fn spawn_idle(app: &mut App, n: usize) {
    for _ in 0..n {
        app.new_tab(&["/bin/bash"]).expect("spawn bash");
    }
    std::thread::sleep(Duration::from_millis(300));
    for tab in app.tabs_mut().iter_mut() {
        let _ = tab.poll(Instant::now());
    }
}

#[test]
fn single_idle_tab_does_not_need_confirmation() {
    let mut app = App::new();
    spawn_idle(&mut app, 1);
    assert!(!app.close_needs_confirmation(&Ui::default()));
}

#[test]
fn multi_idle_tabs_need_confirmation() {
    let mut app = App::new();
    spawn_idle(&mut app, 2);
    assert!(app.close_needs_confirmation(&Ui::default()));
}

#[test]
fn busy_tab_needs_confirmation_even_if_single() {
    let mut app = App::new();
    spawn_idle(&mut app, 1);
    app.tabs_mut()[0].send_input(b"sleep 30\n").expect("send sleep");
    assert!(
        wait_until(Duration::from_secs(5), || {
            let _ = app.tabs_mut()[0].poll(Instant::now());
            app.close_needs_confirmation(&Ui::default())
        }),
        "single busy tab should require confirmation within 5s"
    );
}

#[test]
fn confirm_on_close_false_bypasses_dialog_for_any_tab_count() {
    let mut app = App::new();
    spawn_idle(&mut app, 5);
    app.tabs_mut()[0].send_input(b"sleep 30\n").expect("send sleep");
    // Wait for bash to fork sleep so the assertion is meaningful.
    let _ = wait_until(Duration::from_secs(5), || {
        let _ = app.tabs_mut()[0].poll(Instant::now());
        !app.busy_tabs().is_empty()
    });
    let ui = Ui { confirm_on_close: false };
    assert!(!app.close_needs_confirmation(&ui));
}

#[test]
fn busy_tabs_idle_multi_tab_returns_empty_with_correct_tab_count() {
    let mut app = App::new();
    spawn_idle(&mut app, 3);
    let busy = app.busy_tabs();
    assert!(busy.is_empty());
    let tab_count = app.tabs().len();
    let state = ConfirmCloseState::new(busy, tab_count);
    assert!(!state.is_busy_mode());
    assert_eq!(state.tab_count, 3);
}

#[test]
fn busy_tabs_lists_subprocess_with_running_label() {
    let mut app = App::new();
    spawn_idle(&mut app, 1);
    app.tabs_mut()[0].send_input(b"sleep 30\n").expect("send sleep");
    assert!(
        wait_until(Duration::from_secs(5), || {
            let _ = app.tabs_mut()[0].poll(Instant::now());
            !app.busy_tabs().is_empty()
        }),
        "bash running `sleep 30` should surface a busy tab within 5s"
    );
    let busy = app.busy_tabs();
    assert_eq!(busy.len(), 1);
    assert_eq!(busy[0].display_label, "sleep");
    assert_eq!(busy[0].state_label, "running");
    assert_eq!(busy[0].tab_index, 1);
}

#[test]
fn confirm_close_state_default_focus_is_cancel() {
    let state = ConfirmCloseState::new(Vec::new(), 2);
    assert_eq!(state.focus, FocusedButton::Cancel);
}

#[test]
fn cycle_focus_toggles_between_buttons() {
    let mut state = ConfirmCloseState::new(Vec::new(), 2);
    state.cycle_focus();
    assert_eq!(state.focus, FocusedButton::CloseAnyway);
    state.cycle_focus();
    assert_eq!(state.focus, FocusedButton::Cancel);
}
