//! Integration smoke for OSC 52 clipboard write.
//!
//! Spawns a real `PtySession` (via python3, which outputs the OSC sequence
//! immediately on stdout), polls until the dispatcher emits
//! `SessionEvent::Osc52ClipboardWrite`, then asserts the selection + text
//! round-trip correctly.
//!
//! Does NOT test the actual clipboard write side-effect (requires a display
//! server). The unit test in `clipboard.rs` covers the API shape; this test
//! covers the parser-dispatcher-session pipeline end-to-end.

use std::time::{Duration, Instant};
use vibeflow::session::osc::Osc52Selection;
use vibeflow::session::session::{PtySession, SessionEvent};
use vibeflow::session::tracker::TrackerConfig;

#[test]
fn osc52_write_full_pipeline_round_trip() {
    // Spawn python3 which writes the literal OSC 52 sequence to stdout.
    // This matches the in-session unit test pattern that Task 5 added.
    // PtySession::spawn signature: spawn(argv: &[&str], config: TrackerConfig,
    //                                    history_lines: usize) -> std::io::Result<Self>
    let mut session = PtySession::spawn(
        &[
            "python3",
            "-c",
            "import sys; sys.stdout.buffer.write(b'\\x1b]52;c;SGVsbG8=\\x07'); sys.stdout.flush()",
        ],
        TrackerConfig::default(),
        10_000,
    )
    .expect("spawn PtySession");

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut observed = None;
    while Instant::now() < deadline {
        let evs = session.poll(Instant::now());
        for ev in evs {
            if let SessionEvent::Osc52ClipboardWrite { selection, text } = ev {
                observed = Some((selection, text));
                break;
            }
        }
        if observed.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let (selection, text) = observed.expect("Osc52ClipboardWrite within 3s");
    assert_eq!(
        selection,
        Osc52Selection::Clipboard,
        "selection 'c' maps to Clipboard"
    );
    assert_eq!(text, "Hello", "decoded text round-trips correctly");
}
