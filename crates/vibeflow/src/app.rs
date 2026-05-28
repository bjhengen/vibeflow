//! `App` — single-threaded "central authority" that owns every tab's
//! [`PtySession`] and orchestrates polling and timeout ticks.

use crate::session::tracker::TrackerConfig;
use crate::session::{PtySession, SessionEvent};

/// v0.1.3 confirm-on-close: per-tab summary surfaced to the confirmation
/// dialog when the user attempts to close a vibeflow window. Snapshot
/// captured at dialog-open time; immutable thereafter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusyTabInfo {
    /// 1-based tab index for display ("Tab 1", "Tab 2", …).
    pub tab_index: usize,
    /// What to show in the dialog as the process / tool name.
    /// Priority: AI tool name (if Working/Waiting) > FG process comm > "process".
    pub display_label: String,
    /// What to show as the state. "Working" / "Waiting" for AI; "running"
    /// for non-AI foreground subprocesses. Elapsed-time deferred to v0.1.4.
    pub state_label: String,
}

/// Default per-tracker config used for every new tab. Stage 8 will replace
/// this with a TOML-loaded config sourced from `~/.config/vibeflow/config.toml`.
fn default_tracker_config() -> TrackerConfig {
    TrackerConfig::default()
}

/// v0.1.3: shared per-tab busy-detection. Returns `Some(BusyTabInfo)` when
/// the session has a foreground subprocess beyond the shell OR is in an
/// AI-busy tracker state. `idx_0based` becomes `tab_index = idx + 1` for
/// 1-based display.
fn busy_info_for(sess: &PtySession, idx_0based: usize) -> Option<BusyTabInfo> {
    use crate::session::tracker::TabState;
    let busy_fg = sess.has_foreground_child();
    let tracker = sess.tracker_state();
    let busy_ai = matches!(tracker, TabState::Working | TabState::Waiting);
    if !busy_fg && !busy_ai {
        return None;
    }
    // display_label priority: AI tool name first (most informative),
    // then FG process comm, then a generic fallback.
    let display_label = sess
        .detected_ai_tool()
        .filter(|_| busy_ai)
        .or_else(|| {
            if busy_fg {
                sess.child_pid()
                    .and_then(crate::session::proc_watch::foreground_command_name)
            } else {
                None
            }
        })
        .unwrap_or_else(|| "process".to_string());
    let state_label = if busy_ai {
        format!("{tracker:?}") // "Working" / "Waiting"
    } else {
        "running".to_string() // elapsed-time deferred to v0.1.4
    };
    Some(BusyTabInfo {
        tab_index: idx_0based + 1,
        display_label,
        state_label,
    })
}

/// Single-threaded central authority for the terminal app: owns every tab,
/// dispatches polls and ticks across them, tracks the focused tab.
pub struct App {
    tabs: Vec<PtySession>,
    active: usize,
    tracker_config: TrackerConfig,
    /// Mirror of `Config.tabs.respect_osc_title`. Applied to every new
    /// `PtySession` at spawn time so freshly-opened tabs honor the current
    /// config without WindowApp having to re-walk after each spawn.
    default_respect_osc_title: bool,
    /// Mirror of `Config.tabs.title_strip_prefix`. Same lifecycle as
    /// `default_respect_osc_title`.
    default_title_strip_prefix: String,
    /// Stage 11: mirror of `Config.ai.tools`. Applied to subsequently-spawned tabs.
    default_tools_list: Vec<String>,
    /// Stage 11: mirror of `Config.ai.foreground_check_interval_ms`. Applied
    /// to subsequently-spawned tabs.
    default_proc_check_interval: std::time::Duration,
    /// Stage 12: mirror of `Config.scrollback.scrollbar_fade_ms`. Applied to
    /// subsequently-spawned tabs AND to restarted sessions.
    default_scrollbar_fade_ms: u64,
    /// Stage 12: mirror of `Config.scrollback.history_lines`. Passed to
    /// `PtySession::spawn` to size each new session's scrollback buffer.
    /// Existing tabs are NOT retroactively resized (alacritty_terminal's
    /// grid is sized at construction).
    default_history_lines: u32,
    /// Stage 13: mirror of `Config.colors.preset`. Applied (as the theme
    /// NAME only) to subsequently-spawned tabs AND to restarted sessions.
    /// Color resolution happens in WindowApp, which owns the ThemeRegistry.
    default_theme: Option<String>,
}

