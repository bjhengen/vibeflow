//! winit `ApplicationHandler` integration: the [`WindowApp`] type owns the
//! `Window`, the [`crate::render::Renderer`], and the [`crate::app::App`].
//! Drives polling, ticking, and event routing on the main thread.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

use crate::app::App;
use crate::render::Renderer;
use crate::session::SessionEvent;

/// Compute terminal grid dimensions (rows, cols) from a window's physical
/// pixel size and per-cell pixel size. Floor-divides; clamps to at least 1×1
/// so degenerate (0, 0) surfaces still produce a usable grid for the child.
fn pixels_to_grid(width_px: u32, height_px: u32, cell_w: u32, cell_h: u32) -> (u16, u16) {
    let cols = (width_px / cell_w).max(1);
    let rows = (height_px / cell_h).max(1);
    // PTY size fields are u16. Realistic terminal sizes are well under
    // u16::MAX (~65k cells), but we still saturate-cast defensively.
    (
        rows.min(u16::MAX as u32) as u16,
        cols.min(u16::MAX as u32) as u16,
    )
}

/// Translate a winit key press into the bytes the PTY child expects on stdin.
/// Returns `None` for releases, modifier-only events, and any key not in
/// Stage 4's minimal subset (Stage 8 fills in arrows, F-keys, full Alt/Meta
/// handling, etc.).
///
/// Takes decomposed parameters rather than a `&KeyEvent` because winit 0.30's
/// `KeyEvent` has a `pub(crate)` `platform_specific` field that prevents
/// external struct-literal construction in tests.
fn key_to_bytes(
    logical_key: &Key,
    state: ElementState,
    modifiers: ModifiersState,
) -> Option<Vec<u8>> {
    if state != ElementState::Pressed {
        return None;
    }
    match logical_key {
        Key::Character(s) => {
            if modifiers.contains(ModifiersState::CONTROL) {
                // Ctrl+letter → 0x01..=0x1A. Only handle a..=z; leave numbers
                // and punctuation to Stage 8.
                let lower = s.to_ascii_lowercase();
                let mut chars = lower.chars();
                if let (Some(c), None) = (chars.next(), chars.next()) {
                    if c.is_ascii_lowercase() {
                        return Some(vec![(c as u8) - b'`']); // 'a' (0x61) - 0x60 = 0x01
                    }
                }
                None
            } else {
                Some(s.as_bytes().to_vec())
            }
        }
        Key::Named(NamedKey::Enter) => Some(vec![b'\r']),
        Key::Named(NamedKey::Backspace) => Some(vec![0x7f]),
        Key::Named(NamedKey::Tab) => Some(vec![b'\t']),
        Key::Named(NamedKey::Escape) => Some(vec![0x1b]),
        // winit 0.30 routes the spacebar through `NamedKey::Space`, not
        // through `Character(" ")` — so without this arm it falls into the
        // `_ => None` catch-all and the byte never reaches the PTY.
        Key::Named(NamedKey::Space) => Some(vec![b' ']),
        // Anything else → Stage 8.
        _ => None,
    }
}

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
    current_modifiers: ModifiersState,
    /// Latest cursor position from `WindowEvent::CursorMoved`. Used by mouse
    /// click handlers to hit-test the tab bar.
    cursor_pos: Option<(u32, u32)>,
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
            cursor_pos: None,
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
            SessionEvent::TermUpdated => {
                // Bytes were fed into the per-session Term in PtySession::poll.
                // Request a redraw so the renderer reads the new grid contents.
                tracing::trace!(tab = idx, "term updated");
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            SessionEvent::Died => {
                tracing::warn!(tab = idx, "session died");
                // The session stays in `App.tabs` with `is_alive() == false`.
                // Stage 6 (tab bar) renders the dead-tab banner; closing the
                // tab here would remove the thing the banner needs to draw.
                // The window does not auto-exit on the last tab dying in
                // Stage 4 — the user closes the window with the close button.
            }
            SessionEvent::Bell => {
                tracing::trace!(tab = idx, "bell rung");
                // Only flash for the active tab to avoid background tabs spamming.
                if idx == self.app.active() {
                    if let Some(renderer) = self.renderer.as_mut() {
                        renderer.note_bell();
                    }
                    if let Some(window) = self.window.as_ref() {
                        window.request_redraw();
                    }
                }
            }
        }
    }

    /// Hit-test the latest cursor position against the tab bar and dispatch
    /// the corresponding action.
    fn handle_left_click_release(&mut self) {
        use crate::render::tabs::{TabBarHit, TabBarLayout};

        let Some((px, py)) = self.cursor_pos else {
            return;
        };
        // We need the same layout the renderer used. Since cell pitch + window
        // width are the inputs, recompute it here from the renderer's atlas.
        let Some(renderer) = self.renderer.as_ref() else {
            return;
        };
        let (_cell_w, cell_h) = renderer.cell_pitch();
        let (window_w, _window_h) = renderer.surface_size();
        let layout = TabBarLayout::compute(window_w, cell_h, self.app.tabs().len());

        match layout.hit_test(px, py) {
            TabBarHit::NewTab => {
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
                if let Err(e) = self.app.new_tab(&[shell.as_str()]) {
                    tracing::warn!(error = ?e, "new_tab failed");
                }
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            TabBarHit::TabBody(idx) => {
                self.app.set_active(idx);
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            TabBarHit::TabClose(idx) => {
                self.app.close_tab(idx);
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            TabBarHit::None => {}
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
            return;
        }

        // Resize the freshly-spawned PTY to match the actual window size.
        // `spawn_pty` defaults to 80×24, which is wrong if the user opened a
        // larger window. Some compositors don't fire `WindowEvent::Resized`
        // on initial show, so we don't rely on that to correct the size.
        if let Some(renderer) = self.renderer.as_ref() {
            let (width, height) = renderer.surface_size();
            let (cell_w, cell_h) = renderer.cell_pitch();
            // Reserve the tab-bar strip at the top — the PTY only sees the
            // visible cell area, so its row count matches what's actually
            // rendered below the bar.
            let bar_h = crate::render::tabs::tab_bar_height_px(cell_h);
            let visible_h = height.saturating_sub(bar_h);
            let (rows, cols) = pixels_to_grid(width, visible_h, cell_w, cell_h);
            if let Err(e) = self.app.resize_all(rows, cols) {
                tracing::warn!(error = %e, rows, cols, "initial PTY resize failed");
            }
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
                let term = self.app.active_term();
                let Some(renderer) = self.renderer.as_mut() else {
                    return;
                };
                match renderer.render(term, &self.app) {
                    Ok(()) => {}
                    Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                        // Surface needs to be re-created with current config.
                        // The existing render config (size, format) is reused.
                        renderer.reconfigure();
                    }
                    Err(wgpu::SurfaceError::OutOfMemory) => {
                        tracing::error!("GPU out of memory; exiting");
                        event_loop.exit();
                    }
                    Err(wgpu::SurfaceError::Timeout) => {
                        // Frame took longer than the deadline. Skip this frame
                        // and request another. Common during driver hiccups.
                        if let Some(window) = self.window.as_ref() {
                            window.request_redraw();
                        }
                    }
                }
            }
            WindowEvent::Resized(new_size) => {
                let cell_pitch = self.renderer.as_ref().map(|r| r.cell_pitch());
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(new_size.width, new_size.height);
                }
                if let Some((cell_w, cell_h)) = cell_pitch {
                    let bar_h = crate::render::tabs::tab_bar_height_px(cell_h);
                    let visible_h = new_size.height.saturating_sub(bar_h);
                    let (rows, cols) = pixels_to_grid(new_size.width, visible_h, cell_w, cell_h);
                    if let Err(e) = self.app.resize_all(rows, cols) {
                        tracing::warn!(error = %e, rows, cols, "PTY resize failed");
                    }
                }
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.current_modifiers = modifiers.state();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                // Log every keypress at trace level so we can diagnose which
                // logical keys reach us and which fall into the catch-all.
                // Run with `RUST_LOG=vibeflow=trace` to see this.
                tracing::trace!(
                    state = ?event.state,
                    logical_key = ?event.logical_key,
                    text = ?event.text,
                    "key event"
                );
                match key_to_bytes(&event.logical_key, event.state, self.current_modifiers) {
                    Some(bytes) => {
                        tracing::trace!(?bytes, "key → pty bytes");
                        if let Err(e) = self.app.send_input(&bytes) {
                            tracing::warn!(error = %e, "send_input failed");
                        }
                    }
                    None => {
                        if event.state == winit::event::ElementState::Pressed {
                            tracing::trace!(
                                logical_key = ?event.logical_key,
                                "press dropped (no key_to_bytes mapping)"
                            );
                        }
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_pos = Some((position.x as u32, position.y as u32));
            }
            WindowEvent::MouseInput { state, button, .. } => {
                use winit::event::{ElementState, MouseButton};
                if state == ElementState::Released && button == MouseButton::Left {
                    self.handle_left_click_release();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        use crate::session::tracker::TabState;

        let now = Instant::now();

        for (idx, ev) in self.app.poll_all(now) {
            self.handle_session_event(idx, ev);
        }
        for (idx, ev) in self.app.tick_all(now) {
            self.handle_session_event(idx, ev);
        }

        let any_waiting = self
            .app
            .tabs()
            .iter()
            .any(|tab| tab.state() == TabState::Waiting);

        let next_deadline = if any_waiting {
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
            now + Duration::from_millis(16)
        } else {
            // Cursor blinks at 1 Hz (500 ms toggle). Schedule a redraw at the
            // next blink boundary, capped at 100 ms so tracker timeouts still
            // tick at their usual cadence.
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
            now + Duration::from_millis(100)
        };

        event_loop.set_control_flow(ControlFlow::WaitUntil(next_deadline));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixels_to_grid_uses_floor_division() {
        // 960 / 8 = 120, 600 / 16 = 37 (with 8 px remainder ignored).
        assert_eq!(pixels_to_grid(960, 600, 8, 16), (37, 120));
    }

    #[test]
    fn pixels_to_grid_clamps_to_minimum_one_cell() {
        // A degenerate 0×0 surface should still produce 1×1 — terminal
        // children expect at least one row and one column.
        assert_eq!(pixels_to_grid(0, 0, 8, 16), (1, 1));
    }

    #[test]
    fn pixels_to_grid_handles_unusual_cell_sizes() {
        // Square cells, 100×100 surface.
        assert_eq!(pixels_to_grid(100, 100, 10, 10), (10, 10));
    }

    use winit::event::ElementState;
    use winit::keyboard::{Key, ModifiersState, NamedKey, SmolStr};

    #[test]
    fn key_to_bytes_printable_ascii() {
        assert_eq!(
            key_to_bytes(
                &Key::Character(SmolStr::new("a")),
                ElementState::Pressed,
                ModifiersState::empty()
            ),
            Some(b"a".to_vec())
        );
    }

    #[test]
    fn key_to_bytes_printable_unicode() {
        assert_eq!(
            key_to_bytes(
                &Key::Character(SmolStr::new("é")),
                ElementState::Pressed,
                ModifiersState::empty()
            ),
            Some("é".as_bytes().to_vec())
        );
    }

    #[test]
    fn key_to_bytes_enter_returns_carriage_return() {
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::Enter),
                ElementState::Pressed,
                ModifiersState::empty()
            ),
            Some(vec![b'\r'])
        );
    }

    #[test]
    fn key_to_bytes_backspace_returns_del() {
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::Backspace),
                ElementState::Pressed,
                ModifiersState::empty()
            ),
            Some(vec![0x7f])
        );
    }

    #[test]
    fn key_to_bytes_ctrl_c_returns_etx() {
        assert_eq!(
            key_to_bytes(
                &Key::Character(SmolStr::new("c")),
                ElementState::Pressed,
                ModifiersState::CONTROL
            ),
            Some(vec![0x03])
        );
    }

    #[test]
    fn key_to_bytes_ctrl_d_returns_eot() {
        assert_eq!(
            key_to_bytes(
                &Key::Character(SmolStr::new("d")),
                ElementState::Pressed,
                ModifiersState::CONTROL
            ),
            Some(vec![0x04])
        );
    }

    #[test]
    fn key_to_bytes_space_returns_space_byte() {
        // Regression: winit 0.30 routes the spacebar through NamedKey::Space,
        // not Character(" "). Without this arm the byte never reached the PTY.
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::Space),
                ElementState::Pressed,
                ModifiersState::empty()
            ),
            Some(vec![b' '])
        );
    }

    #[test]
    fn key_to_bytes_ignores_release_events() {
        assert_eq!(
            key_to_bytes(
                &Key::Character(SmolStr::new("a")),
                ElementState::Released,
                ModifiersState::empty()
            ),
            None
        );
    }

    #[test]
    fn key_to_bytes_ignores_unhandled_named_keys() {
        // F5 is not in Stage 4's subset; Stage 8 will handle it.
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::F5),
                ElementState::Pressed,
                ModifiersState::empty()
            ),
            None
        );
    }

    #[test]
    fn pixels_to_grid_with_real_jbm_metrics() {
        // JetBrains Mono Regular at 16 px: advance_width ≈ 9.6 px → ceil = 10,
        // line metrics' new_line_size ≈ 21.6 px → ceil = 22. Verify the math
        // works for that pitch (we don't hardcode the values here because they
        // depend on the font binary's hinting, but we sanity-check that
        // 800/10 = 80 columns, not 800/8 = 100.
        let (rows_jbm, cols_jbm) = pixels_to_grid(800, 480, 10, 22);
        assert_eq!(cols_jbm, 80);
        assert_eq!(rows_jbm, 21);

        // The Stage-4 placeholder pitch (8×16) would have given different math.
        // This contrast test makes the bug obvious if someone re-introduces
        // the placeholders.
        let (rows_placeholder, cols_placeholder) = pixels_to_grid(800, 480, 8, 16);
        assert_eq!(cols_placeholder, 100);
        assert_eq!(rows_placeholder, 30);
        assert_ne!(
            (rows_jbm, cols_jbm),
            (rows_placeholder, cols_placeholder),
            "real font metrics should produce different grid dims than the \
             Stage-4 placeholder 8×16 pitch — if these are equal, window.rs \
             is still using the placeholders"
        );
    }
}
