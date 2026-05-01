#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Goal: never panic, never OOM, regardless of input.
    let _ = vibeflow_protocol::parse(data);
});
