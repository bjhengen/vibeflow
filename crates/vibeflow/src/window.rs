//! winit `ApplicationHandler` integration: the [`WindowApp`] type owns the
//! `Window`, the [`crate::render::Renderer`], and the [`crate::app::App`].
//! Drives polling, ticking, and event routing on the main thread.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::App;
use crate::render::Renderer;
use crate::session::SessionEvent;

/// User-facing winit application. Implements [`ApplicationHandler`].
///
/// Lifecycle:
/// - `new()` builds an empty `App` (no tabs yet) — no winit calls.
/// - `resumed()` (called once after `event_loop.run_app`) creates the window,
///   initialises the renderer, and spawns the first tab.
/// - `window_event(...)` routes user events (close, redraw, resize, keys).
/// - `about_to_wait(...)` (added in Task 5) drains `App::poll_all` and
///   `App::tick_all`, logs `SessionEvent`s via tracing, schedules the next
///   wake-up via `ControlFlow::WaitUntil`.
pub struct WindowApp {
    /// `None` until `resumed` fires for the first time. After that, `Some` for
    /// the entire process lifetime.
    window: Option<Arc<Window>>,
    /// `None` until window is created and wgpu init succeeds. Wrapped in an
    /// `Option` so `resumed` can construct it lazily.
    renderer: Option<Renderer>,
    /// The application core. Holds every tab.
    app: App,
    /// Latest modifier state from `WindowEvent::ModifiersChanged`. winit 0.30
    /// delivers modifier state via a separate event rather than as a field on
    /// `KeyEvent`, so we cache it here and pass it into the `key_to_bytes`
    /// helper alongside each key press.
    ///
    /// Unused until Task 7 wires keyboard handling; the `#[allow(dead_code)]`
    /// is removed in Task 7.
    #[allow(dead_code)]
    current_modifiers: ModifiersState,
}

impl WindowApp {
    /// Build a `WindowApp` with no window and no tabs. Call
    /// `event_loop.run_app(&mut app)` to drive it.
    #[must_use]
    pub fn new() -> Self {
        Self {
            window: None,
            renderer: None,
            app: App::new(),
            current_modifiers: ModifiersState::empty(),
        }
    }

    /// Spawn the user's shell as the first tab. `$SHELL` if set; otherwise
    /// `/bin/sh`. The path is logged at info level.
    fn spawn_first_tab(&mut self) -> Result<()> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        tracing::info!(shell = %shell, "spawning first tab");
        self.app
            .new_tab(&[shell.as_str()])
            .with_context(|| format!("spawn first tab via {shell}"))?;
        Ok(())
    }

    /// React to a single `SessionEvent`. Stage 4 just logs; Stage 5 will route
    /// `PassThrough` bytes into the per-tab `alacritty_terminal` grid and call
    /// `window.request_redraw()` on `StateChanged`.
    fn handle_session_event(&mut self, idx: usize, ev: SessionEvent) {
        match ev {
            SessionEvent::StateChanged(state) => {
                tracing::info!(tab = idx, state = ?state, "state changed");
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            SessionEvent::PassThrough(bytes) => {
                // Stage 5 sends these into alacritty_terminal::Term::input.
                // Stage 4 just records the byte count at trace level so we can
                // sanity-check throughput from the log without spamming.
                tracing::trace!(tab = idx, bytes = bytes.len(), "passthrough");
            }
            SessionEvent::Died => {
                tracing::warn!(tab = idx, "session died");
                // The session stays in `App.tabs` with `is_alive() == false`.
                // Stage 6 (tab bar) renders the dead-tab banner; closing the
                // tab here would remove the thing the banner needs to draw.
                // The window does not auto-exit on the last tab dying in
                // Stage 4 — the user closes the window with the close button.
            }
        }
    }
}

impl Default for WindowApp {
    fn default() -> Self {
        Self::new()
    }
}

impl ApplicationHandler for WindowApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            // resumed can fire more than once on some platforms (e.g. when the
            // app comes back from suspended). For Stage 4 we only construct the
            // window the first time.
            return;
        }
        let window_attrs = Window::default_attributes()
            .with_title("vibeflow")
            .with_inner_size(winit::dpi::LogicalSize::new(960, 600));
        let window = match event_loop.create_window(window_attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                tracing::error!(error = %e, "failed to create window");
                event_loop.exit();
                return;
            }
        };
        let renderer = match Renderer::new(window.clone()) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = ?e, "failed to initialise renderer");
                event_loop.exit();
                return;
            }
        };
        self.window = Some(window);
        self.renderer = Some(renderer);

        if let Err(e) = self.spawn_first_tab() {
            tracing::error!(error = ?e, "failed to spawn first tab");
            event_loop.exit();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                tracing::info!("close requested; exiting");
                event_loop.exit();
            }
            WindowEvent::RedrawRequested => {
                if let Some(renderer) = self.renderer.as_ref() {
                    if let Err(e) = renderer.render() {
                        tracing::warn!(error = ?e, "render error");
                    }
                }
            }
            // Resize, KeyboardInput, etc. arrive in Tasks 6–7.
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();

        // Drain bytes that arrived since last tick.
        for (idx, ev) in self.app.poll_all(now) {
            self.handle_session_event(idx, ev);
        }
        // Fire any timeout-driven transitions.
        for (idx, ev) in self.app.tick_all(now) {
            self.handle_session_event(idx, ev);
        }

        // Re-arm a 100ms wake-up so trackers tick steadily. Stage 6+ will
        // compute the exact next deadline from the per-session tracker state.
        event_loop.set_control_flow(ControlFlow::WaitUntil(now + Duration::from_millis(100)));
    }
}
