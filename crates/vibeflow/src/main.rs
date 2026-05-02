//! `vibeflow` binary entry point.
//!
//! Initialises tracing, builds a winit `EventLoop`, and runs the
//! [`vibeflow::window::WindowApp`]. The Stage 3 sleep-loop demo is gone —
//! Stage 4 is the real binary.

use anyhow::{Context, Result};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use vibeflow::window::WindowApp;
use winit::event_loop::{ControlFlow, EventLoop};

fn main() -> Result<()> {
    // tracing-subscriber: respect RUST_LOG when set, otherwise default to
    // `vibeflow=info,warn`. Stage 9 will add file logging + rotation.
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("vibeflow=info,warn"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(false))
        .init();

    let event_loop = EventLoop::new().context("create winit EventLoop")?;
    // `Wait` is the cheapest mode — the loop blocks until an event arrives.
    // Task 5 swaps this for `WaitUntil(deadline)` so tracker timeouts fire.
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = WindowApp::new();
    event_loop
        .run_app(&mut app)
        .context("run winit event loop")?;
    Ok(())
}
