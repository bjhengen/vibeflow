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
use crate::render::tabs::RenameInputState;
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

/// Translate a pixel position to an alacritty terminal grid `Point`. Returns
/// `None` when the pixel is inside the tab bar (above the cell grid) or when
/// a zero-sized cell pitch would cause a divide-by-zero.
///
/// This is a SEPARATE helper from `pixels_to_grid`. That function returns
/// `(u16, u16)` for PTY resize; this one returns `Point` for mouse routing.
/// They have different semantics and must not be consolidated.
fn pixel_to_grid_point(
    cell_w: u32,
    cell_h: u32,
    bar_height_px: u32,
    px: u32,
    py: u32,
    display_offset: usize,
) -> Option<alacritty_terminal::index::Point> {
    use alacritty_terminal::index::{Column, Line, Point};
    if cell_w == 0 || cell_h == 0 {
        return None;
    }
    if py < bar_height_px {
        return None; // tab bar — selection is grid-only
    }
    let py_local = py - bar_height_px;
    let col = (px / cell_w) as usize;
    // Stage 12 lesson: the input path is the third parallel offset path — render adds
    // display_offset, input subtracts it, so the grid Point lands on the scrolled-up
    // row the user actually sees.
    let line = (py_local / cell_h) as i32 - display_offset as i32;
    Some(Point::new(Line(line), Column(col)))
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
        // Stage 13: Ctrl + arrow keys → xterm modifier code 5 sequences.
        // Guards require exactly Ctrl (no Shift/Alt/Super) so they don't shadow
        // future Ctrl+Shift or other combos.
        Key::Named(NamedKey::ArrowUp)
            if modifiers.contains(ModifiersState::CONTROL)
                && !modifiers.contains(ModifiersState::SHIFT)
                && !modifiers.contains(ModifiersState::ALT)
                && !modifiers.contains(ModifiersState::SUPER) =>
        {
            Some(b"\x1b[1;5A".to_vec())
        }
        Key::Named(NamedKey::ArrowDown)
            if modifiers.contains(ModifiersState::CONTROL)
                && !modifiers.contains(ModifiersState::SHIFT)
                && !modifiers.contains(ModifiersState::ALT)
                && !modifiers.contains(ModifiersState::SUPER) =>
        {
            Some(b"\x1b[1;5B".to_vec())
        }
        Key::Named(NamedKey::ArrowRight)
            if modifiers.contains(ModifiersState::CONTROL)
                && !modifiers.contains(ModifiersState::SHIFT)
                && !modifiers.contains(ModifiersState::ALT)
                && !modifiers.contains(ModifiersState::SUPER) =>
        {
            Some(b"\x1b[1;5C".to_vec())
        }
        Key::Named(NamedKey::ArrowLeft)
            if modifiers.contains(ModifiersState::CONTROL)
                && !modifiers.contains(ModifiersState::SHIFT)
                && !modifiers.contains(ModifiersState::ALT)
                && !modifiers.contains(ModifiersState::SUPER) =>
        {
            Some(b"\x1b[1;5D".to_vec())
        }
        // Stage 13: Shift + arrow keys → xterm modifier code 2 sequences.
        Key::Named(NamedKey::ArrowUp)
            if modifiers.contains(ModifiersState::SHIFT)
                && !modifiers.contains(ModifiersState::CONTROL)
                && !modifiers.contains(ModifiersState::ALT)
                && !modifiers.contains(ModifiersState::SUPER) =>
        {
            Some(b"\x1b[1;2A".to_vec())
        }
        Key::Named(NamedKey::ArrowDown)
            if modifiers.contains(ModifiersState::SHIFT)
                && !modifiers.contains(ModifiersState::CONTROL)
                && !modifiers.contains(ModifiersState::ALT)
                && !modifiers.contains(ModifiersState::SUPER) =>
        {
            Some(b"\x1b[1;2B".to_vec())
        }
        Key::Named(NamedKey::ArrowRight)
            if modifiers.contains(ModifiersState::SHIFT)
                && !modifiers.contains(ModifiersState::CONTROL)
                && !modifiers.contains(ModifiersState::ALT)
                && !modifiers.contains(ModifiersState::SUPER) =>
        {
            Some(b"\x1b[1;2C".to_vec())
        }
        Key::Named(NamedKey::ArrowLeft)
            if modifiers.contains(ModifiersState::SHIFT)
                && !modifiers.contains(ModifiersState::CONTROL)
                && !modifiers.contains(ModifiersState::ALT)
                && !modifiers.contains(ModifiersState::SUPER) =>
        {
            Some(b"\x1b[1;2D".to_vec())
        }
        // Plain (unmodified) arrow keys.
        Key::Named(NamedKey::ArrowUp) if modifiers.is_empty() => Some(b"\x1b[A".to_vec()),
        Key::Named(NamedKey::ArrowDown) if modifiers.is_empty() => Some(b"\x1b[B".to_vec()),
        Key::Named(NamedKey::ArrowRight) if modifiers.is_empty() => Some(b"\x1b[C".to_vec()),
        Key::Named(NamedKey::ArrowLeft) if modifiers.is_empty() => Some(b"\x1b[D".to_vec()),
        Key::Named(NamedKey::Home) if modifiers.is_empty() => Some(b"\x1b[H".to_vec()),
        Key::Named(NamedKey::End) if modifiers.is_empty() => Some(b"\x1b[F".to_vec()),
        Key::Named(NamedKey::PageUp) if modifiers.is_empty() => Some(b"\x1b[5~".to_vec()),
        Key::Named(NamedKey::PageDown) if modifiers.is_empty() => Some(b"\x1b[6~".to_vec()),
        Key::Named(NamedKey::Insert) if modifiers.is_empty() => Some(b"\x1b[2~".to_vec()),
        Key::Named(NamedKey::Delete) if modifiers.is_empty() => Some(b"\x1b[3~".to_vec()),
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
    /// System clipboard handle for Ctrl+Shift+C / Ctrl+Shift+V. `None` on
    /// systems without a display server (CI, headless containers).
    clipboard: Option<crate::clipboard::Clipboard>,
    /// Proxy for the file-watcher thread to ship `AppUserEvent` back to the
    /// main thread. Cloned and handed to the watcher in `resumed`.
    proxy: winit::event_loop::EventLoopProxy<crate::config::AppUserEvent>,
    /// Active shortcut table. Replaces the static Stage 8 lookup.
    shortcut_table: crate::keymap::ShortcutTable,
    /// Banner state for config errors (Stage 9). Empty until first reload reports errors.
    error_banner: crate::config::error_banner::ErrorBannerState,
    /// Path to the config file. Stored so the watcher can be respawned if needed.
    config_path: std::path::PathBuf,
    /// Stage 9: in-progress inline rename of a tab title. None when not renaming.
    rename_state: Option<RenameInputState>,
    /// Stage 10: open right-click context menu, if any. At most one open.
    context_menu: Option<crate::render::context_menu::ContextMenuState>,
    /// Stage 12: how many lines a single wheel detent scrolls. Mirrors
    /// `[scrollback] wheel_lines_per_detent`. Cached so `WindowEvent::MouseWheel`
    /// can scale without re-reading config.
    wheel_lines_per_detent: u32,
    /// Stage 12: cached `term.grid().screen_lines()` for the active session.
    /// Updated on `WindowEvent::Resized`. Used by Shift+PageUp/Down for
    /// half-page scroll math.
    last_grid_size_lines: usize,
    /// Stage 13: mirror of `Config.scrollback.snap_on_esc`. When false, Esc
    /// does NOT snap to bottom (only character-producing keys do).
    snap_on_esc: bool,
    /// Stage 13: bell behavior config cache.
    // Stage 13: bell_mode/bell_debounce updated by apply_config in T17.
    bell_mode: crate::config::BellMode,
    bell_debounce: std::time::Duration,
    last_bell_at: Option<std::time::Instant>,
    /// Stage 13: theme registry loaded at startup from
    /// ~/.config/vibeflow/themes/, refreshed on config reload (T17).
    theme_registry: crate::theme::registry::ThemeRegistry,
    /// Stage 13 follow-up: false until the first `RedrawRequested` reconciles
    /// the grid to the true `window.inner_size()`. `resumed()` sizes from
    /// `renderer.surface_size()`, which some compositors/VNC report as the
    /// requested size before the real window size is final; this one-shot
    /// reconcile corrects the grid before the first visible frame.
    initial_size_reconciled: bool,
}

