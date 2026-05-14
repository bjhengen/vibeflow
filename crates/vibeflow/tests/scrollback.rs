//! Stage 12 integration tests — scrollback rendering + selection covers history.

use std::time::{Duration, Instant};
use vibeflow::app::App;

fn drive_until(app: &mut App, deadline: Instant) {
    while Instant::now() < deadline {
        let now = Instant::now();
        let _ = app.poll_all(now);
        let _ = app.tick_all(now);
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn scroll_by_then_to_bottom_round_trip() {
    let mut app = App::new();
    let _ = app
        .new_tab(&[
            "/bin/sh",
            "-c",
            "for i in $(seq 1 200); do echo $i; done; sleep 5",
        ])
        .expect("spawn");
    // Drive long enough for all 200 lines to flow through the dispatcher.
    drive_until(&mut app, Instant::now() + Duration::from_millis(600));

    let active = app.active();
    let now = Instant::now();
    app.tabs_mut()[active].scroll_by(-50, now);
    let after_scroll = app.tabs()[active].display_offset();
    assert!(
        after_scroll > 0,
        "scroll_by(-50) should advance display_offset; got {after_scroll}"
    );

    app.tabs_mut()[active].scroll_to_bottom(now);
    assert_eq!(app.tabs()[active].display_offset(), 0);
}

#[test]
fn scrollbar_fade_arms_on_scroll_and_decays() {
    let mut app = App::new();
    let _ = app
        .new_tab(&["/bin/sh", "-c", "echo hello; sleep 5"])
        .expect("spawn");
    drive_until(&mut app, Instant::now() + Duration::from_millis(200));

    let now = Instant::now();
    assert_eq!(app.tabs()[0].scrollbar_fade_alpha(now), 0.0);
    app.tabs_mut()[0].scroll_by(-1, now);
    assert_eq!(app.tabs()[0].scrollbar_fade_alpha(now), 1.0);

    // Past default fade_ms (1500), should be 0.
    let later = now + Duration::from_millis(1600);
    assert_eq!(app.tabs()[0].scrollbar_fade_alpha(later), 0.0);
}

#[test]
fn select_all_with_scrollback_includes_history() {
    let mut app = App::new();
    let _ = app.new_tab(&["bash"]).expect("spawn bash");
    drive_until(&mut app, Instant::now() + Duration::from_millis(500));
    // Issue `seq 1 200` to the shell to produce history.
    let active = app.active();
    let _ = app.tabs_mut()[active].send_input(b"seq 1 200\n");
    drive_until(&mut app, Instant::now() + Duration::from_millis(2000));

    app.select_all_active();
    let s = &app.tabs()[active];
    let text = s.selection.text(s.term()).unwrap_or_default();
    // Terminal rows are padded to full column width with spaces, so each line
    // looks like "1   ...79 spaces...\n". The pattern "\n1 " matches a line
    // that starts with "1" (i.e. the single-digit seq output).
    let has_line_1 = text.split('\n').any(|l| l.trim() == "1");
    let has_line_200 = text.split('\n').any(|l| l.trim() == "200");
    assert!(
        has_line_1,
        "selection should include line '1' from scrollback; text len={}",
        text.len()
    );
    assert!(
        has_line_200,
        "selection should include line '200' near the bottom; text len={}",
        text.len()
    );
}
