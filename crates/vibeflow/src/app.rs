//! `App` — single-threaded "central authority" that owns every tab's
//! [`PtySession`] and orchestrates polling and timeout ticks.

use crate::session::tracker::TrackerConfig;
use crate::session::PtySession;

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
    /// Create an empty `App` with no tabs. Call [`new_tab`] to spawn the first.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active: 0,
            tracker_config: default_tracker_config(),
        }
    }

    /// Spawn a new tab. Returns the index of the new tab in [`tabs`]. The new
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
    /// and joins the reader thread.
    pub fn close_tab(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        let _dropped = self.tabs.remove(idx);
        if self.active >= self.tabs.len() && !self.tabs.is_empty() {
            self.active = self.tabs.len() - 1;
        }
    }

    /// Snapshot of all sessions (for read-only inspection — Stage 4+ tab-bar
    /// renderer uses this to draw indicator stripes).
    #[must_use]
    pub fn tabs(&self) -> &[PtySession] {
        &self.tabs
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
    use crate::session::SessionEvent;

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
    fn _unused_session_event_silences_dead_code() {
        // Force a use of SessionEvent so its `Died` variant isn't reported as
        // unread until App::poll_all (Task 8) wires it through.
        let _ = SessionEvent::Died;
    }
}