impl WindowApp {
    fn activate_focused_menu_item(&mut self) {
        let Some(menu) = self.context_menu.take() else {
            return;
        };
        let Some(item) = menu.items.get(menu.focused) else {
            return;
        };
        if !item.enabled || matches!(item.kind, crate::render::context_menu::ItemKind::Separator) {
            // Re-arm: defensive — should never happen if focus invariants hold.
            return;
        }
        let action = item.action.clone();
        let target_idx = menu.target_idx;
        self.dispatch_menu_action(action, target_idx);
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    fn dispatch_menu_action(
        &mut self,
        action: crate::render::context_menu::MenuAction,
        target_idx: Option<usize>,
    ) {
        use crate::render::context_menu::MenuAction;
        match action {
            MenuAction::Shortcut(shortcut) => {
                // For tab-menu actions that target a specific tab, switch
                // active to it first so existing handlers (which key off
                // `App::active()`) operate against the right tab. The Stage 9
                // rename for tab N is the canonical case.
                if let Some(idx) = target_idx {
                    self.app.set_active(idx);
                }
                self.handle_shortcut(shortcut);
            }
            MenuAction::PastePrimary => {
                self.handle_paste_primary();
            }
            MenuAction::ClearBuffer => {
                let target = target_idx.unwrap_or_else(|| self.app.active());
                if let Some(s) = self.app.tabs_mut().get_mut(target) {
                    let _ = s.send_input(&[0x0c]);
                }
            }
            MenuAction::CloseOtherTabs => {
                let target = target_idx.unwrap_or_else(|| self.app.active());
                // Close from end to start so indices stay stable for `target`.
                let mut idx = self.app.tabs().len();
                while idx > 0 {
                    idx -= 1;
                    if idx != target {
                        self.app.close_tab(idx);
                    }
                }
                // After closing, the surviving tab is at index 0.
                if !self.app.tabs().is_empty() {
                    self.app.set_active(0);
                }
            }
            MenuAction::OpenConfig => {
                let path = self.config_path.clone();
                let _ = std::process::Command::new("xdg-open")
                    .arg(&path)
                    .spawn()
                    .map_err(|e| {
                        tracing::warn!("xdg-open {} failed: {e}", path.display());
                    });
            }
            MenuAction::ShowAbout => {
                // Wired in Task 5 of the About-feature plan. Until then, log + no-op so
                // the menu item visibly does nothing (rather than crashing on a non-
                // exhaustive match).
                tracing::debug!("MenuAction::ShowAbout dispatched (handler not yet wired)");
            }
            MenuAction::SetTheme(name) => {
                let target = target_idx.unwrap_or_else(|| self.app.active());
                if let Some(s) = self.app.tabs_mut().get_mut(target) {
                    s.set_theme(Some(name), &self.theme_registry);
                }
            }
        }
    }

    /// Build a `WindowApp` with no window and no tabs. Call
    /// `event_loop.run_app(&mut app)` to drive it.
    #[must_use]
    pub fn new(proxy: winit::event_loop::EventLoopProxy<crate::config::AppUserEvent>) -> Self {
        let clipboard = match crate::clipboard::Clipboard::new() {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::warn!("system clipboard unavailable: {e}");
                None
            }
        };
        let config_path = crate::config::default_path()
            .unwrap_or_else(|| std::path::PathBuf::from("./vibeflow-config.toml"));
        let (initial_config, initial_errors) = crate::config::Config::load(&config_path);
        let error_banner = crate::config::error_banner::ErrorBannerState::new(initial_errors);
        // Build the shortcut table from the loaded config so user-bound chords
        // are honored from the moment `new` returns. `apply_config` (in
        // `resumed`) re-applies once the renderer exists; renderer-dependent
        // settings (colors, blink, fonts) wait for that.
        let shortcut_table = build_shortcut_table(&initial_config.shortcuts);
        Self {
            window: None,
            renderer: None,
            app: App::new(),
            current_modifiers: ModifiersState::empty(),
            cursor_pos: None,
            clipboard,
            proxy,
            shortcut_table,
            error_banner,
            config_path,
            rename_state: None,
            context_menu: None,
            wheel_lines_per_detent: 3,
            last_grid_size_lines: 24,
            snap_on_esc: true,
            bell_mode: crate::config::BellMode::Visual,
            bell_debounce: std::time::Duration::from_millis(100),
            last_bell_at: None,
            theme_registry: crate::theme::registry::ThemeRegistry::load(
                dirs::config_dir()
                    .map(|d| d.join("vibeflow").join("themes"))
                    .unwrap_or_default(),
            ),
            initial_size_reconciled: false,
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
                // Stage 13: only react if this is the active tab.
                if idx != self.app.active() {
                    return;
                }
                // Debounce — drop bells closer than `bell_debounce`.
                let now = std::time::Instant::now();
                if let Some(last) = self.last_bell_at {
                    if now.saturating_duration_since(last) < self.bell_debounce {
                        return;
                    }
                }
                self.last_bell_at = Some(now);

                use crate::config::BellMode;
                match self.bell_mode {
                    BellMode::Silent => {}
                    BellMode::Visual => {
                        if let Some(renderer) = self.renderer.as_mut() {
                            renderer.note_bell();
                        }
                        if let Some(window) = self.window.as_ref() {
                            window.request_redraw();
                        }
                    }
                    BellMode::Audible => {
                        crate::render::bell::play_audible_bell();
                    }
                    BellMode::Both => {
                        if let Some(renderer) = self.renderer.as_mut() {
                            renderer.note_bell();
                        }
                        if let Some(window) = self.window.as_ref() {
                            window.request_redraw();
                        }
                        crate::render::bell::play_audible_bell();
                    }
                }
            }
            SessionEvent::Osc52ClipboardWrite { selection, text } => {
                use crate::session::osc::Osc52Selection;
                // self.clipboard is Option<Clipboard> (Stage 8 made it lazy
                // so vibeflow runs even when arboard init fails — e.g. SSH
                // without X forwarding). Skip silently if absent.
                let Some(clipboard) = self.clipboard.as_mut() else {
                    tracing::debug!(tab = idx, "OSC 52 write dropped: no clipboard");
                    return;
                };
                let want_clipboard =
                    matches!(selection, Osc52Selection::Clipboard | Osc52Selection::Both);
                let want_primary =
                    matches!(selection, Osc52Selection::Primary | Osc52Selection::Both);
                if want_clipboard {
                    if let Err(e) = clipboard.copy_clipboard_only(&text) {
                        tracing::warn!(
                            error = %e,
                            tab = idx,
                            "OSC 52 write to system clipboard failed"
                        );
                    }
                }
                if want_primary {
                    if let Err(e) = clipboard.copy_primary(&text) {
                        tracing::warn!(
                            error = %e,
                            tab = idx,
                            "OSC 52 write to primary selection failed"
                        );
                    }
                }
                // No redraw needed — clipboard writes are invisible to the grid.
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
                self.context_menu = None;
                self.app.set_active(idx);
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            TabBarHit::TabClose(idx) => {
                self.app.close_tab(idx);
                // Stage 10: dismiss context menu if one is open when a tab closes.
                if self.context_menu.is_some() {
                    self.context_menu = None;
                }
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            TabBarHit::None => {}
        }
    }

    fn handle_shortcut(&mut self, shortcut: crate::keymap::Shortcut) {
        use crate::keymap::Shortcut;
        match shortcut {
            Shortcut::NewTab => {
                // `App::new_tab` spawns + appends + sets active in one call.
                let argv = std::env::var("SHELL").unwrap_or_else(|_| "bash".to_string());
                if let Err(e) = self.app.new_tab(&[argv.as_str()]) {
                    tracing::warn!("new tab spawn failed: {e}");
                }
            }
            Shortcut::CloseTab => {
                self.app.close_tab(self.app.active());
                // Stage 10: dismiss context menu when a tab closes.
                if self.context_menu.is_some() {
                    self.context_menu = None;
                }
            }
            Shortcut::NextTab => self.app.cycle_active(1),
            Shortcut::PrevTab => self.app.cycle_active(-1),
            Shortcut::RestartTab => {
                if let Err(e) = self.app.restart_active() {
                    tracing::warn!("restart failed: {e}");
                }
                // Stage 13 FN2: re-resolve the (app-default) theme onto the
                // freshly-restarted session. Note: a per-tab theme override
                // selected via the context menu is intentionally NOT
                // preserved across restart — the restarted tab adopts the
                // current app default, consistent with how history_lines /
                // tools_list propagate (App::restart_active).
                let active = self.app.active();
                let theme_name = self.app.tabs().get(active).and_then(|s| s.theme.clone());
                if let Some(s) = self.app.tabs_mut().get_mut(active) {
                    s.set_theme(theme_name, &self.theme_registry);
                }
            }
            Shortcut::Copy => self.handle_copy(),
            Shortcut::Paste => self.handle_paste(),
            Shortcut::RenameTab => {
                self.start_rename(self.app.active());
            }
            Shortcut::SelectAll => {
                let active = self.app.active();
                let Some(s) = self.app.tabs_mut().get_mut(active) else {
                    return;
                };
                let (sel, term) = s.split_borrow_mouse();
                sel.select_all(term);
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
        }
    }

    fn handle_copy(&mut self) {
        let Some(clipboard) = self.clipboard.as_mut() else {
            return;
        };
        let active = self.app.active();
        let Some(s) = self.app.tabs().get(active) else {
            return;
        };
        let Some(text) = s.selection.text(s.term()) else {
            return;
        };
        if let Err(e) = clipboard.copy(&text) {
            tracing::warn!("copy failed: {e}");
        }
    }

    /// Distribute a newly-loaded config to all subscribers.
    fn apply_config(&mut self, config: &crate::config::Config) {
        if let Some(r) = self.renderer.as_mut() {
            r.set_selection_color(config.colors.selection);
            r.set_indicator_colors([
                config.colors.indicator_active,
                config.colors.indicator_working,
                config.colors.indicator_waiting,
                config.colors.indicator_inactive,
            ]);
            r.set_cursor_blink_ms(config.cursor.blink_ms);
            r.set_font_priorities(&config.fonts.priority);
            r.set_menu_colors(crate::render::context_menu::MenuColors {
                bg: config.colors.menu_bg,
                border: config.colors.menu_border,
                text: config.colors.menu_text,
                text_disabled: config.colors.menu_text_disabled,
                shortcut: config.colors.menu_shortcut,
                focus_bg: config.colors.menu_focus_bg,
            });
        }
        // Rebuild the shortcut table from the bindings.
        self.shortcut_table = build_shortcut_table(&config.shortcuts);
        if let Some(c) = self.clipboard.as_mut() {
            c.set_primary_enabled(config.clipboard.primary);
        }
        // Propagate `respect_osc_title` + `title_strip_prefix` to all current
        // tabs and remember them so future `App::new_tab` spawns inherit.
        let respect = config.tabs.respect_osc_title;
        let prefix = config.tabs.title_strip_prefix.clone();
        for s in self.app.tabs_mut().iter_mut() {
            s.respect_osc_title = respect;
            s.title_strip_prefix = prefix.clone();
        }
        self.app.set_default_respect_osc_title(respect);
        self.app.set_default_title_strip_prefix(prefix);

        // Stage 11: [ai] section.
        let ai = &config.ai;
        let tracker_cfg = crate::session::tracker::TrackerConfig {
            debounce: std::time::Duration::from_millis(ai.debounce_ms),
            heuristic_silence: std::time::Duration::from_millis(ai.heuristic_silence_ms),
            stale_state: std::time::Duration::from_secs(ai.stale_state_timeout_s),
            explicit_stale_state: std::time::Duration::from_secs(ai.explicit_stale_state_s),
        };
        let proc_interval = std::time::Duration::from_millis(ai.foreground_check_interval_ms);
        self.app.set_default_tracker_config(tracker_cfg);
        self.app.set_default_tools_list(ai.tools.clone());
        self.app.set_default_proc_check_interval(proc_interval);
        for s in self.app.tabs_mut().iter_mut() {
            s.set_tracker_config(tracker_cfg);
            s.tools_list = ai.tools.clone();
            s.proc_check_interval = proc_interval;
        }

        // Stage 12: [scrollback] section.
        let sb = &config.scrollback;
        self.wheel_lines_per_detent = sb.wheel_lines_per_detent;
        let fade_ms = sb.scrollbar_fade_ms;
        self.app.set_default_scrollbar_fade_ms(fade_ms);
        self.app.set_default_history_lines(sb.history_lines);
        for s in self.app.tabs_mut().iter_mut() {
            s.scrollbar_fade.set_fade_ms(fade_ms);
        }
        // Stage 13 (T2 carryover): mirror snap_on_esc into the runtime cache
        // so the Esc-snap gate honors config reloads.
        self.snap_on_esc = sb.snap_on_esc;
        // Scrollbar colors (from [colors]).
        if let Some(r) = self.renderer.as_mut() {
            r.set_scrollbar_colors(crate::render::scrollbar::ScrollbarColors {
                track: config.colors.scrollbar_track,
                thumb: config.colors.scrollbar_thumb,
            });
        }
        // Stage 13 (T4 carryover): [bell] section → runtime cache.
        self.bell_mode = config.bell.mode;
        self.bell_debounce = std::time::Duration::from_millis(config.bell.debounce_ms);

        // Stage 13: theme preset. Reload the registry FIRST (so freshly
        // imported themes resolve), set the app default for new/restarted
        // tabs, then propagate to every existing tab.
        let new_preset = config.color_preset.clone();
        self.theme_registry.reload();
        self.app.set_default_theme(new_preset.clone());
        let tab_count = self.app.tabs().len();
        for i in 0..tab_count {
            let preset = new_preset.clone();
            if let Some(s) = self.app.tabs_mut().get_mut(i) {
                s.set_theme(preset, &self.theme_registry);
            }
        }
    }

    fn handle_paste(&mut self) {
        let Some(clipboard) = self.clipboard.as_mut() else {
            return;
        };
        let Some(text) = clipboard.paste() else {
            return;
        };
        let active = self.app.active();
        let Some(s) = self.app.tabs_mut().get_mut(active) else {
            return;
        };
        let bracketed = s
            .term()
            .mode()
            .contains(alacritty_terminal::term::TermMode::BRACKETED_PASTE);
        let sanitised = crate::clipboard::sanitise_paste(&text);
        if bracketed {
            let _ = s.send_input(b"\x1b[200~");
            let _ = s.send_input(sanitised.as_bytes());
            let _ = s.send_input(b"\x1b[201~");
        } else {
            let _ = s.send_input(sanitised.as_bytes());
        }
    }

    /// Paste the PRIMARY selection (X11 middle-click clipboard) into the active tab.
    /// Called by both the `MouseButton::Middle` arm and `MenuAction::PastePrimary`.
    fn handle_paste_primary(&mut self) {
        let active = self.app.active();
        let Some(s) = self.app.tabs_mut().get_mut(active) else {
            return;
        };
        let bracketed = s
            .term()
            .mode()
            .contains(alacritty_terminal::term::TermMode::BRACKETED_PASTE);
        if let Some(clipboard) = self.clipboard.as_mut() {
            if let Some(text) = clipboard.paste_primary() {
                let sanitised = crate::clipboard::sanitise_paste(&text);
                if bracketed {
                    let _ = s.send_input(b"\x1b[200~");
                    let _ = s.send_input(sanitised.as_bytes());
                    let _ = s.send_input(b"\x1b[201~");
                } else {
                    let _ = s.send_input(sanitised.as_bytes());
                }
            }
        }
    }

    fn start_rename(&mut self, tab_idx: usize) {
        let title = match self.app.tabs().get(tab_idx) {
            Some(s) => s.label().title.clone(),
            None => return,
        };
        self.rename_state = Some(RenameInputState {
            tab_idx,
            cursor_pos: title.len(),
            buffer: title.clone(),
            original: title,
        });
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    fn commit_rename(&mut self) {
        let Some(rs) = self.rename_state.take() else {
            return;
        };
        if let Some(s) = self.app.tabs_mut().get_mut(rs.tab_idx) {
            s.set_title(rs.buffer);
            s.user_renamed = true;
        }
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    fn cancel_rename(&mut self) {
        let Some(rs) = self.rename_state.take() else {
            return;
        };
        if let Some(s) = self.app.tabs_mut().get_mut(rs.tab_idx) {
            s.set_title(rs.original);
        }
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    /// Open a context menu anchored at (px_x, px_y). `target_idx` is `Some` for
    /// tab menus (set to the tab the user right-clicked) and `None` for grid
    /// menus (action targets the active tab).
    fn open_context_menu(&mut self, anchor: (f32, f32), target_idx: Option<usize>) {
        use crate::render::context_menu::{self, ContextMenuState, MenuFontMetrics, MenuLayout};

        // If a rename is in progress, commit it before opening the menu.
        if self.rename_state.is_some() {
            self.commit_rename();
        }

        // Build items based on context.
        let items = match target_idx {
            Some(idx) => {
                // PtySession exposes `is_alive()` (not `is_dead`). Negate.
                let is_dead = self
                    .app
                    .tabs()
                    .get(idx)
                    .map(|s| !s.is_alive())
                    .unwrap_or(true);
                let tab_count = self.app.tabs().len();
                let theme_names = self.theme_registry.names();
                context_menu::tab_menu(idx, is_dead, tab_count, &theme_names)
            }
            None => {
                let active = self.app.active();
                let has_selection = self
                    .app
                    .tabs()
                    .get(active)
                    .and_then(|s| s.selection.current())
                    .is_some();
                context_menu::grid_menu(has_selection)
            }
        };
        // Find the first enabled action for initial focus.
        let focused = items
            .iter()
            .position(|item| matches!(item.kind, context_menu::ItemKind::Action) && item.enabled)
            .unwrap_or(0);
        // Compute layout. Font metrics come from the renderer (cell metrics).
        // `Renderer::cell_pitch()` returns (cell_w, cell_h) in physical px.
        let (cell_w, cell_h) = self
            .renderer
            .as_ref()
            .map(|r| r.cell_pitch())
            .unwrap_or((8, 16));
        let font = MenuFontMetrics {
            item_height_px: cell_h as f32 + 4.0,
            char_width_px: cell_w as f32,
        };
        let window_size = self
            .window
            .as_ref()
            .map(|w| {
                let s = w.inner_size();
                (s.width as f32, s.height as f32)
            })
            .unwrap_or((1024.0, 768.0));
        let layout = MenuLayout::compute(&items, font, anchor, window_size);
        self.context_menu = Some(ContextMenuState {
            anchor,
            items,
            focused,
            target_idx,
            layout,
        });
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    fn handle_rename_keyboard(&mut self, key: &winit::keyboard::Key) {
        use winit::keyboard::{Key, NamedKey};

        enum RenameOutcome {
            None,
            Commit,
            Cancel,
        }

        let outcome = {
            let Some(rs) = self.rename_state.as_mut() else {
                return;
            };
            match key {
                Key::Named(NamedKey::Enter) => RenameOutcome::Commit,
                Key::Named(NamedKey::Escape) => RenameOutcome::Cancel,
                Key::Named(NamedKey::Backspace) => {
                    if rs.cursor_pos > 0 {
                        let new_pos = prev_grapheme(&rs.buffer, rs.cursor_pos);
                        rs.buffer.replace_range(new_pos..rs.cursor_pos, "");
                        rs.cursor_pos = new_pos;
                    }
                    RenameOutcome::None
                }
                Key::Named(NamedKey::Delete) => {
                    if rs.cursor_pos < rs.buffer.len() {
                        let new_end = next_grapheme(&rs.buffer, rs.cursor_pos);
                        rs.buffer.replace_range(rs.cursor_pos..new_end, "");
                    }
                    RenameOutcome::None
                }
                Key::Named(NamedKey::ArrowLeft) => {
                    if rs.cursor_pos > 0 {
                        rs.cursor_pos = prev_grapheme(&rs.buffer, rs.cursor_pos);
                    }
                    RenameOutcome::None
                }
                Key::Named(NamedKey::ArrowRight) => {
                    if rs.cursor_pos < rs.buffer.len() {
                        rs.cursor_pos = next_grapheme(&rs.buffer, rs.cursor_pos);
                    }
                    RenameOutcome::None
                }
                Key::Named(NamedKey::Home) => {
                    rs.cursor_pos = 0;
                    RenameOutcome::None
                }
                Key::Named(NamedKey::End) => {
                    rs.cursor_pos = rs.buffer.len();
                    RenameOutcome::None
                }
                Key::Named(NamedKey::Space) => {
                    // winit routes spacebar through `Named(Space)`, not
                    // `Character(" ")` — without this arm the space is dropped.
                    rs.buffer.insert(rs.cursor_pos, ' ');
                    rs.cursor_pos += 1;
                    RenameOutcome::None
                }
                Key::Character(c) => {
                    let s = c.as_str();
                    rs.buffer.insert_str(rs.cursor_pos, s);
                    rs.cursor_pos += s.len();
                    RenameOutcome::None
                }
                _ => RenameOutcome::None,
            }
        };
        match outcome {
            RenameOutcome::Commit => self.commit_rename(),
            RenameOutcome::Cancel => self.cancel_rename(),
            RenameOutcome::None => {}
        }
    }
}

impl ApplicationHandler<crate::config::AppUserEvent> for WindowApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            // resumed can fire more than once on some platforms (e.g. when the
            // app comes back from suspended). For Stage 4 we only construct the
            // window the first time.
            return;
        }
        let icon = crate::icon::load_icon();
        if icon.is_none() {
            tracing::warn!("embedded window icon failed to decode; falling back to OS default");
        }
        let window_attrs = Window::default_attributes()
            .with_title("vibeflow")
            .with_inner_size(winit::dpi::LogicalSize::new(960, 600))
            .with_window_icon(icon);
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

        // Apply initial config now that renderer is built.
        let (config, errors) = crate::config::Config::load(&self.config_path);
        self.apply_config(&config);
        self.error_banner.update(errors);

        // Start the file watcher.
        let proxy = self.proxy.clone();
        let path = self.config_path.clone();
        if let Err(e) = crate::config::watcher::spawn(path, proxy) {
            tracing::warn!(error = %e, "config watcher failed to start");
        }

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
                if !self.initial_size_reconciled {
                    self.initial_size_reconciled = true;
                    // Re-sync BOTH the wgpu surface and the PTY grid to the
                    // TRUE window size. `resumed()` sizes from
                    // `renderer.surface_size()`, which some compositors/VNC
                    // report as the requested size before the real window is
                    // mapped; `RedrawRequested` is the earliest safe point —
                    // by the first repaint the compositor has mapped the
                    // window and finalized its dimensions. Full mirror of the
                    // `WindowEvent::Resized` path (surface resize THEN grid
                    // resize); if either differed from `resumed()`'s guess,
                    // both must be corrected together or the surface and grid
                    // diverge (clipping) on the first frame.
                    // NOTE: three sites now share this resize math (resumed,
                    // Resized, here). If `tab_bar_height_px` / the reserve
                    // changes, update all three or extract a helper.
                    let size = self.window.as_ref().map(|w| w.inner_size());
                    if let (Some(size), Some(renderer)) = (size, self.renderer.as_mut()) {
                        renderer.resize(size.width, size.height);
                        let (cell_w, cell_h) = renderer.cell_pitch();
                        let bar_h = crate::render::tabs::tab_bar_height_px(cell_h);
                        let visible_h = size.height.saturating_sub(bar_h);
                        let (rows, cols) = pixels_to_grid(size.width, visible_h, cell_w, cell_h);
                        if let Err(e) = self.app.resize_all(rows, cols) {
                            tracing::warn!(error = %e, rows, cols, "initial reconcile resize failed");
                        }
                    }
                }
                let term = self.app.active_term();
                let Some(renderer) = self.renderer.as_mut() else {
                    return;
                };
                match renderer.render(
                    term,
                    &self.app,
                    &self.error_banner,
                    self.rename_state.as_ref(),
                    self.context_menu.as_ref(),
                ) {
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
                // Stage 10: a resize invalidates any open context menu (anchor
                // coordinates shift and the hit regions would be stale).
                if self.context_menu.is_some() {
                    self.context_menu = None;
                }
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
                // Any resize invalidates the current selection — grid coordinates
                // shift when the column count changes, so the selection range no
                // longer refers to the same visual characters.
                for tab in self.app.tabs_mut().iter_mut() {
                    tab.selection.clear();
                }
                // Stage 12: cache for half-page scroll math.
                if let Some(s) = self.app.tabs().get(self.app.active()) {
                    use alacritty_terminal::grid::Dimensions;
                    self.last_grid_size_lines = s.term().grid().screen_lines();
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
                if event.state != ElementState::Pressed {
                    return;
                }
                // Stage 10: if a context menu is open, it gets first crack at
                // keyboard input. Arrow keys navigate, Enter activates, Escape
                // closes, bare modifier presses keep the menu alive, and any
                // other typed key closes the menu and falls through to the grid.
                if self.context_menu.is_some() {
                    use winit::keyboard::{Key, NamedKey};
                    if event.state == ElementState::Pressed {
                        match &event.logical_key {
                            Key::Named(NamedKey::ArrowDown) => {
                                if let Some(menu) = self.context_menu.as_mut() {
                                    menu.focus_next();
                                }
                                if let Some(window) = self.window.as_ref() {
                                    window.request_redraw();
                                }
                                return;
                            }
                            Key::Named(NamedKey::ArrowUp) => {
                                if let Some(menu) = self.context_menu.as_mut() {
                                    menu.focus_prev();
                                }
                                if let Some(window) = self.window.as_ref() {
                                    window.request_redraw();
                                }
                                return;
                            }
                            Key::Named(NamedKey::Enter) => {
                                self.activate_focused_menu_item();
                                return;
                            }
                            Key::Named(NamedKey::Escape) => {
                                self.context_menu = None;
                                if let Some(window) = self.window.as_ref() {
                                    window.request_redraw();
                                }
                                return;
                            }
                            // Modifier-only presses keep the menu alive (per
                            // Stage 8 lesson: bare modifiers are key events
                            // too). Detect by checking that the key is one of
                            // the modifier NamedKeys.
                            Key::Named(
                                NamedKey::Control
                                | NamedKey::Shift
                                | NamedKey::Alt
                                | NamedKey::Super
                                | NamedKey::Meta,
                            ) => {
                                // Don't close on modifier-only press.
                            }
                            _ => {
                                // Any other typed key: close, then fall
                                // through to normal handling so the keystroke
                                // reaches the grid.
                                self.context_menu = None;
                            }
                        }
                    }
                }
                // Stage 9: while renaming a tab, capture all keystrokes.
                if event.state == ElementState::Pressed && self.rename_state.is_some() {
                    self.handle_rename_keyboard(&event.logical_key);
                    if let Some(window) = self.window.as_ref() {
                        window.request_redraw();
                    }
                    return;
                }
                // Esc dismisses the config-error banner (Stage 9) before any
                // other handling so the Escape byte is not forwarded to the PTY
                // while the banner is visible.
                if matches!(
                    event.logical_key,
                    winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
                ) && self.error_banner.visible()
                {
                    self.error_banner.dismiss();
                    if let Some(window) = self.window.as_ref() {
                        window.request_redraw();
                    }
                    return;
                }
                // Shortcut dispatch FIRST. If the combo matches, suppress the
                // literal byte fallthrough.
                if let Some(shortcut) = self
                    .shortcut_table
                    .lookup(&event.logical_key, self.current_modifiers)
                {
                    self.handle_shortcut(shortcut);
                    return;
                }
                // Stage 12: scrollback keyboard chords. These go AFTER Stage
                // 10's menu intercept and AFTER Stage 9's rename handler (both
                // return early above), but BEFORE key_to_bytes so chord keys
                // are not double-handled as PTY bytes.
                //
                // Plain (no-modifier) PageUp/PageDown still fall through to
                // key_to_bytes and emit \x1b[5~ / \x1b[6~ (Stage 8 behavior).
                {
                    let mods = self.current_modifiers;
                    let shift = mods.shift_key();
                    let ctrl = mods.control_key();

                    let active_idx = self.app.active();
                    if let Some(s) = self.app.tabs_mut().get_mut(active_idx) {
                        let now = Instant::now();
                        match &event.logical_key {
                            Key::Named(NamedKey::PageUp) if shift => {
                                let half = (self.last_grid_size_lines / 2).max(1) as i32;
                                s.scroll_by(-half, now);
                                if let Some(w) = self.window.as_ref() {
                                    w.request_redraw();
                                }
                                return;
                            }
                            Key::Named(NamedKey::PageDown) if shift => {
                                let half = (self.last_grid_size_lines / 2).max(1) as i32;
                                s.scroll_by(half, now);
                                if let Some(w) = self.window.as_ref() {
                                    w.request_redraw();
                                }
                                return;
                            }
                            Key::Named(NamedKey::Home) if ctrl => {
                                s.scroll_to_top(now);
                                if let Some(w) = self.window.as_ref() {
                                    w.request_redraw();
                                }
                                return;
                            }
                            Key::Named(NamedKey::End) if ctrl => {
                                s.scroll_to_bottom(now);
                                if let Some(w) = self.window.as_ref() {
                                    w.request_redraw();
                                }
                                return;
                            }
                            _ => {}
                        }
                    }
                }
                // Otherwise: typed-input fallthrough. Selection clears only
                // when a key actually produces PTY bytes — bare modifier
                // presses (Ctrl, Shift, Alt, Super) must NOT clear the
                // selection, or shortcuts like Ctrl+Shift+C never see the
                // selection their action depends on (the user presses Ctrl
                // first, then Shift, then C, and we only want the selection
                // to survive long enough for the C event to read it).
                if let Some(bytes) =
                    key_to_bytes(&event.logical_key, event.state, self.current_modifiers)
                {
                    tracing::trace!(?bytes, "key → pty bytes");
                    let active = self.app.active();
                    if let Some(s) = self.app.tabs_mut().get_mut(active) {
                        s.selection.clear();
                        if let Err(e) = s.send_input(&bytes) {
                            tracing::warn!(error = %e, "send_input failed");
                        }
                    }
                    // Stage 12: any input-producing key snaps to bottom of
                    // scrollback. This only runs when key_to_bytes returns
                    // Some — bare modifier presses (Ctrl alone, Shift alone)
                    // never reach here, per Stage 8 lesson.
                    // Stage 13: when snap_on_esc is false, skip the snap for
                    // Esc specifically. All other input-producing keys still snap.
                    let is_esc = matches!(&event.logical_key, Key::Named(NamedKey::Escape));
                    if !is_esc || self.snap_on_esc {
                        let active_idx = self.app.active();
                        if let Some(s) = self.app.tabs_mut().get_mut(active_idx) {
                            if s.display_offset() > 0 {
                                s.scroll_to_bottom(Instant::now());
                                if let Some(w) = self.window.as_ref() {
                                    w.request_redraw();
                                }
                            }
                        }
                    }
                } else {
                    tracing::trace!(
                        logical_key = ?event.logical_key,
                        "press dropped (no key_to_bytes mapping)"
                    );
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let (px, py) = (position.x as u32, position.y as u32);
                self.cursor_pos = Some((px, py));

                // Stage 10: update hover focus when a context menu is open.
                if let Some(menu) = self.context_menu.as_mut() {
                    let cursor = (position.x as f32, position.y as f32);
                    if let crate::render::context_menu::HitRegion::Inside(idx) =
                        menu.layout.hit_test(cursor)
                    {
                        if matches!(
                            menu.items[idx].kind,
                            crate::render::context_menu::ItemKind::Action
                        ) && menu.items[idx].enabled
                            && menu.focused != idx
                        {
                            menu.focused = idx;
                            if let Some(window) = self.window.as_ref() {
                                window.request_redraw();
                            }
                        }
                    }
                }

                let Some(renderer) = self.renderer.as_ref() else {
                    return;
                };
                let (cell_w, cell_h) = renderer.cell_pitch();
                let bar_h = crate::render::tabs::tab_bar_height_px(cell_h);

                if py < bar_h {
                    return; // tab bar — no drag tracking
                }
                // Fetch display_offset via an immutable tabs() borrow BEFORE tabs_mut() below —
                // and before pixel_to_grid_point — so selection lands on the scrolled-up row.
                // Do not move this down next to tabs_mut().
                let active = self.app.active();
                let display_offset = self
                    .app
                    .tabs()
                    .get(active)
                    .map(|s| s.display_offset())
                    .unwrap_or(0);
                let Some(point) =
                    pixel_to_grid_point(cell_w, cell_h, bar_h, px, py, display_offset)
                else {
                    return;
                };
                let shift = self.current_modifiers.shift_key();

                let Some(s) = self.app.tabs_mut().get_mut(active) else {
                    return;
                };

                let mode_on = s.term().mode().intersects(
                    alacritty_terminal::term::TermMode::MOUSE_REPORT_CLICK
                        | alacritty_terminal::term::TermMode::MOUSE_DRAG
                        | alacritty_terminal::term::TermMode::MOUSE_MOTION,
                );
                let sgr = s
                    .term()
                    .mode()
                    .contains(alacritty_terminal::term::TermMode::SGR_MOUSE);
                let drag_tracking = s.term().mode().intersects(
                    alacritty_terminal::term::TermMode::MOUSE_DRAG
                        | alacritty_terminal::term::TermMode::MOUSE_MOTION,
                );

                if mode_on && drag_tracking && !shift {
                    // Same stale-drag guard as MouseInput — clear an in-progress
                    // selection if mouse mode kicked in mid-drag.
                    if s.selection.is_dragging() {
                        s.selection.clear();
                    }
                    let bytes = crate::render::mouse_encoder::encode_drag(
                        crate::render::mouse_encoder::Button::Left,
                        point,
                        sgr,
                    );
                    let _ = s.send_input(&bytes);
                } else if s.selection.is_dragging() {
                    let (sel, term) = s.split_borrow_mouse();
                    sel.mouse_drag(point, term);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                use winit::event::{ElementState, MouseButton};
                let Some((px, py)) = self.cursor_pos else {
                    return;
                };

                // Stage 10 fix: when a context menu is open, intercept ALL
                // left-button events FIRST — before tab-strip or grid routing.
                // - Pressed: consume; do not propagate to selection.mouse_down
                //   (otherwise drag_anchor gets set and mouse-move drags after
                //   the menu dismisses, leaving the user in selection mode).
                // - Released: hit-test the menu; Inside(enabled+Action)
                //   activates; Outside dismisses; both consume the click.
                // The menu's bbox can span both the tab-strip and grid areas
                // (e.g., tab menus anchored within the tab strip), so this
                // branch MUST run before the `py < bar_h` split.
                if self.context_menu.is_some() && button == MouseButton::Left {
                    if state == ElementState::Pressed {
                        return;
                    }
                    if state == ElementState::Released {
                        let cursor = (px as f32, py as f32);
                        let menu = self.context_menu.as_ref().unwrap();
                        match menu.layout.hit_test(cursor) {
                            crate::render::context_menu::HitRegion::Inside(idx) => {
                                let item = &menu.items[idx];
                                if item.enabled
                                    && matches!(
                                        item.kind,
                                        crate::render::context_menu::ItemKind::Action
                                    )
                                {
                                    // Reuse the keyboard activation path with focused = clicked.
                                    if let Some(menu) = self.context_menu.as_mut() {
                                        menu.focused = idx;
                                    }
                                    self.activate_focused_menu_item();
                                }
                                // Disabled or separator: no-op (menu stays open).
                                return;
                            }
                            crate::render::context_menu::HitRegion::Outside => {
                                // Dismiss; consume the click.
                                self.context_menu = None;
                                if let Some(window) = self.window.as_ref() {
                                    window.request_redraw();
                                }
                                return;
                            }
                        }
                    }
                }

                // Resolve cell metrics.
                let Some(renderer) = self.renderer.as_ref() else {
                    return;
                };
                let (cell_w, cell_h) = renderer.cell_pitch();
                let bar_h = crate::render::tabs::tab_bar_height_px(cell_h);

                // Tab bar passthrough: route Released-Left (Stage 6 contract —
                // click to switch / close / new) to the existing handler. Press
                // in the tab bar and any non-Left buttons in the tab bar are
                // ignored (no-op).
                if py < bar_h {
                    // Stage 10: right-click anywhere in the tab bar opens a context menu.
                    // Hit-test determines whether to show a tab menu (click on a tab body)
                    // or a grid menu (click in the gutters / empty area of the tab bar).
                    if state == ElementState::Released && button == MouseButton::Right {
                        let anchor = (px as f32, py as f32);
                        let tab_idx = if let Some(r) = self.renderer.as_ref() {
                            let (window_w, _) = r.surface_size();
                            let (_, cell_h) = r.cell_pitch();
                            let layout = crate::render::tabs::TabBarLayout::compute(
                                window_w,
                                cell_h,
                                self.app.tabs().len(),
                            );
                            // Stage 9 used TabBarHit::TabBody; we mirror that pattern
                            // but only care about the index, not the hit variant.
                            if let crate::render::tabs::TabBarHit::TabBody(idx) =
                                layout.hit_test(px, py)
                            {
                                Some(idx)
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        match tab_idx {
                            Some(idx) => self.open_context_menu(anchor, Some(idx)),
                            None => {
                                // Click in tab-bar gutter → grid menu targeting active tab.
                                self.open_context_menu(anchor, None);
                            }
                        }
                        return; // consumed
                    }
                    // Existing Stage 8 release-left handler (preserve behavior):
                    if state == ElementState::Released && button == MouseButton::Left {
                        // Stage 9: clicking on a different tab while renaming cancels.
                        if self.rename_state.is_some() {
                            // We don't know which tab was clicked yet; cancel anyway —
                            // the existing handler will switch active tab.
                            self.cancel_rename();
                        }
                        self.handle_left_click_release();
                    }
                    return;
                }

                // Below the tab bar: cell-grid mouse routing.
                // Fetch display_offset via an immutable tabs() borrow BEFORE tabs_mut() below —
                // and before pixel_to_grid_point — so selection lands on the scrolled-up row.
                // Do not move this down next to tabs_mut().
                let active = self.app.active();
                let display_offset = self
                    .app
                    .tabs()
                    .get(active)
                    .map(|s| s.display_offset())
                    .unwrap_or(0);
                let Some(point) =
                    pixel_to_grid_point(cell_w, cell_h, bar_h, px, py, display_offset)
                else {
                    return;
                };
                let pressed = state == ElementState::Pressed;
                let released = state == ElementState::Released;
                let shift = self.current_modifiers.shift_key();

                // Stage 9: click in cell area cancels in-progress rename.
                if pressed && self.rename_state.is_some() {
                    self.cancel_rename();
                    // Fall through to selection / mouse-mode logic.
                }

                // Stage 10: right-click in the grid area opens the grid context menu.
                // We consume this before any mouse-mode routing so the right-click
                // does not get forwarded to the PTY as a mouse event.
                if released && button == MouseButton::Right {
                    let anchor = (px as f32, py as f32);
                    self.open_context_menu(anchor, None);
                    return; // consumed
                }

                let Some(s) = self.app.tabs_mut().get_mut(active) else {
                    return;
                };

                let mode_on = s.term().mode().intersects(
                    alacritty_terminal::term::TermMode::MOUSE_REPORT_CLICK
                        | alacritty_terminal::term::TermMode::MOUSE_DRAG
                        | alacritty_terminal::term::TermMode::MOUSE_MOTION,
                );
                let sgr = s
                    .term()
                    .mode()
                    .contains(alacritty_terminal::term::TermMode::SGR_MOUSE);
                let encoder_button = match button {
                    MouseButton::Left => Some(crate::render::mouse_encoder::Button::Left),
                    MouseButton::Middle => Some(crate::render::mouse_encoder::Button::Middle),
                    MouseButton::Right => Some(crate::render::mouse_encoder::Button::Right),
                    _ => None,
                };

                // Stage 9: middle-click in NON-mouse-mode pastes PRIMARY.
                // Let `s` go out of scope so `handle_paste_primary` can take `&mut self`.
                if button == MouseButton::Middle && released && !mode_on {
                    let _ = s; // end the borrow
                    self.handle_paste_primary();
                    return;
                }

                if mode_on && !shift {
                    // If a selection drag was started before mouse mode engaged,
                    // discard it so it doesn't haunt us when mouse mode toggles back off.
                    if s.selection.is_dragging() {
                        s.selection.clear();
                    }
                    // Pass to PTY as encoded mouse event.
                    if let Some(b) = encoder_button {
                        let bytes = if pressed {
                            crate::render::mouse_encoder::encode_press(b, point, sgr)
                        } else if released {
                            crate::render::mouse_encoder::encode_release(b, point, sgr)
                        } else {
                            return;
                        };
                        let _ = s.send_input(&bytes);
                    }
                    return;
                }

                // Selection path — only Left button creates / clears selection.
                // Right and Middle buttons are no-ops in the selection world.
                if button != MouseButton::Left {
                    return;
                }
                if pressed {
                    let now = std::time::Instant::now();
                    let (sel, term) = s.split_borrow_mouse();
                    sel.mouse_down(point, shift, self.current_modifiers.alt_key(), term, now);
                } else if released {
                    s.selection.mouse_up();
                    // Stage 9: if a finalized selection exists AND PRIMARY is
                    // enabled, auto-copy to PRIMARY (X11 middle-click semantic).
                    // CLIPBOARD is unaffected — Ctrl+Shift+C still copies there.
                    if let Some(text) = s.selection.text(s.term()) {
                        if let Some(clipboard) = self.clipboard.as_mut() {
                            #[cfg(target_os = "linux")]
                            if clipboard.primary_enabled() {
                                let _ = clipboard.copy_primary(&text);
                            }
                        }
                    }
                }
            }
            // Stage 10: losing focus dismisses the context menu to avoid a
            // stale overlay. The Focused arm didn't exist before Stage 10 so
            // this is a new arm (not a modification of an existing handler).
            WindowEvent::Focused(false) if self.context_menu.is_some() => {
                self.context_menu = None;
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                use alacritty_terminal::term::TermMode;
                use winit::event::MouseScrollDelta;
                let active_idx = self.app.active();
                let Some(s) = self.app.tabs_mut().get_mut(active_idx) else {
                    return;
                };
                let now = Instant::now();

                // Stage 8: if mouse mode is on, encode wheel as mouse button press.
                let mouse_mode = s.term().mode().intersects(
                    TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION,
                );
                if mouse_mode {
                    // Compute cursor point in grid coordinates.
                    let cursor_point = self
                        .cursor_pos
                        .and_then(|(px, py)| {
                            let (cell_w, cell_h) = self
                                .renderer
                                .as_ref()
                                .map(|r| r.cell_pitch())
                                .unwrap_or((8, 16));
                            let bar_h = crate::render::tabs::tab_bar_height_px(cell_h);
                            pixel_to_grid_point(cell_w, cell_h, bar_h, px, py, 0 /* mouse-report: cursor coords must be viewport-relative (the TUI defines row 0); the vibeflow scrollback offset is intentionally not reported to the app */)
                        })
                        .unwrap_or_else(|| {
                            alacritty_terminal::index::Point::new(
                                alacritty_terminal::index::Line(0),
                                alacritty_terminal::index::Column(0),
                            )
                        });

                    let button = match delta {
                        MouseScrollDelta::LineDelta(_, y) if y > 0.0 => {
                            crate::render::mouse_encoder::Button::WheelUp
                        }
                        MouseScrollDelta::LineDelta(_, _) => {
                            crate::render::mouse_encoder::Button::WheelDown
                        }
                        MouseScrollDelta::PixelDelta(p) if p.y > 0.0 => {
                            crate::render::mouse_encoder::Button::WheelUp
                        }
                        MouseScrollDelta::PixelDelta(_) => {
                            crate::render::mouse_encoder::Button::WheelDown
                        }
                    };
                    let sgr = s.term().mode().intersects(TermMode::SGR_MOUSE);
                    let bytes =
                        crate::render::mouse_encoder::encode_press(button, cursor_point, sgr);
                    let _ = s.send_input(&bytes);
                } else {
                    // Plain shell: vibeflow scrollback.
                    let lines_raw = match delta {
                        MouseScrollDelta::LineDelta(_, y) => -(y.round() as i32),
                        MouseScrollDelta::PixelDelta(p) => {
                            let cell_h_f = self
                                .renderer
                                .as_ref()
                                .map(|r| r.cell_pitch().1 as f64)
                                .unwrap_or(16.0);
                            -((p.y / cell_h_f).round() as i32)
                        }
                    };
                    let lines = lines_raw * (self.wheel_lines_per_detent as i32);
                    s.scroll_by(lines, now);
                }
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
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

    fn user_event(
        &mut self,
        _event_loop: &winit::event_loop::ActiveEventLoop,
        event: crate::config::AppUserEvent,
    ) {
        match event {
            crate::config::AppUserEvent::ConfigReloaded { config, errors } => {
                tracing::info!(error_count = errors.len(), "config reloaded");
                self.apply_config(&config);
                self.error_banner.update(errors);
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            crate::config::AppUserEvent::ConfigError(err) => {
                tracing::warn!(?err, "config error");
                self.error_banner.update(vec![err]);
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
        }
    }
}

fn prev_grapheme(s: &str, pos: usize) -> usize {
    use unicode_segmentation::UnicodeSegmentation;
    s.grapheme_indices(true)
        .map(|(i, _)| i)
        .rfind(|&i| i < pos)
        .unwrap_or(0)
}

fn next_grapheme(s: &str, pos: usize) -> usize {
    use unicode_segmentation::UnicodeSegmentation;
    s.grapheme_indices(true)
        .map(|(i, _)| i)
        .find(|&i| i > pos)
        .unwrap_or(s.len())
}

fn build_shortcut_table(
    bindings: &crate::config::ShortcutBindings,
) -> crate::keymap::ShortcutTable {
    // For now, when the user supplies a config, replace the default table
    // wholesale. The default still applies if a Shortcut variant has no
    // entry in `bindings`.
    let mut table = crate::keymap::ShortcutTable::with_default_bindings();
    table.replace_from_bindings(bindings);
    table
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

    #[test]
    fn key_to_bytes_arrow_up_emits_csi_a() {
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::ArrowUp),
                ElementState::Pressed,
                ModifiersState::empty()
            ),
            Some(b"\x1b[A".to_vec())
        );
    }

    #[test]
    fn key_to_bytes_arrow_down_emits_csi_b() {
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::ArrowDown),
                ElementState::Pressed,
                ModifiersState::empty()
            ),
            Some(b"\x1b[B".to_vec())
        );
    }

    #[test]
    fn key_to_bytes_arrow_right_emits_csi_c() {
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::ArrowRight),
                ElementState::Pressed,
                ModifiersState::empty()
            ),
            Some(b"\x1b[C".to_vec())
        );
    }

    #[test]
    fn key_to_bytes_arrow_left_emits_csi_d() {
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::ArrowLeft),
                ElementState::Pressed,
                ModifiersState::empty()
            ),
            Some(b"\x1b[D".to_vec())
        );
    }

