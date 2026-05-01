//! `vibeflow-emit` — tiny CLI for emitting one OSC 1338 frame to stdout.
//!
//! Usage:
//!     vibeflow-emit <state> [--tool=<name>] [--project=<name>]
//!
//! `<state>` is one of: active, working, waiting, done.

use std::process::ExitCode;
use vibeflow_protocol::{emit, Frame, State};

fn print_usage(out: &mut impl std::io::Write) {
    let _ = writeln!(
        out,
        "usage: vibeflow-emit <state> [--tool=<name>] [--project=<name>]\n\
         \n\
         <state>: one of active, working, waiting, done\n\
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

    if let Err(e) = emit(&frame) {
        eprintln!("vibeflow-emit: write failed: {e}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}
