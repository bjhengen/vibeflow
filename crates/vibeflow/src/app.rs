//! `App` — single-threaded "central authority" that owns every tab's
//! [`PtySession`] and orchestrates polling and timeout ticks.

use crate::session::tracker::TrackerConfig;
use crate::session::{PtySession, SessionEvent};

/// Default per-tracker config used for every new tab. Stage 8 will replace
/// this with a TOML-loaded config sourced from `~/.config/vibeflow/config.toml`.
fn default_tracker_config() -> TrackerConfig {
    TrackerConfig::default()
}

/// Single-threaded central authority for the terminal app: owns every tab,
/// dispatches polls and ticks across them, tracks the focused tab.
pub struct App {
    tabs: Vec<PtySession>,
    active: usize,
    tracker_config: TrackerConfig,
}

impl App {
    /// Create an empty `App` with no tabs. Call [`Self::new_tab`] to spawn the first.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active: 0,
            tracker_config: default_tracker_config(),
        }
    }

    /// Spawn a new tab. Returns the index of the new tab in [`Self::tabs`]. The new
    /// tab becomes the active tab.
    ///
    /// # Errors
    /// Propagates any failure from [`PtySession::spawn`].
    pub fn new_tab(&mut self, argv: &[&str]) -> std::io::Result<usize> {
        let session = PtySession::spawn(argv, self.tracker_config)?;
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
        } else if self.active >= self.tabs.len() && !self.tabs.is_empty() {
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
    pub fn resize_all(&self, rows: u16, cols: u16) -> std::io::Result<()> {
        let mut first_error: Option<std::io::Error> = None;
        for tab in &self.tabs {
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

    /// Index of the currently focused tab. Valid only when `tabs()` is non-empty.
    #[must_use]
    pub fn active(&self) -> usize {
        self.active
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
        let mut got = false;
        while Instant::now() < deadline && !got {
            for (_, ev) in app.poll_all(Instant::now()) {
                if let SessionEvent::PassThrough(bytes) = ev {
                    if bytes.windows(2).any(|w| w == b"hi") {
                        got = true;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(got, "expected `hi` to round-trip through cat");
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
}
