//! Integration test for the vibeflow-emit CLI.
//!
//! The `/dev/tty` success path requires a real PTY and cannot be exercised in
//! the Cargo test runner (which runs without a controlling terminal).  That path
//! is covered by the manual VNC smoke verify described in the fix commit.
//!
//! Here we pin the two deterministic contracts:
//! 1. `VIBEFLOW_EMIT_STDOUT=1` escape hatch → exact OSC 1338 bytes on stdout.
//! 2. Unknown state argument → exit code 2 + "usage: vibeflow-emit" on stderr.

use std::process::Command;

#[test]
fn emits_osc_1338_to_stdout_when_override_set() {
    let out = Command::new(env!("CARGO_BIN_EXE_vibeflow-emit"))
        .args(["waiting", "--tool=claude"])
        .env("VIBEFLOW_EMIT_STDOUT", "1")
        .output()
        .expect("run vibeflow-emit");
    assert!(out.status.success(), "exit: {:?}", out.status);
    // Exact bytes per Frame::to_bytes (same wire format as the lib.rs emit test, with state=waiting here).
    // Key order is state → tool (→ project if set), terminator is BEL (\x07).
    assert_eq!(
        out.stdout,
        b"\x1b]1338;state=waiting;tool=claude\x07".to_vec(),
        "stdout must carry the exact OSC 1338 frame"
    );
}

#[test]
fn unknown_state_exits_2_and_writes_usage_to_stderr() {
    // VIBEFLOW_EMIT_STDOUT is irrelevant here — bad-arg exits before the write path; set only for consistency.
    let out = Command::new(env!("CARGO_BIN_EXE_vibeflow-emit"))
        .args(["bogus"])
        .env("VIBEFLOW_EMIT_STDOUT", "1")
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("usage: vibeflow-emit"),
        "stderr must contain usage text; got: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
}
