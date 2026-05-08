//! Integration test: a realistic byte stream fed through `OscDispatcher` →
//! events routed into `AiStateTracker` → expected state sequence.

use std::time::{Duration, Instant};

use vibeflow::session::osc::{DispatchEvent, OscDispatcher, PromptMarker};
use vibeflow::session::tracker::{AiStateTracker, TabState, TrackerConfig, TrackerInput};

/// Helper: feed bytes, route events into the tracker, return a vector of
/// (post-event tracker state, was-it-a-change-bool) per event.
///
/// Each event is timestamped 200 ms after its predecessor in the same feed
/// call, well past the 100 ms debounce window. In real use, bytes arrive on a
/// PTY over wall-clock time so this models reality faithfully; the integration
/// test would otherwise see all events at the same instant and have legitimate
/// state transitions silently dropped by debounce.
fn feed_and_track(
    dispatcher: &mut OscDispatcher,
    tracker: &mut AiStateTracker,
    bytes: &[u8],
    start: Instant,
) -> Vec<(TabState, bool)> {
    let events = dispatcher.feed(bytes);
    events
        .into_iter()
        .enumerate()
        .map(|(i, ev)| {
            let now = start + Duration::from_millis(200 * i as u64);
            match ev {
                DispatchEvent::AiState(frame) => {
                    let changed = tracker.on_input(TrackerInput::AiFrame(frame), now);
                    (tracker.state(), changed)
                }
                DispatchEvent::Prompt(marker) => {
                    let changed = tracker.on_input(TrackerInput::Prompt(marker), now);
                    (tracker.state(), changed)
                }
                DispatchEvent::SetTitle(_title) => {
                    // OSC 0/2 window-title updates don't drive tracker state; the
                    // integration test just observes the tracker state unchanged.
                    let _ = _title;
                    (tracker.state(), false)
                }
                DispatchEvent::PassThrough(bytes) => {
                    // Real PTY/terminal-grid path (Stage 3+) would forward bytes
                    // here. For Stage 2, observing output through the tracker
                    // is the nearest equivalent.
                    let _ = bytes;
                    let changed = tracker.on_input(TrackerInput::OutputObserved, now);
                    (tracker.state(), changed)
                }
            }
        })
        .collect()
}

#[test]
fn shell_prompt_then_command_then_done() {
    // Wall-clock-style timestamps spread far enough apart to avoid debounce.
    let mut t0 = Instant::now();
    let mut bump = || {
        t0 += Duration::from_secs(1);
        t0
    };

    let mut d = OscDispatcher::new();
    let mut tr = AiStateTracker::new(TrackerConfig::default());

    // Shell renders a prompt: OSC 133;A then OSC 133;B, then prints "$ ".
    let prompt = b"\x1b]133;A\x07\x1b]133;B\x07$ ";
    let states = feed_and_track(&mut d, &mut tr, prompt, bump());
    // Two prompt markers + one passthrough ("$ "). The first PromptStart
    // transitions Active -> Idle; PromptEnd is Idle -> Idle (no change);
    // PassThrough drives OutputObserved (no change).
    assert_eq!(
        states,
        vec![
            (TabState::Idle, true),
            (TabState::Idle, false),
            (TabState::Idle, false),
        ]
    );

    // User runs `claude`. Shell emits OSC 133;C (command-start). Then claude
    // prints output, then emits OSC 1338;state=working, then more output, then
    // OSC 1338;state=waiting.
    let session = b"\x1b]133;C\x07hello from claude\
                   \x1b]1338;state=working;tool=claude\x07\
                   ...working...\
                   \x1b]1338;state=waiting;tool=claude\x07";
    let states = feed_and_track(&mut d, &mut tr, session, bump());

    // Expect, in order:
    //   - Prompt(CommandStart)             → Idle → Working (changed)
    //   - PassThrough("hello from claude") → OutputObserved (no change)
    //   - AiState(working)                 → Working (no change — same state)
    //   - PassThrough("...working...")     → OutputObserved (no change)
    //   - AiState(waiting)                 → Working → Waiting (changed)
    assert_eq!(
        states,
        vec![
            (TabState::Working, true),
            (TabState::Working, false),
            (TabState::Working, false),
            (TabState::Working, false),
            (TabState::Waiting, true),
        ]
    );

    // Claude exits. Shell emits OSC 133;D (command-end), then prints another
    // prompt.
    let after = b"\x1b]133;D;0\x07\x1b]133;A\x07$ ";
    let states = feed_and_track(&mut d, &mut tr, after, bump());

    // CommandEnd transitions Waiting → Idle (changed); PromptStart is Idle →
    // Idle (no change); PassThrough is OutputObserved.
    assert_eq!(
        states,
        vec![
            (TabState::Idle, true),
            (TabState::Idle, false),
            (TabState::Idle, false),
        ]
    );
}

#[test]
fn unknown_osc_passes_through_without_disturbing_tracker_state() {
    let mut d = OscDispatcher::new();
    let mut tr = AiStateTracker::new(TrackerConfig::default());
    let now = Instant::now();

    // Set tracker to Working via an explicit AI frame so we can observe that
    // OSC 0 (window-title) doesn't change the tracker state.
    feed_and_track(&mut d, &mut tr, b"\x1b]1338;state=working\x07", now);
    assert_eq!(tr.state(), TabState::Working);

    // OSC 0 (window-title) is recognised in Stage 9 but doesn't drive tracker
    // state — the title update is a UI concern (Stage 9+), not a tracker event.
    let states = feed_and_track(
        &mut d,
        &mut tr,
        b"\x1b]0;my window title\x07",
        now + Duration::from_secs(1),
    );
    assert_eq!(states, vec![(TabState::Working, false)]);
    assert_eq!(tr.state(), TabState::Working);
}

#[test]
fn dispatcher_marker_smoke_check() {
    // A sanity check that the imports above resolve. Catches a refactor that
    // accidentally removes one of the public re-exports.
    let marker = PromptMarker::PromptStart;
    assert_eq!(format!("{marker:?}"), "PromptStart");
}