    #[test]
    fn key_to_bytes_home_emits_csi_h() {
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::Home),
                ElementState::Pressed,
                ModifiersState::empty()
            ),
            Some(b"\x1b[H".to_vec())
        );
    }

    #[test]
    fn key_to_bytes_end_emits_csi_f() {
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::End),
                ElementState::Pressed,
                ModifiersState::empty()
            ),
            Some(b"\x1b[F".to_vec())
        );
    }

    #[test]
    fn key_to_bytes_page_up_emits_csi_5_tilde() {
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::PageUp),
                ElementState::Pressed,
                ModifiersState::empty()
            ),
            Some(b"\x1b[5~".to_vec())
        );
    }

    #[test]
    fn key_to_bytes_page_down_emits_csi_6_tilde() {
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::PageDown),
                ElementState::Pressed,
                ModifiersState::empty()
            ),
            Some(b"\x1b[6~".to_vec())
        );
    }

    #[test]
    fn key_to_bytes_insert_emits_csi_2_tilde() {
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::Insert),
                ElementState::Pressed,
                ModifiersState::empty()
            ),
            Some(b"\x1b[2~".to_vec())
        );
    }

    #[test]
    fn key_to_bytes_delete_emits_csi_3_tilde() {
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::Delete),
                ElementState::Pressed,
                ModifiersState::empty()
            ),
            Some(b"\x1b[3~".to_vec())
        );
    }

    fn rename_state_init(buffer: &str, cursor: usize) -> crate::render::tabs::RenameInputState {
        crate::render::tabs::RenameInputState {
            tab_idx: 0,
            buffer: buffer.to_string(),
            cursor_pos: cursor,
            original: buffer.to_string(),
        }
    }

    #[test]
    fn rename_backspace_deletes_grapheme() {
        let mut rs = rename_state_init("hello", 5);
        let new_pos = prev_grapheme(&rs.buffer, rs.cursor_pos);
        rs.buffer.replace_range(new_pos..rs.cursor_pos, "");
        rs.cursor_pos = new_pos;
        assert_eq!(rs.buffer, "hell");
        assert_eq!(rs.cursor_pos, 4);
    }

    #[test]
    fn rename_backspace_handles_multibyte() {
        let mut rs = rename_state_init("café", 5);
        let new_pos = prev_grapheme(&rs.buffer, rs.cursor_pos);
        rs.buffer.replace_range(new_pos..rs.cursor_pos, "");
        rs.cursor_pos = new_pos;
        assert_eq!(rs.buffer, "caf");
        assert_eq!(rs.cursor_pos, 3);
    }

    #[test]
    fn rename_arrow_left_moves_by_grapheme() {
        let mut rs = rename_state_init("abc", 3);
        rs.cursor_pos = prev_grapheme(&rs.buffer, rs.cursor_pos);
        assert_eq!(rs.cursor_pos, 2);
    }

    #[test]
    fn rename_home_jumps_to_zero() {
        let mut rs = rename_state_init("abc", 2);
        rs.cursor_pos = 0;
        assert_eq!(rs.cursor_pos, 0);
    }

    #[test]
    fn rename_end_jumps_to_len() {
        let mut rs = rename_state_init("abc", 0);
        rs.cursor_pos = rs.buffer.len();
        assert_eq!(rs.cursor_pos, 3);
    }

    #[test]
    fn rename_insert_at_cursor() {
        let mut rs = rename_state_init("ab", 1);
        rs.buffer.insert(rs.cursor_pos, 'X');
        rs.cursor_pos += 1;
        assert_eq!(rs.buffer, "aXb");
        assert_eq!(rs.cursor_pos, 2);
    }

    #[test]
    fn key_to_bytes_arrow_up_with_ctrl_emits_modifier_5() {
        // Stage 13: Ctrl+ArrowUp now emits the xterm modifier-5 sequence.
        // Previously this returned None as a placeholder "until Stage 10+";
        // T5 is that implementation — the placeholder behavior is now replaced.
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::ArrowUp),
                ElementState::Pressed,
                ModifiersState::CONTROL
            ),
            Some(b"\x1b[1;5A".to_vec())
        );
    }

    #[test]
    fn ctrl_arrows_emit_xterm_modifier_5_sequences() {
        let pressed = ElementState::Pressed;
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::ArrowLeft),
                pressed,
                ModifiersState::CONTROL
            ),
            Some(b"\x1b[1;5D".to_vec())
        );
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::ArrowRight),
                pressed,
                ModifiersState::CONTROL
            ),
            Some(b"\x1b[1;5C".to_vec())
        );
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::ArrowUp),
                pressed,
                ModifiersState::CONTROL
            ),
            Some(b"\x1b[1;5A".to_vec())
        );
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::ArrowDown),
                pressed,
                ModifiersState::CONTROL
            ),
            Some(b"\x1b[1;5B".to_vec())
        );
    }

    #[test]
    fn shift_arrows_emit_xterm_modifier_2_sequences() {
        let pressed = ElementState::Pressed;
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::ArrowLeft),
                pressed,
                ModifiersState::SHIFT
            ),
            Some(b"\x1b[1;2D".to_vec())
        );
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::ArrowRight),
                pressed,
                ModifiersState::SHIFT
            ),
            Some(b"\x1b[1;2C".to_vec())
        );
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::ArrowUp),
                pressed,
                ModifiersState::SHIFT
            ),
            Some(b"\x1b[1;2A".to_vec())
        );
        assert_eq!(
            key_to_bytes(
                &Key::Named(NamedKey::ArrowDown),
                pressed,
                ModifiersState::SHIFT
            ),
            Some(b"\x1b[1;2B".to_vec())
        );
    }

    #[test]
    fn plain_arrows_still_emit_unmodified_sequences() {
        let pressed = ElementState::Pressed;
        let none = ModifiersState::empty();
        assert_eq!(
            key_to_bytes(&Key::Named(NamedKey::ArrowUp), pressed, none),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            key_to_bytes(&Key::Named(NamedKey::ArrowDown), pressed, none),
            Some(b"\x1b[B".to_vec())
        );
        assert_eq!(
            key_to_bytes(&Key::Named(NamedKey::ArrowRight), pressed, none),
            Some(b"\x1b[C".to_vec())
        );
        assert_eq!(
            key_to_bytes(&Key::Named(NamedKey::ArrowLeft), pressed, none),
            Some(b"\x1b[D".to_vec())
        );
    }

    #[test]
    fn pixel_to_grid_point_subtracts_scrollback_offset() {
        use alacritty_terminal::index::{Column, Line, Point};
        // cell 10x20, bar 30. Click at py=30+2*20=70 -> screen row 2.
        // offset 0 -> Line(2); offset 5 -> Line(2-5) = Line(-3) (scrollback).
        assert_eq!(
            super::pixel_to_grid_point(10, 20, 30, 5, 70, 0),
            Some(Point::new(Line(2), Column(0)))
        );
        assert_eq!(
            super::pixel_to_grid_point(10, 20, 30, 5, 70, 1),
            Some(Point::new(Line(1), Column(0)))
        );
        assert_eq!(
            super::pixel_to_grid_point(10, 20, 30, 5, 70, 5),
            Some(Point::new(Line(-3), Column(0)))
        );
    }

    #[test]
    fn reconcile_recomputes_grid_for_true_window_size() {
        // resumed() may have sized for the requested 960x600; the real
        // window is larger. The reconcile must recompute via pixels_to_grid
        // on the true size (tab-bar strip reserved) and yield more rows/cols.
        let cell_w = 10u32;
        let cell_h = 20u32;
        let bar_h = crate::render::tabs::tab_bar_height_px(cell_h);
        let requested = pixels_to_grid(960, 600u32.saturating_sub(bar_h), cell_w, cell_h);
        let actual = pixels_to_grid(1920, 1080u32.saturating_sub(bar_h), cell_w, cell_h);
        assert!(
            actual.0 > requested.0,
            "more rows on the larger real window"
        );
        assert!(
            actual.1 > requested.1,
            "more cols on the larger real window"
        );
    }
}
