//! Per-tab AI state tracker — debounced state machine with heuristic-silence
//! and stale-state timeouts.

use std::time::Duration;

use vibeflow_protocol::State;

/// Visual state of a single tab/session.
///
/// A strict superset of [`vibeflow_protocol::State`]: adds [`TabState::Idle`]
/// for "shell at prompt, no command running", which the OSC 1338 protocol
/// cannot carry (only AI tools emit OSC 1338, and an idle shell isn't one).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TabState {
    /// Default — nothing notable is happening.
    #[default]
    Active,
    /// A tool/shell is running a command.
    Working,
    /// A tool is waiting for user input. The headline state.
    Waiting,
    /// A tool just finished a task; usually transient.
    Done,
    /// Shell at prompt, nothing running.
    Idle,
}

impl From<State> for TabState {
    fn from(s: State) -> Self {
        match s {
            State::Active => TabState::Active,
            State::Working => TabState::Working,
            State::Waiting => TabState::Waiting,
            State::Done => TabState::Done,
        }
    }
}

/// Tunable thresholds for the tracker. Mirrors the `[ai]` section of vibeflow's
/// TOML config (added in a later stage); defaults match the design spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackerConfig {
    /// Ignore state transitions closer together than this. Spec default 100 ms.
    pub debounce: Duration,
    /// Tier 3 fallback: infer `Waiting` after this much output silence on a
    /// session whose foreground process is in the configured AI-tool list.
    /// Spec default 4000 ms.
    pub heuristic_silence: Duration,
    /// Reset to `Active` if a tool emits a state but never updates again — guards
    /// against stuck indicators when a tool dies mid-task. Spec default 30 s.
    pub stale_state: Duration,
}

impl Default for TrackerConfig {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(100),
            heuristic_silence: Duration::from_millis(4000),
            stale_state: Duration::from_secs(30),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_state_converts_to_tab_state() {
        assert_eq!(TabState::from(State::Active), TabState::Active);
        assert_eq!(TabState::from(State::Working), TabState::Working);
        assert_eq!(TabState::from(State::Waiting), TabState::Waiting);
        assert_eq!(TabState::from(State::Done), TabState::Done);
    }

    #[test]
    fn tracker_config_defaults_match_spec() {
        let c = TrackerConfig::default();
        assert_eq!(c.debounce, Duration::from_millis(100));
        assert_eq!(c.heuristic_silence, Duration::from_millis(4000));
        assert_eq!(c.stale_state, Duration::from_secs(30));
    }

    #[test]
    fn tab_state_default_is_active() {
        assert_eq!(TabState::default(), TabState::Active);
    }
}
