//! `vibeflow` binary entry point.
//!
//! Initialises tracing, builds a winit `EventLoop`, and runs the
//! [`vibeflow::window::WindowApp`]. The Stage 3 sleep-loop demo is gone —
//! Stage 4 is the real binary.

use anyhow::{Context, Result};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use vibeflow::window::WindowApp;
use winit::event_loop::EventLoop;

fn main() -> Result<()> {
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
