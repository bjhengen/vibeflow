//! `vibeflow-emit` — tiny CLI for emitting one OSC 1338 frame to the controlling
//! terminal (`/dev/tty`), so the bytes reach the terminal emulator PTY even when
//! the process is launched as a hook whose **stdout is captured** by the calling
//! tool (e.g. Claude Code captures hook stdout, so writing there would silently
//! swallow the OSC sequence instead of delivering it to vibeflow's reader thread).
//!
//! **Target selection** (in priority order):
//! 1. If `VIBEFLOW_EMIT_STDOUT` is set to a non-empty value → write to stdout.
//!    Useful for pipes, CI, and testing where there is no controlling terminal.
//! 2. Try to open `/dev/tty` (the controlling terminal). On success, write there
//!    so the OSC reaches the terminal emulator regardless of stdout redirection.
//! 3. If `/dev/tty` cannot be opened (no controlling terminal — piped/CI/redirected)
//!    → fall back to stdout. Preserves the original behaviour in non-hook contexts.
//!
//! Usage:
//!
//! ```text
//! vibeflow-emit <state> [--tool=<name>] [--project=<name>]
//! ```
//!
//! `<state>` is one of: active, working, waiting, done.

use std::io::Write;
use std::process::ExitCode;
use vibeflow_protocol::{emit, emit_to, Frame, State};

fn print_usage(out: &mut impl std::io::Write) {
    let _ = writeln!(
        out,
        "usage: vibeflow-emit <state> [--tool=<name>] [--project=<name>]\n\
         \n\
         <state>: one of active, working, waiting, done\n\
         \n\
         Writes the OSC 1338 frame to the controlling terminal (/dev/tty) so it\n\
         reaches the terminal emulator even when stdout is captured by a hook\n\
         runner. Set VIBEFLOW_EMIT_STDOUT=1 to force stdout (pipes/CI/debug).\n\
         \n\
         examples:\n  \
         vibeflow-emit waiting --tool=claude\n  \
         vibeflow-emit working --tool=codex --project=vibeflow"
    );
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.is_empty() || matches!(args[0].as_str(), "-h" | "--help") {
        let mut out = std::io::stderr();
        print_usage(&mut out);
        return if args.is_empty() {
            ExitCode::from(2)
        } else {
            ExitCode::SUCCESS
        };
    }

    let state: State = match args[0].parse() {
        Ok(s) => s,
        Err(_) => {
            eprintln!("vibeflow-emit: unknown state {:?}", args[0]);
            print_usage(&mut std::io::stderr());
            return ExitCode::from(2);
        }
    };

    let mut frame = Frame::new(state);
    for arg in &args[1..] {
        if let Some(v) = arg.strip_prefix("--tool=") {
            frame.tool = Some(v.to_owned());
        } else if let Some(v) = arg.strip_prefix("--project=") {
            frame.project = Some(v.to_owned());
        } else {
            eprintln!("vibeflow-emit: unexpected argument {arg:?}");
            print_usage(&mut std::io::stderr());
            return ExitCode::from(2);
        }
    }

    // Determine where to send the OSC 1338 frame.
    //
    // When vibeflow-emit runs as a Claude Code hook (or similar tool), the hook
    // runner captures stdout, so bytes written there never reach the terminal
    // emulator's PTY.  Writing to the controlling terminal /dev/tty bypasses that
    // capture and delivers the OSC directly to vibeflow's reader thread.
    //
    // The VIBEFLOW_EMIT_STDOUT escape hatch lets callers force stdout — useful in
    // CI, pipes, and the integration-test suite (which has no real PTY to open).
    let want_stdout = std::env::var_os("VIBEFLOW_EMIT_STDOUT")
        .map(|v| !v.is_empty())
        .unwrap_or(false);

    let result = if want_stdout {
        emit(&frame)
    } else {
        match std::fs::OpenOptions::new().write(true).open("/dev/tty") {
            Ok(mut tty) => emit_to(&mut tty, &frame).and_then(|()| tty.flush()),
            // No controlling terminal (piped/CI/redirected) — fall back to stdout
            // so non-hook usage continues to work as before.
            Err(_) => emit(&frame),
        }
    };

    if let Err(e) = result {
        eprintln!("vibeflow-emit: write failed: {e}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}
