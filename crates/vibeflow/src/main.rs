//! `vibeflow` binary entry point.
//!
//! Initialises tracing, builds a winit `EventLoop`, and runs the
//! [`vibeflow::window::WindowApp`]. The Stage 3 sleep-loop demo is gone —
//! Stage 4 is the real binary.
//!
//! Also handles the `--import-colors <path> [--overwrite]` subcommand, which
//! converts an iTerm2 `.itermcolors` file into a vibeflow theme TOML and
//! writes it to `~/.config/vibeflow/themes/<name>.toml`.

use anyhow::{Context, Result};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use vibeflow::window::WindowApp;
use winit::event_loop::EventLoop;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    // Note: only the space-separated form `--import-colors <path>` is
    // recognized; the `--import-colors=PATH` (=-joined) form is not. A
    // future clap migration would handle both.
    if let Some(pos) = args.iter().position(|a| a == "--import-colors") {
        let Some(path_str) = args.get(pos + 1) else {
            eprintln!("usage: vibeflow --import-colors <path> [--overwrite]");
            std::process::exit(2);
        };
        if path_str.starts_with("--") {
            eprintln!(
                "usage: vibeflow --import-colors <path> [--overwrite]  (expected a path, got `{path_str}`)"
            );
            std::process::exit(2);
        }
        let overwrite = args.iter().any(|a| a == "--overwrite");
        run_import_colors(path_str, overwrite);
        return Ok(());
    }
    run_gui()
}

fn run_gui() -> Result<()> {
    // tracing-subscriber: respect RUST_LOG when set, otherwise default to
    // `vibeflow=info,warn`. Stage 9 will add file logging + rotation.
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("vibeflow=info,warn"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(false))
        .init();

    let event_loop = EventLoop::<vibeflow::config::AppUserEvent>::with_user_event()
        .build()
        .context("create winit EventLoop")?;
    // The control flow is set per-iteration in `WindowApp::about_to_wait`
    // (a 100 ms `WaitUntil` so tracker timeouts fire). The initial value
    // doesn't matter — the first `about_to_wait` overrides it.

    let proxy = event_loop.create_proxy();
    let mut app = WindowApp::new(proxy);
    event_loop
        .run_app(&mut app)
        .context("run winit event loop")?;
    Ok(())
}

fn run_import_colors(path_str: &str, overwrite: bool) {
    use std::path::Path;
    let in_path = Path::new(path_str);
    let bytes = match std::fs::read(in_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {path_str}: {e}");
            std::process::exit(1);
        }
    };
    let mut theme = match vibeflow::theme::iterm2::parse_itermcolors(&bytes) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("parse error in {path_str}: {e}");
            std::process::exit(1);
        }
    };
    let name = in_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("imported");
    if name == "default" {
        eprintln!("'default' is a reserved theme name");
        std::process::exit(1);
    }
    theme.name = name.to_owned();

    let Some(config_base) = dirs::config_dir() else {
        eprintln!("cannot determine config directory (is HOME set?)");
        std::process::exit(1);
    };
    let themes_dir = config_base.join("vibeflow").join("themes");
    if let Err(e) = std::fs::create_dir_all(&themes_dir) {
        eprintln!("cannot create {}: {e}", themes_dir.display());
        std::process::exit(1);
    }
    let out_path = themes_dir.join(format!("{name}.toml"));
    if out_path.exists() && !overwrite {
        eprintln!(
            "theme '{name}' already exists at {}; use --overwrite to replace",
            out_path.display()
        );
        std::process::exit(1);
    }
    let toml_str = theme.to_toml();
    if let Err(e) = std::fs::write(&out_path, toml_str) {
        eprintln!("cannot write {}: {e}", out_path.display());
        std::process::exit(1);
    }
    println!("imported theme '{name}' to {}", out_path.display());
}
