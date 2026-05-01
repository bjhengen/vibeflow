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
                let target = match marker {
                    PromptMarker::PromptStart
                    | PromptMarker::PromptEnd
                    | PromptMarker::CommandEnd { .. } => TabState::Idle,
                    PromptMarker::CommandStart => TabState::Working,
                };
                self.transition_to(target, now)
            }
            TrackerInput::OutputObserved => {
                self.last_output_at = Some(now);
                false
            }
        }
    }

    /// Stale-state and heuristic-silence checks at `now`. Returns `true` if a
    /// timeout caused a state change.
    pub fn tick(&mut self, now: Instant) -> bool {
        // Heuristic-silence (Tier 3): when active and Working, infer Waiting
        // after `config.heuristic_silence` of observed output silence.
        if self.heuristic_active && self.state == TabState::Working {
            if let Some(last_out) = self.last_output_at {
                if now.saturating_duration_since(last_out) >= self.config.heuristic_silence {
                    // Note: we bypass `transition_to` here because the heuristic
                    // is *itself* a debounce-tier signal — it shouldn't be
                    // suppressed by the 100 ms inter-transition window.
                    self.state = TabState::Waiting;
                    self.last_event_at = Some(now);
                    return true;
                }
            }
        }
        // Stale-state timeout: reset to Active if non-Active and inactive for
        // longer than `config.stale_state`.
        if self.state != TabState::Active {
            if let Some(last) = self.last_event_at {
                if now.saturating_duration_since(last) >= self.config.stale_state {
                    self.state = TabState::Active;
                    self.last_event_at = Some(now);
                    return true;
                }
            }
        }
        false
    }

    /// Toggle the Tier 3 heuristic — set true when the foreground process is
    /// in the configured AI-tool list, false otherwise.
    pub fn set_heuristic_active(&mut self, active: bool) {
        self.heuristic_active = active;
    }

    /// Internal: change state if the new value differs and (Task 10+) debounce
    /// allows. Returns true if the state actually changed.
    fn transition_to(&mut self, new_state: TabState, now: Instant) -> bool {
        if self.state == new_state {
            return false;
        }
        // Debounce: suppress transitions closer together than `config.debounce`.
        // The first transition (last_event_at == None) is always accepted.
        if let Some(last) = self.last_event_at {
            if now.saturating_duration_since(last) < self.config.debounce {
                return false;
            }
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

    #[test]
    fn tracker_transitions_to_idle_on_prompt_start() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        // First, transition out of the default Active so we can observe the
        // change to Idle.
        t.on_input(
            TrackerInput::AiFrame(Frame::new(State::Working)),
            Instant::now(),
        );
        let changed = t.on_input(
            TrackerInput::Prompt(PromptMarker::PromptStart),
            Instant::now() + Duration::from_secs(1),
        );
        assert!(changed);
        assert_eq!(t.state(), TabState::Idle);
    }

    #[test]
    fn tracker_transitions_to_idle_on_prompt_end() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        t.on_input(
            TrackerInput::AiFrame(Frame::new(State::Working)),
            Instant::now(),
        );
        let changed = t.on_input(
            TrackerInput::Prompt(PromptMarker::PromptEnd),
            Instant::now() + Duration::from_secs(1),
        );
        assert!(changed);
        assert_eq!(t.state(), TabState::Idle);
    }

    #[test]
    fn tracker_transitions_to_working_on_command_start() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        // Start by going to Idle so the Working transition is observable.
        t.on_input(
            TrackerInput::Prompt(PromptMarker::PromptStart),
            Instant::now(),
        );
        let changed = t.on_input(
            TrackerInput::Prompt(PromptMarker::CommandStart),
            Instant::now() + Duration::from_secs(1),
        );
        assert!(changed);
        assert_eq!(t.state(), TabState::Working);
    }

    #[test]
    fn tracker_transitions_to_idle_on_command_end() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        t.on_input(
            TrackerInput::Prompt(PromptMarker::CommandStart),
            Instant::now(),
        );
        let changed = t.on_input(
            TrackerInput::Prompt(PromptMarker::CommandEnd { exit_code: Some(0) }),
            Instant::now() + Duration::from_secs(1),
        );
        assert!(changed);
        assert_eq!(t.state(), TabState::Idle);
    }

    #[test]
    fn tracker_suppresses_flapping_within_debounce_window() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        // First transition at `now` — accepted.
        let c1 = t.on_input(TrackerInput::AiFrame(Frame::new(State::Working)), now);
        assert!(c1);
        assert_eq!(t.state(), TabState::Working);
        // Second transition 50 ms later — within the 100 ms debounce window;
        // suppressed. State must remain Working.
        let c2 = t.on_input(
            TrackerInput::AiFrame(Frame::new(State::Waiting)),
            now + Duration::from_millis(50),
        );
        assert!(!c2);
        assert_eq!(t.state(), TabState::Working);
    }

    #[test]
    fn tracker_accepts_transitions_past_debounce_window() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        t.on_input(TrackerInput::AiFrame(Frame::new(State::Working)), now);
        let changed = t.on_input(
            TrackerInput::AiFrame(Frame::new(State::Waiting)),
            now + Duration::from_millis(150),
        );
        assert!(changed);
        assert_eq!(t.state(), TabState::Waiting);
    }

    #[test]
    fn tracker_debounce_is_configurable() {
        let mut t = AiStateTracker::new(TrackerConfig {
            debounce: Duration::from_millis(500),
            ..TrackerConfig::default()
        });
        let now = Instant::now();
        t.on_input(TrackerInput::AiFrame(Frame::new(State::Working)), now);
        // 200 ms — within the custom 500 ms debounce, so suppressed.
        let changed = t.on_input(
            TrackerInput::AiFrame(Frame::new(State::Waiting)),
            now + Duration::from_millis(200),
        );
        assert!(!changed);
        assert_eq!(t.state(), TabState::Working);
    }

    #[test]
    fn tracker_stale_state_timeout_resets_to_active() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        t.on_input(TrackerInput::AiFrame(Frame::new(State::Working)), now);
        assert_eq!(t.state(), TabState::Working);

        // 31 seconds later (past the 30 s default), tick → reset to Active.
        let later = now + Duration::from_secs(31);
        let changed = t.tick(later);
        assert!(changed);
        assert_eq!(t.state(), TabState::Active);
    }

    #[test]
    fn tracker_does_not_reset_within_stale_window() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        t.on_input(TrackerInput::AiFrame(Frame::new(State::Working)), now);
        // 10 seconds later — still well within the 30 s stale window.
        let changed = t.tick(now + Duration::from_secs(10));
        assert!(!changed);
        assert_eq!(t.state(), TabState::Working);
    }

    #[test]
    fn tracker_stale_state_does_not_fire_when_already_active() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        // Tracker starts Active; an Active->Active "transition" never sets
        // last_event_at, so tick() can't have a baseline. Should not fire.
        let changed = t.tick(now + Duration::from_secs(60));
        assert!(!changed);
        assert_eq!(t.state(), TabState::Active);
    }

    #[test]
    fn tracker_stale_state_after_idle_resets_to_active() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        t.on_input(TrackerInput::Prompt(PromptMarker::PromptStart), now);
        assert_eq!(t.state(), TabState::Idle);
        // 31 seconds later — Idle should also be reset (stale-state spec
        // doesn't carve out shell-derived states).
        let changed = t.tick(now + Duration::from_secs(31));
        assert!(changed);
        assert_eq!(t.state(), TabState::Active);
    }

    #[test]
    fn tracker_heuristic_silence_infers_waiting() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        t.set_heuristic_active(true);
        // Working state set + last output observed.
        t.on_input(TrackerInput::AiFrame(Frame::new(State::Working)), now);
        t.on_input(TrackerInput::OutputObserved, now);

        // 5 seconds later — past the 4 s default heuristic_silence — but
        // BEFORE the debounce window from `now` would naturally have closed
        // (since 5 s > 100 ms). Heuristic timeout fires.
        let later = now + Duration::from_secs(5);
        let changed = t.tick(later);
        assert!(changed);
        assert_eq!(t.state(), TabState::Waiting);
    }

    #[test]
    fn tracker_heuristic_silence_inactive_when_flag_off() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        // heuristic_active stays false (default).
        t.on_input(TrackerInput::AiFrame(Frame::new(State::Working)), now);
        t.on_input(TrackerInput::OutputObserved, now);

        let changed = t.tick(now + Duration::from_secs(5));
        // No timeout fires; state stays Working until something else changes
        // it (or the stale-state timeout at 30 s).
        assert!(!changed);
        assert_eq!(t.state(), TabState::Working);
    }

    #[test]
    fn tracker_heuristic_silence_does_not_fire_outside_working() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        t.set_heuristic_active(true);
        // Tracker is Active (default). Heuristic only fires from Working.
        t.on_input(TrackerInput::OutputObserved, now);
        let changed = t.tick(now + Duration::from_secs(5));
        assert!(!changed);
        assert_eq!(t.state(), TabState::Active);
    }

    #[test]
    fn tracker_heuristic_silence_resets_on_new_output() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        t.set_heuristic_active(true);
        t.on_input(TrackerInput::AiFrame(Frame::new(State::Working)), now);
        // Output observed at now+3s — well within the 4 s silence window.
        t.on_input(TrackerInput::OutputObserved, now + Duration::from_secs(3));
        // Tick at now+5s — only 2 s of silence since last output. No fire.
        let changed = t.tick(now + Duration::from_secs(5));
        assert!(!changed);
        assert_eq!(t.state(), TabState::Working);
    }
}
