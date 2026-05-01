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

use std::time::Instant;

use vibeflow_protocol::Frame;

use crate::session::osc::PromptMarker;

/// Inputs the tracker reacts to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackerInput {
    /// An OSC 1338 frame from a tool — directly drives state.
    AiFrame(Frame),
    /// An OSC 133 prompt marker from the shell — used to derive Idle/Working
    /// when no AI tool is active.
    Prompt(PromptMarker),
    /// "Output bytes observed at `now`" — used by heuristic silence detection.
    /// Doesn't directly change state; just resets the silence timer.
    OutputObserved,
}

/// Per-tab state machine: tracks the current [`TabState`], applies debounce,
/// and surfaces stale-state and heuristic-silence timeouts via [`tick`].
///
/// Time is injected as an explicit `now: Instant` argument on every method
/// rather than read from the system clock. This keeps the tracker a pure
/// function of its inputs (testable without sleeping) and matches the
/// single-thread main-loop call site that already has `Instant::now()` in hand.
///
/// [`tick`]: AiStateTracker::tick
#[derive(Debug)]
pub struct AiStateTracker {
    state: TabState,
    #[allow(dead_code)] // used in Task 10 for debounce logic
    config: TrackerConfig,
    /// `Instant` of the last input that affected state. `None` until the first
    /// state transition.
    last_event_at: Option<Instant>,
    /// `Instant` of the last `OutputObserved` input. `None` until first observed.
    last_output_at: Option<Instant>,
    /// Set externally by the App (Stage 3+) when the foreground process matches
    /// the configured AI-tool list. Drives Tier 3 heuristic silence inference.
    heuristic_active: bool,
}

impl AiStateTracker {
    #[must_use]
    pub fn new(config: TrackerConfig) -> Self {
        Self {
            state: TabState::default(),
            config,
            last_event_at: None,
            last_output_at: None,
            heuristic_active: false,
        }
    }

    /// Current visual state.
    #[must_use]
    pub fn state(&self) -> TabState {
        self.state
    }

    /// Apply an input at `now`. Returns `true` if the state changed.
    pub fn on_input(&mut self, input: TrackerInput, now: Instant) -> bool {
        match input {
            TrackerInput::AiFrame(frame) => self.transition_to(frame.state.into(), now),
            TrackerInput::Prompt(marker) => {
                let _ = marker;
                // Prompt-driven transitions land in Task 9.
                false
            }
            TrackerInput::OutputObserved => {
                self.last_output_at = Some(now);
                false
            }
        }
    }

    /// Stale-state and heuristic-silence checks at `now`. Returns `true` if a
    /// timeout caused a state change. (Stub for Task 8; real logic in Tasks 11–12.)
    #[allow(dead_code)] // first lib-level caller arrives in Task 11
    pub fn tick(&mut self, now: Instant) -> bool {
        let _ = now;
        false
    }

    /// Toggle the Tier 3 heuristic — set true when the foreground process is
    /// in the configured AI-tool list, false otherwise.
    #[allow(dead_code)] // first lib-level caller is in the App in Stage 3
    pub fn set_heuristic_active(&mut self, active: bool) {
        self.heuristic_active = active;
    }

    /// Internal: change state if the new value differs and (Task 10+) debounce
    /// allows. Returns true if the state actually changed.
    fn transition_to(&mut self, new_state: TabState, now: Instant) -> bool {
        if self.state == new_state {
            return false;
        }
        self.state = new_state;
        self.last_event_at = Some(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use vibeflow_protocol::Frame;

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

    #[test]
    fn tracker_starts_in_active_state() {
        let t = AiStateTracker::new(TrackerConfig::default());
        assert_eq!(t.state(), TabState::Active);
    }

    #[test]
    fn tracker_transitions_to_working_on_ai_frame() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        let changed = t.on_input(TrackerInput::AiFrame(Frame::new(State::Working)), now);
        assert!(changed);
        assert_eq!(t.state(), TabState::Working);
    }

    #[test]
    fn tracker_transitions_to_waiting_on_ai_frame() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        // Use a long delay past the debounce window so this test is unaffected
        // when debounce is added in Task 10.
        t.on_input(TrackerInput::AiFrame(Frame::new(State::Working)), now);
        let changed = t.on_input(
            TrackerInput::AiFrame(Frame::new(State::Waiting)),
            now + Duration::from_secs(1),
        );
        assert!(changed);
        assert_eq!(t.state(), TabState::Waiting);
    }

    #[test]
    fn tracker_returns_false_when_state_unchanged() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        // Tracker starts in Active; sending an Active frame must not register
        // as a change.
        let changed = t.on_input(TrackerInput::AiFrame(Frame::new(State::Active)), now);
        assert!(!changed);
        assert_eq!(t.state(), TabState::Active);
    }

    #[test]
    fn tracker_handles_output_observed_without_changing_state() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        t.on_input(
            TrackerInput::AiFrame(Frame::new(State::Working)),
            Instant::now(),
        );
        let changed = t.on_input(TrackerInput::OutputObserved, Instant::now());
        assert!(!changed);
        assert_eq!(t.state(), TabState::Working);
    }
}
