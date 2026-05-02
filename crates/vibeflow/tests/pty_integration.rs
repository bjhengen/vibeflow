//! Integration test: fake AI tool emits OSC 1338 sequences, App observes
//! tracker state transitions through the full PTY pipeline.

use std::time::{Duration, Instant};

use vibeflow::app::App;
use vibeflow::session::tracker::TabState;
use vibeflow::session::SessionEvent;
use vibeflow_protocol::{Frame, State};

/// Spawn a python3-driven emitter that writes the given byte sequence to
/// stdout (which is the PTY slave) then sleeps to keep the session alive
/// long enough to observe events. Bytes are passed as a comma-separated
/// decimal list to avoid shell-escape interpretation issues — `/bin/sh` on
/// Ubuntu is `dash`, whose `printf` does not interpret `\xNN` escapes.
fn spawn_emitter_app(bytes: &[u8]) -> App {
    let bytes_repr = bytes
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let mut app = App::new();
    app.new_tab(&[
        "python3",
        "-c",
        &format!(
            "import sys, time; sys.stdout.buffer.write(bytes([{bytes_repr}])); sys.stdout.flush(); time.sleep(5)"
        ),
    ])
    .unwrap();
    app
}

/// Poll the app for up to `max` looking for a state change on tab 0 to
/// `target`. Returns `true` if observed, `false` on timeout.
fn wait_for_state(app: &mut App, target: TabState, max: Duration) -> bool {
    let deadline = Instant::now() + max;
    while Instant::now() < deadline {
        for (idx, ev) in app.poll_all(Instant::now()) {
            if idx == 0 {
                if let SessionEvent::StateChanged(state) = ev {
                    if state == target {
                        return true;
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn osc_1338_working_frame_drives_state_to_working() {
    let bytes = Frame::new(State::Working).with_tool("claude").to_bytes();
    let mut app = spawn_emitter_app(&bytes);
    assert!(
        wait_for_state(&mut app, TabState::Working, Duration::from_secs(5)),
        "expected tab 0 to transition to Working"
    );
    assert_eq!(app.tabs()[0].state(), TabState::Working);
}

#[test]
fn osc_1338_waiting_frame_drives_state_to_waiting() {
    let bytes = Frame::new(State::Waiting).to_bytes();
    let mut app = spawn_emitter_app(&bytes);
    assert!(
        wait_for_state(&mut app, TabState::Waiting, Duration::from_secs(5)),
        "expected tab 0 to transition to Waiting"
    );
}

#[test]
fn osc_133_command_start_drives_state_to_working_via_shell_path() {
    // OSC 133;C is what shells emit when a command starts. The tracker
    // should transition to Working without any AI-tool involvement.
    // Octal escapes (\033 = ESC, \007 = BEL) instead of \xNN because dash
    // printf doesn't interpret hex.
    let mut app = App::new();
    app.new_tab(&["/bin/sh", "-c", "printf '\\033]133;C\\007'; sleep 5"])
        .unwrap();
    assert!(
        wait_for_state(&mut app, TabState::Working, Duration::from_secs(5)),
        "expected tab 0 to transition to Working from OSC 133;C"
    );
}

#[test]
fn child_exit_produces_died_event() {
    // Child runs `true` (exits 0 immediately). We should observe a `Died`
    // event on tab 0 within a couple of seconds.
    let mut app = App::new();
    app.new_tab(&["/bin/true"]).unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut died = false;
    while Instant::now() < deadline && !died {
        for (_, ev) in app.poll_all(Instant::now()) {
            if matches!(ev, SessionEvent::Died) {
                died = true;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(died, "expected SessionEvent::Died for /bin/true");
    assert!(!app.tabs()[0].is_alive());
}
