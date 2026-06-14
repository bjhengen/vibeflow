#![no_main]

use libfuzzer_sys::fuzz_target;
use vibeflow::session::osc::{DispatchEvent, OscDispatcher};

fuzz_target!(|segments: Vec<Vec<u8>>| {
    // Whole: feed the concatenation in one call.
    let whole_input: Vec<u8> = segments.concat();
    let mut whole_dispatcher = OscDispatcher::new();
    let whole_events = whole_dispatcher.feed(&whole_input);

    // Split: feed each segment through a fresh dispatcher.
    let mut split_dispatcher = OscDispatcher::new();
    let mut split_events = Vec::new();
    for seg in &segments {
        split_events.extend(split_dispatcher.feed(seg));
    }

    // A streaming parser must be split-invariant once the only representation
    // difference — how PassThrough byte-runs are chunked across feed() calls —
    // is normalised away. Completed-sequence events (AiState / Prompt /
    // SetTitle / Osc52Write) occur at the same logical point regardless of
    // split, so the coalesced streams must be equal. A genuine reassembly bug
    // (a sequence recognised whole but missed when split, or vice versa) is
    // exactly what this catches and is NOT normalised away.
    assert_eq!(
        coalesce_passthrough(whole_events),
        coalesce_passthrough(split_events),
        "OscDispatcher produced different events for whole vs segmented input"
    );
});

/// Merge consecutive `PassThrough(bytes)` events into one.
fn coalesce_passthrough(events: Vec<DispatchEvent>) -> Vec<DispatchEvent> {
    let mut out: Vec<DispatchEvent> = Vec::with_capacity(events.len());
    for ev in events {
        match (out.last_mut(), &ev) {
            (Some(DispatchEvent::PassThrough(acc)), DispatchEvent::PassThrough(next)) => {
                acc.extend_from_slice(next);
            }
            _ => out.push(ev),
        }
    }
    out
}
