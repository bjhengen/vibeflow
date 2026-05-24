//! Integration tests for the `--version` / `-V` CLI flag.
//!
//! Spawns the built `vibeflow` binary as a subprocess and asserts the flag
//! short-circuits before any GUI init (no winit window, no wgpu, no config
//! file access). Mirrors the pattern in `crates/vibeflow-protocol/tests/emit_cli.rs`.

use std::process::Command;
use std::time::Duration;

fn run_with_flag(flag: &str) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vibeflow"));
    cmd.arg(flag);
    // Override $HOME to a path that does not exist so the test fails LOUDLY
    // if the flag accidentally falls through to config loading.
    cmd.env("HOME", "/nonexistent/vibeflow-cli-version-test");
    cmd.env_remove("XDG_CONFIG_HOME");
    cmd.env_remove("XDG_STATE_HOME");
    cmd.output().expect("spawn vibeflow")
}

#[test]
fn long_version_flag_prints_name_and_version_and_exits_zero() {
    let start = std::time::Instant::now();
    let out = run_with_flag("--version");
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "--version took {elapsed:?}; GUI init must be skipped"
    );
    assert_eq!(out.status.code(), Some(0), "exit: {:?}", out.status);
    let expected = format!("vibeflow {}\n", env!("CARGO_PKG_VERSION"));
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
    assert_eq!(out.stderr, b"", "stderr must be empty");
}

#[test]
fn short_version_flag_prints_name_and_version_and_exits_zero() {
    let out = run_with_flag("-V");
    assert_eq!(out.status.code(), Some(0), "exit: {:?}", out.status);
    let expected = format!("vibeflow {}\n", env!("CARGO_PKG_VERSION"));
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
    assert_eq!(out.stderr, b"", "stderr must be empty");
}

#[test]
fn version_flag_takes_precedence_over_other_args() {
    // Putting --version after a bogus arg must still exit 0 with the version.
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vibeflow"));
    cmd.args(["--some-future-arg", "--version"]);
    cmd.env("HOME", "/nonexistent/vibeflow-cli-version-test");
    let out = cmd.output().expect("spawn vibeflow");
    assert_eq!(out.status.code(), Some(0), "exit: {:?}", out.status);
    let expected = format!("vibeflow {}\n", env!("CARGO_PKG_VERSION"));
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
}