impl App {
    /// Create an empty `App` with no tabs. Call [`Self::new_tab`] to spawn the first.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active: 0,
            tracker_config: default_tracker_config(),
            default_respect_osc_title: true,
            default_title_strip_prefix: String::new(),
            default_tools_list: Vec::new(),
            default_proc_check_interval: std::time::Duration::from_millis(250),
            default_scrollbar_fade_ms: 1500,
            default_history_lines: 10000,
            default_theme: None,
        }
    }

    /// Update the default OSC-title policy applied to subsequently-spawned tabs.
    /// `WindowApp::apply_config` calls this whenever the TOML config reloads.
    pub fn set_default_respect_osc_title(&mut self, respect: bool) {
        self.default_respect_osc_title = respect;
    }

    /// Update the default OSC-title prefix-strip applied to subsequently-spawned tabs.
    pub fn set_default_title_strip_prefix(&mut self, prefix: String) {
        self.default_title_strip_prefix = prefix;
    }

    /// Stage 11: update the default `TrackerConfig` for subsequently-spawned tabs.
    /// Existing tabs keep their current config until `set_tracker_config` is
    /// called explicitly (typically via `apply_config`).
    pub fn set_default_tracker_config(&mut self, cfg: TrackerConfig) {
        self.tracker_config = cfg;
    }

    /// Stage 11: update the default AI tool list for subsequently-spawned tabs.
    pub fn set_default_tools_list(&mut self, tools: Vec<String>) {
        self.default_tools_list = tools;
    }

    /// Stage 11: update the default proc-check interval for subsequently-spawned tabs.
    pub fn set_default_proc_check_interval(&mut self, interval: std::time::Duration) {
        self.default_proc_check_interval = interval;
    }

    /// Stage 12: update the default scrollbar fade duration for subsequently-spawned tabs.
    pub fn set_default_scrollbar_fade_ms(&mut self, ms: u64) {
        self.default_scrollbar_fade_ms = ms;
    }

    /// Stage 12: update the default scrollback history size for subsequently-spawned tabs.
    /// Existing tabs keep their original size — alacritty_terminal's grid is sized at construction.
    pub fn set_default_history_lines(&mut self, n: u32) {
        self.default_history_lines = n.max(1);
    }

    /// Stage 13: update the default theme name for subsequently-spawned tabs
    /// and restarted sessions. WindowApp::apply_config calls this on reload.
    pub fn set_default_theme(&mut self, name: Option<String>) {
        self.default_theme = name;
    }

    /// Spawn a new tab. Returns the index of the new tab in [`Self::tabs`]. The new
    /// tab becomes the active tab.
    ///
    /// # Errors
    /// Propagates any failure from [`PtySession::spawn`].
    pub fn new_tab(&mut self, argv: &[&str]) -> std::io::Result<usize> {
        let mut session = PtySession::spawn(
            argv,
            self.tracker_config,
            self.default_history_lines as usize,
        )?;
        session.respect_osc_title = self.default_respect_osc_title;
        session.title_strip_prefix = self.default_title_strip_prefix.clone();
        session.tools_list = self.default_tools_list.clone();
        session.proc_check_interval = self.default_proc_check_interval;
        session
            .scrollbar_fade
            .set_fade_ms(self.default_scrollbar_fade_ms);
        session.theme = self.default_theme.clone();
        self.tabs.push(session);
        let idx = self.tabs.len() - 1;
        self.active = idx;
        Ok(idx)
    }

    /// Close (and drop) the tab at `idx`. The session's `Drop` kills the child
    /// and joins the reader thread. Focus is preserved on whichever tab the user
    /// was looking at: if a tab to the left of `active` is closed, `active`
    /// shifts down to follow the still-focused element; if `active` is closed
    /// or `active` is now past the end, it clamps to the last remaining tab.
    pub fn close_tab(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        let _dropped = self.tabs.remove(idx);
        if idx < self.active {
            // Closed a tab to the left of the focused one: every tab from
            // `idx+1..` shifted down by one, so the focused index moves with it.
            self.active -= 1;
        } else if self.tabs.is_empty() {
            // Closed the only remaining tab — sentinel value. `app.tabs().get(0)`
            // returns None on an empty Vec, so this is harmless until Stage 8's
            // "no tabs → quit" logic lands.
            self.active = 0;
        } else if self.active >= self.tabs.len() {
            // Closed the last tab while it was focused (or beyond): clamp.
            self.active = self.tabs.len() - 1;
        }
    }

    /// Snapshot of all sessions (for read-only inspection — Stage 4+ tab-bar
    /// renderer uses this to draw indicator stripes).
    #[must_use]
    pub fn tabs(&self) -> &[PtySession] {
        &self.tabs
    }

    /// Drive every session's [`PtySession::poll`] at `now` and collect the
    /// resulting events with their tab index. Returned vector is in
    /// `(tab_index, event)` pairs ordered by tab; the caller can iterate and
    /// react.
    pub fn poll_all(&mut self, now: std::time::Instant) -> Vec<(usize, SessionEvent)> {
        let mut all = Vec::new();
        for (idx, tab) in self.tabs.iter_mut().enumerate() {
            for ev in tab.poll(now) {
                all.push((idx, ev));
            }
        }
        all
    }

    /// Run [`PtySession::tick`] on every session at `now` and collect any
    /// timeout-driven [`SessionEvent`]s with their tab index.
    pub fn tick_all(&mut self, now: std::time::Instant) -> Vec<(usize, SessionEvent)> {
        let mut all = Vec::new();
        for (idx, tab) in self.tabs.iter_mut().enumerate() {
            for ev in tab.tick(now) {
                all.push((idx, ev));
            }
        }
        all
    }

    /// Write keystroke bytes to the active tab's PTY child.
    ///
    /// # Errors
    /// Returns the tab's `io::Error` if the write fails. If there are no tabs,
    /// returns `ErrorKind::NotFound` — the caller should ensure at least one
    /// tab exists before calling.
    pub fn send_input(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no active tab",
            ));
        };
        tab.send_input(bytes)
    }

    /// Resize every tab's PTY to `rows × cols`. Called from `WindowApp` on
    /// every `WindowEvent::Resized` after the renderer surface is reconfigured.
    ///
    /// # Errors
    /// Returns the first per-tab `io::Error`; subsequent tabs are still resized
    /// best-effort. (We bias to applying the resize as broadly as possible
    /// because a single tab's resize failure shouldn't block the others — but
    /// we still surface the error so the caller can log it.)
    pub fn resize_all(&mut self, rows: u16, cols: u16) -> std::io::Result<()> {
        let mut first_error: Option<std::io::Error> = None;
        for tab in &mut self.tabs {
            if let Err(e) = tab.resize(rows, cols) {
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
        match first_error {
            None => Ok(()),
            Some(e) => Err(e),
        }
    }

    /// Read-only access to the active tab's [`alacritty_terminal::term::Term`]
    /// for rendering. Returns `None` if there are no tabs.
    #[must_use]
    pub fn active_term(
        &self,
    ) -> Option<&alacritty_terminal::term::Term<alacritty_terminal::event::VoidListener>> {
        self.tabs.get(self.active).map(|t| t.term())
    }

    /// v0.1.3 confirm-on-close: snapshot the list of "busy" tabs.
    /// A tab is busy when EITHER it has a foreground subprocess beyond the
    /// shell OR its AI tracker is in Working/Waiting state.
    ///
    /// Returns an empty vec when all tabs are idle. Caller decides what to do
    /// with that (single idle tab → silent close; multi-tab idle → confirm
    /// with a different message; etc.).
    pub fn busy_tabs(&self) -> Vec<BusyTabInfo> {
        self.tabs
            .iter()
            .enumerate()
            .filter_map(|(idx, sess)| busy_info_for(sess, idx))
            .collect()
    }

    /// v0.1.3 per-tab close amendment: busy-info for one specific tab.
    /// Returns `Some` when the tab at `idx` is busy, `None` when idle or
    /// when `idx` is out of range.
    #[must_use]
    pub fn tab_busy_info(&self, idx: usize) -> Option<BusyTabInfo> {
        self.tabs.get(idx).and_then(|sess| busy_info_for(sess, idx))
    }

    /// v0.1.3 per-tab close amendment: busy-info for every tab except
    /// `keep_idx` (the survivor in a "Close Other Tabs" action).
    #[must_use]
    pub fn tabs_busy_except(&self, keep_idx: usize) -> Vec<BusyTabInfo> {
        self.tabs
            .iter()
            .enumerate()
            .filter(|(idx, _)| *idx != keep_idx)
            .filter_map(|(idx, sess)| busy_info_for(sess, idx))
            .collect()
    }

    /// v0.1.3: true when `WindowEvent::CloseRequested` should show the confirm
    /// dialog instead of exiting immediately. iTerm2-style: confirm if
    /// multi-tab OR any tab busy. Bypassed entirely when
    /// `ui.confirm_on_close == false`.
    pub fn close_needs_confirmation(&self, ui: &crate::config::Ui) -> bool {
        if !ui.confirm_on_close {
            return false;
        }
        self.tabs.len() > 1 || !self.busy_tabs().is_empty()
    }

    /// v0.1.3 per-tab close amendment: true when closing the tab at `idx`
    /// should surface a confirm dialog. Confirm only if that tab is busy —
    /// closing one idle tab in a multi-tab window is a deliberate, contained
    /// action and the surviving tabs are still safe (deliberately different
    /// from `close_needs_confirmation`'s multi-tab rule).
    pub fn tab_close_needs_confirmation(&self, idx: usize, ui: &crate::config::Ui) -> bool {
        if !ui.confirm_on_close {
            return false;
        }
        self.tab_busy_info(idx).is_some()
    }

    /// v0.1.3 per-tab close amendment: true when "Close Other Tabs"
    /// (everything except `keep_idx`) should surface a confirm dialog.
    /// Confirm if more than one tab would be closed OR any of those tabs is
    /// busy. False if zero tabs would be closed (keep_idx is the only tab).
    pub fn close_others_needs_confirmation(&self, keep_idx: usize, ui: &crate::config::Ui) -> bool {
        if !ui.confirm_on_close {
            return false;
        }
        let total = self.tabs.len();
        let would_close = total.saturating_sub(if keep_idx < total { 1 } else { 0 });
        if would_close == 0 {
            return false;
        }
        would_close > 1 || !self.tabs_busy_except(keep_idx).is_empty()
    }

    /// Index of the currently focused tab. Valid only when `tabs()` is non-empty.
    #[must_use]
    pub fn active(&self) -> usize {
        self.active
    }

    /// Set the focused tab. No-op if `idx` is out of range.
    pub fn set_active(&mut self, idx: usize) {
        if idx < self.tabs.len() {
            self.active = idx;
        }
    }

    /// Mutable slice of all sessions. Stage 8's selection / mouse routing
    /// needs `tabs_mut().get_mut(active)` to call `selection.mouse_*` and
    /// `send_input` from the `window.rs` dispatch layer.
    #[must_use]
    pub fn tabs_mut(&mut self) -> &mut [PtySession] {
        &mut self.tabs
    }

    /// Select the entire buffer of the active tab. Thin wrapper over
    /// [`crate::render::selection::SelectionTracker::select_all`] that resolves
    /// the split-borrow between `selection` (needs `&mut`) and `term()` (needs
    /// `&`) on the same `PtySession` via `PtySession::split_borrow_mouse`.
    /// No-op when there are no tabs.
    pub fn select_all_active(&mut self) {
        let Some(s) = self.tabs.get_mut(self.active) else {
            return;
        };
        let (sel, term) = s.split_borrow_mouse();
        sel.select_all(term);
    }

    /// Restart the dead active session. No-op on live sessions and on the
    /// no-tabs sentinel state.
    ///
    /// # Errors
    /// Propagates `PtySession::restart` errors.
    pub fn restart_active(&mut self) -> std::io::Result<()> {
        let Some(s) = self.tabs.get_mut(self.active) else {
            return Ok(());
        };
        if s.is_alive() {
            tracing::trace!("Ctrl+Shift+R on live tab; ignoring");
            return Ok(());
        }
        s.restart()?;

        // Propagate Stage 11 defaults to the restarted session (same pattern as new_tab).
        s.tools_list = self.default_tools_list.clone();
        s.proc_check_interval = self.default_proc_check_interval;
        s.set_tracker_config(self.tracker_config);
        s.scrollbar_fade.set_fade_ms(self.default_scrollbar_fade_ms);
        s.history_lines = self.default_history_lines as usize;
        s.theme = self.default_theme.clone();

        Ok(())
    }

    /// Cycle the active tab by `direction`: +1 = forward, -1 = backward.
    /// Wraps around. No-op when there are no tabs.
    pub fn cycle_active(&mut self, direction: i32) {
        let len = self.tabs.len();
        if len == 0 {
            return;
        }
        let cur = self.active as i32;
        let next = (cur + direction).rem_euclid(len as i32);
        self.active = next as usize;
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::tracker::TabState;

    #[test]
    fn new_app_has_no_tabs() {
        let app = App::new();
        assert!(app.tabs().is_empty());
    }

    #[test]
    fn new_tab_spawns_and_focuses() {
        let mut app = App::new();
        let idx = app.new_tab(&["/bin/sh", "-c", "sleep 5"]).unwrap();
        assert_eq!(idx, 0);
        assert_eq!(app.tabs().len(), 1);
        assert_eq!(app.active(), 0);
        assert_eq!(app.tabs()[0].state(), TabState::Active);
    }

    #[test]
    fn close_tab_removes_session() {
        let mut app = App::new();
        app.new_tab(&["/bin/sh", "-c", "sleep 5"]).unwrap();
        app.new_tab(&["/bin/sh", "-c", "sleep 5"]).unwrap();
        assert_eq!(app.tabs().len(), 2);
        app.close_tab(0);
        assert_eq!(app.tabs().len(), 1);
    }

    #[test]
    fn close_tab_with_invalid_index_is_a_no_op() {
        let mut app = App::new();
        app.new_tab(&["/bin/sh", "-c", "sleep 5"]).unwrap();
        app.close_tab(99); // out of range
        assert_eq!(app.tabs().len(), 1);
    }

    #[test]
    fn close_tab_left_of_active_shifts_active_down() {
        // tabs [0, 1, 2, 3]; focused on 2. Closing tab 0 should leave the same
        // session focused — at its new index 1.
        let mut app = App::new();
        for _ in 0..4 {
            app.new_tab(&["/bin/sh", "-c", "sleep 5"]).unwrap();
        }
        // After spawning 4 tabs, `active` is 3 (every new_tab focuses itself).
        // Move focus to index 2 manually for the test scenario.
        app.active = 2;
        app.close_tab(0);
        assert_eq!(app.tabs().len(), 3);
        assert_eq!(app.active(), 1, "focus should shift down with the element");
    }

    use std::time::{Duration, Instant};
    use vibeflow_protocol::{Frame as ProtoFrame, State as ProtoState};

    /// Poll-with-deadline helper. See `session::session::tests::wait_until`
    /// for the rationale (parallel PTY load defeats fixed sleep windows).
    fn wait_until<F: FnMut() -> bool>(timeout: Duration, mut cond: F) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if cond() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        cond()
    }

    #[test]
    fn tick_all_returns_empty_when_no_timeouts_have_fired() {
        let mut app = App::new();
        app.new_tab(&["/bin/sh", "-c", "sleep 5"]).unwrap();
        let evs = app.tick_all(Instant::now() + Duration::from_secs(1));
        assert!(evs.is_empty());
    }

    #[test]
    fn poll_all_collects_state_changes_from_each_session() {
        let mut app = App::new();
        // Tab 0: emits a single OSC 1338 working frame, then exits.
        let bytes = ProtoFrame::new(ProtoState::Working).to_bytes();
        let bytes_repr = bytes
            .iter()
            .map(|b| b.to_string())
            .collect::<Vec<_>>()
            .join(",");
        app.new_tab(&[
            "python3",
            "-c",
            &format!("import sys; sys.stdout.buffer.write(bytes([{bytes_repr}]))"),
        ])
        .unwrap();

        // Poll for up to 5s, looking for a StateChanged(Working) event from
        // tab 0.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut found = false;
        while Instant::now() < deadline && !found {
            for (idx, ev) in app.poll_all(Instant::now()) {
                if idx == 0 && matches!(ev, SessionEvent::StateChanged(TabState::Working)) {
                    found = true;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(found, "expected tab 0 to transition to Working");
    }

    #[test]
    fn send_input_writes_to_active_tab() {
        let mut app = App::new();
        app.new_tab(&["/bin/cat"]).unwrap();
        app.send_input(b"hi\n").unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saw_hi = false;
        while Instant::now() < deadline && !saw_hi {
            // Drain any TermUpdated/StateChanged events; their side effect
            // is updating the per-session Term, which we read below.
            let _events = app.poll_all(Instant::now());
            if let Some(term) = app.active_term() {
                let row0: String = term
                    .renderable_content()
                    .display_iter
                    .filter(|i| i.point.line.0 == 0)
                    .map(|i| i.cell.c)
                    .collect();
                if row0.contains("hi") {
                    saw_hi = true;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(saw_hi, "expected `hi` in active tab's grid");
        // Tell cat to exit so the test doesn't hang on shutdown.
        let _ = app.send_input(&[0x04]);
    }

    #[test]
    fn resize_all_fans_out_to_every_session() {
        let mut app = App::new();
        app.new_tab(&["/bin/sh", "-c", "sleep 5"]).unwrap();
        app.new_tab(&["/bin/sh", "-c", "sleep 5"]).unwrap();
        // Expect Ok and no panic. Real per-tab observation lives in the
        // PtySession-level test in session::session.
        app.resize_all(40, 100).unwrap();
    }

    #[test]
    fn set_active_focuses_the_specified_tab() {
        let mut app = App::new();
        app.new_tab(&["/bin/sh", "-c", "sleep 5"]).unwrap();
        app.new_tab(&["/bin/sh", "-c", "sleep 5"]).unwrap();
        app.new_tab(&["/bin/sh", "-c", "sleep 5"]).unwrap();
        // After three new_tab calls, active = 2 (most-recently-spawned).
        assert_eq!(app.active(), 2);
        app.set_active(0);
        assert_eq!(app.active(), 0);
    }

    #[test]
    fn set_active_with_out_of_range_idx_is_a_no_op() {
        let mut app = App::new();
        app.new_tab(&["/bin/sh", "-c", "sleep 5"]).unwrap();
        app.set_active(99);
        assert_eq!(app.active(), 0);
    }

    #[test]
    fn close_tab_last_tab_leaves_empty_app_with_active_sentinel() {
        let mut app = App::new();
        app.new_tab(&["/bin/sh", "-c", "true"]).unwrap();
        app.close_tab(0);
        assert!(app.tabs().is_empty());
        // `active` is a sentinel value on an empty App; `tabs().get(0)`
        // returns None so callers never mis-index.
        assert_eq!(app.active(), 0);
    }

    #[test]
    fn cycle_active_wraps_forward_and_backward() {
        let mut app = App::new();
        // App::new() starts empty. App::new_tab spawns a sleep then sets
        // active. Use it three times to populate.
        for _ in 0..3 {
            app.new_tab(&["sleep", "30"]).expect("new_tab spawns");
        }
        app.set_active(0);
        app.cycle_active(1);
        assert_eq!(app.active(), 1);
        app.cycle_active(1);
        assert_eq!(app.active(), 2);
        app.cycle_active(1);
        assert_eq!(app.active(), 0); // wraps
        app.cycle_active(-1);
        assert_eq!(app.active(), 2); // wraps backward
    }

    #[test]
    fn new_tab_inherits_default_tools_list() {
        let mut app = App::new();
        app.set_default_tools_list(vec!["claude".to_owned(), "codex".to_owned()]);
        let _ = app.new_tab(&["/bin/sh", "-c", "sleep 5"]).expect("spawn");
        assert_eq!(
            app.tabs()[0].tools_list,
            vec!["claude".to_owned(), "codex".to_owned()]
        );
    }

    #[test]
    fn new_tab_inherits_default_proc_check_interval() {
        let mut app = App::new();
        app.set_default_proc_check_interval(std::time::Duration::from_millis(500));
        let _ = app.new_tab(&["/bin/sh", "-c", "sleep 5"]).expect("spawn");
        assert_eq!(
            app.tabs()[0].proc_check_interval,
            std::time::Duration::from_millis(500)
        );
    }

    #[test]
    fn set_default_tracker_config_persists_for_future_spawns() {
        // Verify the setter exists and that spawn uses the new default
        // without panicking. Full state-change behavior is covered in Task 2
        // (tracker::set_config_updates_heuristic_silence_threshold) and
        // Task 10's integration tests — those exercise tracker state via
        // PtySession's public `state()` method against a real PTY.
        //
        // We can't easily verify "spawn used the default" from app.rs because
        // `PtySession.tracker` is private (no public getter) and `app.rs` is
        // a different module. So this test just confirms the setter API and
        // that new_tab + spawn complete without error after the setter runs.
        let mut app = App::new();
        let cfg = TrackerConfig {
            heuristic_silence: std::time::Duration::from_millis(7000),
            ..TrackerConfig::default()
        };
        app.set_default_tracker_config(cfg);
        let _ = app.new_tab(&["/bin/sh", "-c", "sleep 5"]).expect("spawn");
        assert_eq!(app.tabs().len(), 1);
        // Calling set_tracker_config on the live session is also safe.
        app.tabs_mut()[0].set_tracker_config(cfg);
    }

    #[test]
    fn new_tab_inherits_default_scrollbar_fade_ms() {
        let mut app = App::new();
        app.set_default_scrollbar_fade_ms(2222);
        let _ = app.new_tab(&["/bin/sh", "-c", "sleep 5"]).expect("spawn");
        let now = std::time::Instant::now();
        let s = &mut app.tabs_mut()[0];
        s.scrollbar_fade.mark_scrolled(now);
        // 2300ms past should be elapsed past 2222ms threshold → 0.0.
        assert_eq!(
            s.scrollbar_fade
                .alpha(now + std::time::Duration::from_millis(2300)),
            0.0
        );
        // Just under threshold should still be > 0 (linear fade — small positive value).
        let near_end = s
            .scrollbar_fade
            .alpha(now + std::time::Duration::from_millis(2100));
        assert!(
            near_end > 0.0 && near_end < 0.1,
            "near-threshold fade alpha out of range: {near_end}"
        );
    }

    #[test]
    fn restart_active_propagates_scrollbar_fade() {
        let mut app = App::new();
        app.set_default_scrollbar_fade_ms(777);
        // Spawn a short-lived child so it can be restarted.
        let _ = app.new_tab(&["/bin/sh", "-c", "true"]).expect("spawn");
        // Wait for the child to die.
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        while std::time::Instant::now() < deadline && app.tabs()[0].is_alive() {
            let _ = app.poll_all(std::time::Instant::now());
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.restart_active().expect("restart");
        // After restart, the scrollbar_fade should still use the App default.
        let now = std::time::Instant::now();
        app.tabs_mut()[0].scrollbar_fade.mark_scrolled(now);
        assert_eq!(app.tabs_mut()[0].scrollbar_fade.alpha(now), 1.0);
        assert_eq!(
            app.tabs_mut()[0]
                .scrollbar_fade
                .alpha(now + std::time::Duration::from_millis(800)),
            0.0,
            "fade should have elapsed past the 777ms threshold"
        );
    }

    #[test]
    fn restart_active_preserves_stage11_fields() {
        let mut app = App::new();
        app.set_default_tools_list(vec!["claude".to_owned()]);
        app.set_default_proc_check_interval(std::time::Duration::from_millis(500));
        app.set_default_tracker_config(TrackerConfig {
            heuristic_silence: std::time::Duration::from_millis(7000),
            ..TrackerConfig::default()
        });
        // Spawn `/bin/sh -c true` (exits quickly).
        let _ = app.new_tab(&["/bin/sh", "-c", "true"]).expect("spawn");

        // Wait briefly for the child to exit.
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        while std::time::Instant::now() < deadline && app.tabs()[0].is_alive() {
            let _ = app.poll_all(std::time::Instant::now());
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!app.tabs()[0].is_alive(), "child should have exited");

        // Restart, then verify the Stage 11 fields are preserved.
        app.restart_active().expect("restart");
        assert_eq!(
            app.tabs()[0].tools_list,
            vec!["claude".to_owned()],
            "tools_list should propagate from App defaults after restart"
        );
        assert_eq!(
            app.tabs()[0].proc_check_interval,
            std::time::Duration::from_millis(500),
            "proc_check_interval should propagate"
        );
        // tracker_config isn't directly readable from app.rs (private field),
        // but set_tracker_config not panicking is a smoke check.
        app.tabs_mut()[0].set_tracker_config(TrackerConfig::default());
    }

    #[test]
    fn restart_active_picks_up_updated_history_lines() {
        let mut app = App::new();
        app.set_default_history_lines(5000);
        let _ = app.new_tab(&["/bin/sh", "-c", "true"]).expect("spawn");
        // Wait for the child to exit.
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        while std::time::Instant::now() < deadline && app.tabs()[0].is_alive() {
            let _ = app.poll_all(std::time::Instant::now());
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // Update history_lines before restart.
        app.set_default_history_lines(123);
        app.restart_active().expect("restart");
        assert_eq!(
            app.tabs()[0].history_lines,
            123,
            "restart should pick up updated default_history_lines"
        );
    }

    #[test]
    fn new_tab_inherits_default_theme() {
        let mut app = App::new();
        app.set_default_theme(Some("solarized".to_owned()));
        let idx = app.new_tab(&["/bin/sh", "-c", "sleep 5"]).expect("spawn");
        assert_eq!(app.tabs()[idx].theme.as_deref(), Some("solarized"));
    }

    use crate::config::Ui;

    fn make_app_with_n_idle_tabs(n: usize) -> App {
        let mut app = App::new();
        for _ in 0..n {
            app.new_tab(&["/bin/bash"]).expect("spawn bash");
        }
        // Allow shells to settle so has_foreground_child is stable.
        std::thread::sleep(Duration::from_millis(250));
        for tab in app.tabs_mut().iter_mut() {
            let _ = tab.poll(std::time::Instant::now());
        }
        app
    }

    #[test]
    fn busy_tabs_empty_for_zero_tab_app() {
        let app = App::new();
        assert!(app.busy_tabs().is_empty());
    }

    #[test]
    fn busy_tabs_empty_for_single_idle_tab() {
        let app = make_app_with_n_idle_tabs(1);
        assert!(
            app.busy_tabs().is_empty(),
            "single idle bash tab should report no busy tabs"
        );
    }

    #[test]
    fn busy_tabs_empty_for_multiple_idle_tabs() {
        let app = make_app_with_n_idle_tabs(3);
        assert!(
            app.busy_tabs().is_empty(),
            "multiple idle tabs should still report no busy tabs (busy != multi-tab)"
        );
    }

    #[test]
    fn busy_tabs_lists_tab_with_foreground_subprocess() {
        let mut app = make_app_with_n_idle_tabs(1);
        app.tabs_mut()[0]
            .send_input(b"sleep 30\n")
            .expect("send sleep");
        assert!(
            wait_until(Duration::from_secs(5), || {
                let _ = app.tabs_mut()[0].poll(Instant::now());
                !app.busy_tabs().is_empty()
            }),
            "bash running `sleep 30` should surface a busy tab within 5s"
        );
        let busy = app.busy_tabs();
        assert_eq!(busy.len(), 1, "exactly one busy tab expected");
        assert_eq!(busy[0].tab_index, 1, "1-based tab index for display");
        assert_eq!(busy[0].display_label, "sleep");
        assert_eq!(busy[0].state_label, "running");
    }

    #[test]
    fn close_needs_confirmation_false_for_single_idle_tab() {
        let app = make_app_with_n_idle_tabs(1);
        let ui = Ui::default();
        assert!(
            !app.close_needs_confirmation(&ui),
            "single idle tab should close silently"
        );
    }

    #[test]
    fn close_needs_confirmation_true_for_multiple_idle_tabs() {
        let app = make_app_with_n_idle_tabs(2);
        let ui = Ui::default();
        assert!(
            app.close_needs_confirmation(&ui),
            "multi-tab (even all idle) should confirm per iTerm2 convention"
        );
    }

    #[test]
    fn close_needs_confirmation_true_for_single_busy_tab() {
        let mut app = make_app_with_n_idle_tabs(1);
        app.tabs_mut()[0]
            .send_input(b"sleep 30\n")
            .expect("send sleep");
        let ui = Ui::default();
        assert!(
            wait_until(Duration::from_secs(5), || {
                let _ = app.tabs_mut()[0].poll(Instant::now());
                app.close_needs_confirmation(&ui)
            }),
            "single tab running `sleep 30` should require confirmation within 5s"
        );
    }

    #[test]
    fn close_needs_confirmation_false_when_ui_disables_it() {
        let mut app = make_app_with_n_idle_tabs(5);
        app.tabs_mut()[0]
            .send_input(b"sleep 30\n")
            .expect("send sleep");
        // Wait for bash to actually fork sleep so the bypass assertion below
        // is meaningfully testing the "busy + 5 tabs but switch is off" case.
        let _ = wait_until(Duration::from_secs(5), || {
            let _ = app.tabs_mut()[0].poll(Instant::now());
            !app.busy_tabs().is_empty()
        });
        let ui = Ui {
            confirm_on_close: false,
        };
        assert!(
            !app.close_needs_confirmation(&ui),
            "confirm_on_close = false should bypass entirely"
        );
    }

    // ---------------- v0.1.3 per-tab close amendment ----------------

    #[test]
    fn tab_busy_info_returns_none_for_idle_tab() {
        let app = make_app_with_n_idle_tabs(2);
        assert!(app.tab_busy_info(0).is_none());
        assert!(app.tab_busy_info(1).is_none());
    }

    #[test]
    fn tab_busy_info_returns_some_for_busy_tab() {
        let mut app = make_app_with_n_idle_tabs(2);
        app.tabs_mut()[1].send_input(b"sleep 30\n").expect("send");
        assert!(
            wait_until(Duration::from_secs(5), || {
                let _ = app.tabs_mut()[1].poll(Instant::now());
                app.tab_busy_info(1).is_some()
            }),
            "tab 1 with `sleep 30` should report busy_info within 5s"
        );
        let info = app.tab_busy_info(1).expect("busy info");
        assert_eq!(info.tab_index, 2, "1-based for display");
        assert_eq!(info.display_label, "sleep");
        assert_eq!(info.state_label, "running");
        // Idle siblings still report None.
        assert!(app.tab_busy_info(0).is_none());
    }

    #[test]
    fn tab_busy_info_returns_none_for_out_of_range() {
        let app = make_app_with_n_idle_tabs(1);
        assert!(app.tab_busy_info(99).is_none());
    }

    #[test]
    fn tab_close_needs_confirmation_true_for_busy_tab() {
        let mut app = make_app_with_n_idle_tabs(1);
        app.tabs_mut()[0].send_input(b"sleep 30\n").expect("send");
        let ui = Ui::default();
        assert!(
            wait_until(Duration::from_secs(5), || {
                let _ = app.tabs_mut()[0].poll(Instant::now());
                app.tab_close_needs_confirmation(0, &ui)
            }),
            "busy tab close should require confirmation within 5s"
        );
    }

    #[test]
    fn tab_close_needs_confirmation_false_for_idle_tab() {
        // Unlike window-close, per-tab close is silent for idle even in a
        // multi-tab window — closing one tab is a deliberate, contained
        // action and the surviving tabs are still safe.
        let app = make_app_with_n_idle_tabs(3);
        let ui = Ui::default();
        assert!(!app.tab_close_needs_confirmation(0, &ui));
        assert!(!app.tab_close_needs_confirmation(2, &ui));
    }

    #[test]
    fn tab_close_needs_confirmation_false_when_ui_disables_it() {
        let mut app = make_app_with_n_idle_tabs(1);
        app.tabs_mut()[0].send_input(b"sleep 30\n").expect("send");
        let _ = wait_until(Duration::from_secs(5), || {
            let _ = app.tabs_mut()[0].poll(Instant::now());
            !app.busy_tabs().is_empty()
        });
        let ui = Ui {
            confirm_on_close: false,
        };
        assert!(!app.tab_close_needs_confirmation(0, &ui));
    }

    #[test]
    fn tabs_busy_except_excludes_keep_idx() {
        let mut app = make_app_with_n_idle_tabs(3);
        app.tabs_mut()[0].send_input(b"sleep 30\n").expect("send");
        app.tabs_mut()[2].send_input(b"sleep 30\n").expect("send");
        // Wait for both subprocesses to fork.
        let _ = wait_until(Duration::from_secs(5), || {
            let _ = app.tabs_mut()[0].poll(Instant::now());
            let _ = app.tabs_mut()[2].poll(Instant::now());
            app.tab_busy_info(0).is_some() && app.tab_busy_info(2).is_some()
        });
        // Keep tab 0; report busy of tabs 1, 2. Tab 1 is idle, tab 2 is busy.
        let busy = app.tabs_busy_except(0);
        assert_eq!(busy.len(), 1, "only tab 2 should appear as busy");
        assert_eq!(busy[0].tab_index, 3, "1-based index for tab 2");
    }

    #[test]
    fn close_others_needs_confirmation_true_for_multi_other_tabs() {
        let app = make_app_with_n_idle_tabs(3);
        let ui = Ui::default();
        // Keep tab 0; closing tabs 1 & 2 (2 tabs > 1) → confirm even if idle.
        assert!(app.close_others_needs_confirmation(0, &ui));
    }

    #[test]
    fn close_others_needs_confirmation_false_for_single_idle_other_tab() {
        let app = make_app_with_n_idle_tabs(2);
        let ui = Ui::default();
        // Keep tab 0; only tab 1 to close, and it's idle → silent.
        assert!(!app.close_others_needs_confirmation(0, &ui));
    }

    #[test]
    fn close_others_needs_confirmation_true_for_single_busy_other_tab() {
        let mut app = make_app_with_n_idle_tabs(2);
        app.tabs_mut()[1].send_input(b"sleep 30\n").expect("send");
        let ui = Ui::default();
        assert!(
            wait_until(Duration::from_secs(5), || {
                let _ = app.tabs_mut()[1].poll(Instant::now());
                app.close_others_needs_confirmation(0, &ui)
            }),
            "one busy other tab should require confirmation within 5s"
        );
    }

    #[test]
    fn close_others_needs_confirmation_false_when_ui_disables_it() {
        let app = make_app_with_n_idle_tabs(5);
        let ui = Ui {
            confirm_on_close: false,
        };
        assert!(!app.close_others_needs_confirmation(0, &ui));
    }
}
