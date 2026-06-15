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
use vibeflow::window::WindowApp;
use winit::event_loop::EventLoop;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    // Short-circuit before any GUI/winit/wgpu init so `--version` works
    // headless (over SSH, in containers, in CI). Matches the convention of
    // `git --version`, `cargo --version`, `rustc --version`.
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("vibeflow {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
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
    // Logging (Spec A'): quiet stderr by default (vibeflow=warn) + INFO+ rotated
    // file log at $XDG_STATE_HOME/vibeflow/vibeflow.log.YYYY-MM-DD. If init fails
    // (extremely rare — no $HOME, unwritable state dir, etc.), fall back to a
    // stderr-only subscriber so vibeflow stays usable. The `_log_guard` binding
    // MUST stay alive for `main`'s scope; dropping it flushes pending writes
    // and stops the worker thread.
    let _log_guard = match vibeflow::logging::init() {
        Ok(guard) => Some(guard),
        Err(e) => {
            eprintln!("warning: file logging unavailable ({e}); using stderr only");
            use tracing_subscriber::{fmt, prelude::*, EnvFilter};
            let filter = EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("vibeflow=warn"));
            tracing_subscriber::registry()
                .with(fmt::layer().with_target(false).with_filter(filter))
                .init();
            None
        }
    };

    // Panic containment: vibeflow runs the winit loop, the VTE processor, and
    // all selection/render code on this one thread, with no per-tab isolation.
    // An unhandled panic would unwind the whole process and take every tab
    // (and any in-flight job) with it, silently. Install a hook that records
    // the panic — message + location — through tracing first, chained to the
    // default hook so the standard stderr backtrace still prints. This turns a
    // silent vanish into a diagnosable crash and gets the failure into the
    // rotated file log (the `_log_guard` above flushes it on the way down).
    let default_panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown location>".to_owned());
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("<non-string panic payload>");
        tracing::error!(target: "vibeflow", location = %location, message = %message, "vibeflow panicked");
        default_panic_hook(info);
    }));

    let event_loop = EventLoop::<vibeflow::config::AppUserEvent>::with_user_event()
        .build()
        .context("create winit EventLoop")?;

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
    const MAX_THEME_FILE_BYTES: u64 = 262_144; // 256 KB
    match std::fs::metadata(in_path) {
        Ok(meta) if meta.len() > MAX_THEME_FILE_BYTES => {
            eprintln!(
                "{path_str}: file size {} bytes exceeds cap {} bytes; refusing to import",
                meta.len(),
                MAX_THEME_FILE_BYTES,
            );
            std::process::exit(1);
        }
        Ok(_) => {}  // OK to proceed
        Err(_) => {} // missing/unreadable will be caught by the read below
    }
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
