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
    /// Stage 13 follow-up: for an explicit (OSC-1338) session, after this
    /// long with no new frame the fuse de-escalates the session (Working→
    /// Active, Tier-3 re-arms). `Duration::ZERO` disables (Q1 behavior).
    pub explicit_stale_state: Duration,
}

impl Default for TrackerConfig {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(100),
            heuristic_silence: Duration::from_millis(4000),
            stale_state: Duration::from_secs(30),
            explicit_stale_state: Duration::from_secs(300),
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
    /// Stage 13 Q1: set once any explicit OSC 1338 frame (Tier-1) is observed.
    /// Tier-1 is authoritative — once a session self-reports, the Tier-3 /proc
    /// heuristic permanently defers for this session (no output→Working
    /// promotion, no silence→Waiting, no stale→Active). Reset naturally on
    /// restart() via *self = new_session.
    explicit_seen: bool,
    /// Stage 13 follow-up: `Instant` of the most recent explicit OSC 1338
    /// frame. Drives the explicit-stale fuse in `tick()`. `None` until the
    /// first frame; refreshed on every `AiFrame`.
    last_explicit_at: Option<Instant>,
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
            explicit_seen: false,
            last_explicit_at: None,
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
            TrackerInput::AiFrame(frame) => {
                self.explicit_seen = true;
                self.last_explicit_at = Some(now);
                self.transition_to(frame.state.into(), now)
            }
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
                // Tier 3: if the foreground process is in the AI tools list (heuristic_active)
                // and we're not already tracking a state from an explicit signal, infer
                // Working from observed output activity. This is the "rapid output → working"
                // half of the Tier 3 heuristic; the Working → Waiting (silence) half lives
                // in `tick()`.
                //
                // Issue #7 (v0.1.4): `Waiting` is included as a source state, so resumed
                // output recovers a Tier-3 Waiting tab back to Working — the symmetric
                // counterpart to the silence rule, and the "output resumes" recovery that
                // replaces the removed stale-state demotion.
                if !self.explicit_seen
                    && self.heuristic_active
                    && matches!(
                        self.state,
                        TabState::Active | TabState::Idle | TabState::Waiting
                    )
                {
                    return self.transition_to(TabState::Working, now);
                }
                false
            }
        }
    }

    /// Stale-state and heuristic-silence checks at `now`. Returns `true` if a
    /// timeout caused a state change.
    pub fn tick(&mut self, now: Instant) -> bool {
        // Stage 13 Q1: explicit OSC 1338 (Tier-1) is authoritative. Once a
        // session self-reports, the Tier-3 heuristic-silence and stale-state
        // timeouts must NOT override the tool's reported state. (If a
        // self-reporting tool's hook dies mid-state the tab holds that state
        // until `config.explicit_stale_state` elapses — then the fuse below
        // de-escalates it. Set `explicit_stale_state = Duration::ZERO` to
        // restore the original hold-forever / "tool's bug" behaviour.)
        if self.explicit_seen {
            let stale = self.config.explicit_stale_state > Duration::ZERO
                && self
                    .last_explicit_at
                    .map(|l| now.saturating_duration_since(l) >= self.config.explicit_stale_state)
                    .unwrap_or(false);
            if !stale {
                return false; // still authoritative (Q1): inert
            }
            // Fuse fires: this session is no longer self-reporting.
            self.explicit_seen = false;
            self.last_explicit_at = None;
            match self.state {
                TabState::Working => {
                    // Dead hook mid-Working → recover to neutral. Direct-set
                    // `state` (bypassing `transition_to`) is deliberate: this
                    // is a fuse-tier signal that must not be debounce-suppressed
                    // (same rationale as the heuristic-silence block below).
                    self.state = TabState::Active;
                    self.last_event_at = Some(now);
                    return true;
                }
                TabState::Waiting => {
                    // Headline "needs you" cue PERSISTS — by design it stays
                    // amber until the user actually acts. Null the stale-state
                    // baseline:
                    // (a) prevents the now-ungated Tier-3 stale-state block from
                    //     silently reclaiming Waiting absent activity;
                    // (b) transition_to() treats last_event_at == None as the
                    //     "first transition" fast-path and skips the 100 ms
                    //     debounce — so when a qualifying signal DOES arrive it
                    //     takes effect immediately.
                    //
                    // Recovery contract (deliberately narrow): a de-escalated
                    // Waiting tab leaves amber ONLY on a real Tier-1 OSC 1338
                    // frame (next AI-tool turn) or an OSC 133 prompt marker.
                    // It is NOT recovered by raw shell output: the Tier-3
                    // OutputObserved promotion is gated to Active|Idle (it
                    // never promotes from Waiting), and a plain shell without
                    // OSC 133 prompt-marker integration emits no marker. So a
                    // formerly-AI tab that drops back to a bare bash shell can
                    // legitimately hold amber indefinitely — that is the
                    // intended "needs you, still unacknowledged" semantics, not
                    // a stuck state. (Enabling OSC 133 in the shell, like the
                    // OSC 1338 hook setup, restores prompt-driven recovery.)
                    //
                    // Invariant: Waiting + last_event_at == None means
                    // "de-escalated amber, pending a Tier-1/OSC-133 signal" —
                    // do NOT assume last_event_at is Some for Waiting.
                    self.last_event_at = None;
                    return false;
                }
                _ => {
                    // Active/Idle/Done: just de-escalate; fall through so
                    // Tier-3 (below) governs normally from here. A long-stale
                    // Idle/Done may then fire the stale-state block on THIS same
                    // tick (Idle/Done→Active) — intentional and neutral.
                }
            }
        }
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
        //
        // Issue #7 (v0.1.4): `Waiting` is exempt. On the Tier-3 heuristic path it
        // is the headline "needs you" cue and must persist until the AI resumes
        // output (recovered to `Working` in `on_input`) or the tool exits
        // (cleared in `set_heuristic_active`) — not silently demoted on a timer.
        // `Working`/`Idle`/`Done` keep the timeout. (The explicit OSC-1338 path
        // never reaches here while authoritative, and after its fuse fires the
        // `Waiting` arm nulls `last_event_at`, so this guard is also a no-op there.)
        if self.state != TabState::Active && self.state != TabState::Waiting {
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
    ///
    /// Returns `true` if this call changed the visual state. Issue #7 Change C:
    /// on the active→inactive edge (the AI tool exited), a Tier-3 `Waiting` tab
    /// is cleared to `Active` — the amber "needs you" cue is moot once the tool
    /// is gone. Explicit OSC-1338 sessions are exempt: their `Waiting`
    /// persistence is governed by the explicit-stale fuse, not the heuristic.
    pub fn set_heuristic_active(&mut self, active: bool) -> bool {
        let was_active = self.heuristic_active;
        self.heuristic_active = active;
        if was_active && !active && !self.explicit_seen && self.state == TabState::Waiting {
            // Direct-set (not `transition_to`): this is a fuse-tier signal that
            // must not be debounce-suppressed, matching the other direct-sets in
            // `tick()`. `last_event_at` is intentionally left as-is — `Active` is
            // exempt from the stale-state block, so the stale baseline is unused.
            self.state = TabState::Active;
            return true;
        }
        false
    }

    /// Update timing config at runtime. Used by hot-reload (`apply_config`).
    /// In-flight timers don't restart; subsequent `tick()` calls compare
    /// against the new thresholds.
    pub fn set_config(&mut self, config: TrackerConfig) {
        self.config = config;
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
        assert_eq!(c.explicit_stale_state, Duration::from_secs(300));
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
        // Enter Working via a shell prompt marker (Tier-3 / non-explicit). The stale-state timeout applies to non-self-reporting sessions;
        // sessions that emit OSC 1338 are exempt (Q1) — that path is covered by explicit_frame_disables_stale_timeout.
        t.on_input(TrackerInput::Prompt(PromptMarker::CommandStart), now);
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
        // Working entered non-explicitly (shell prompt) so the Tier-3 heuristic-silence inference still applies;
        // explicit OSC sessions are exempt (Q1).
        t.on_input(TrackerInput::Prompt(PromptMarker::CommandStart), now);
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
        // Working entered non-explicitly (shell prompt) so explicit_seen stays
        // false and tick()'s heuristic branch is actually reached. heuristic_active
        // is false (default) → the silence timer must NOT fire (the invariant under
        // test). (Explicit-OSC sessions are exempt for a different reason — Q1.)
        t.on_input(TrackerInput::Prompt(PromptMarker::CommandStart), now);
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
        // With the Tier 3 fix, OutputObserved on Active transitions → Working,
        // so the heuristic silence CAN fire afterwards. This test verifies the
        // full Tier 3 path: Active →(OutputObserved)→ Working →(silence)→ Waiting.
        // The "does not fire outside Working" invariant is preserved because the
        // transition goes *through* Working first.
        t.on_input(TrackerInput::OutputObserved, now);
        assert_eq!(
            t.state(),
            TabState::Working,
            "OutputObserved should have elevated to Working"
        );
        let changed = t.tick(now + Duration::from_secs(5));
        assert!(
            changed,
            "silence timer should fire after Working transition"
        );
        assert_eq!(t.state(), TabState::Waiting);
    }

    #[test]
    fn tracker_heuristic_silence_resets_on_new_output() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        t.set_heuristic_active(true);
        // Working via shell prompt (non-explicit) so explicit_seen stays false and
        // the heuristic-silence path in tick() is genuinely exercised. This test
        // pins the silence-baseline RESET on new output (heuristic_active=true).
        t.on_input(TrackerInput::Prompt(PromptMarker::CommandStart), now);
        // Output observed at now+3s — well within the 4 s silence window.
        t.on_input(TrackerInput::OutputObserved, now + Duration::from_secs(3));
        // Tick at now+5s — only 2 s of silence since last output. No fire.
        let changed = t.tick(now + Duration::from_secs(5));
        assert!(!changed);
        assert_eq!(t.state(), TabState::Working);
    }

    #[test]
    fn output_observed_triggers_working_when_heuristic_active() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        t.set_heuristic_active(true);
        assert_eq!(t.state(), TabState::Active);
        // First output observation should transition Active → Working when
        // heuristic_active is true. This is the "rapid output → working"
        // half of Tier 3 the original design spec called for.
        let changed = t.on_input(TrackerInput::OutputObserved, now);
        assert!(
            changed,
            "OutputObserved with heuristic_active should transition"
        );
        assert_eq!(t.state(), TabState::Working);
        // last_output_at also gets set, so the silence timer can fire.
        // (Verified indirectly: a tick past heuristic_silence should now transition to Waiting.)
        let _ = t.tick(now + Duration::from_millis(5000));
        assert_eq!(t.state(), TabState::Waiting);
    }

    #[test]
    fn output_observed_no_transition_when_heuristic_inactive() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        // heuristic_active = false (default).
        let changed = t.on_input(TrackerInput::OutputObserved, now);
        assert!(
            !changed,
            "OutputObserved without heuristic should not transition"
        );
        assert_eq!(t.state(), TabState::Active);
    }

    #[test]
    fn output_observed_does_not_override_working_or_waiting() {
        // OutputObserved must only promote from Active|Idle (the matches! guard
        // in on_input) — it must never override an existing Working or Waiting.
        // Both states are reached NON-explicitly here (shell prompt, then the
        // Tier-3 heuristic-silence inference) so explicit_seen stays false and
        // the matches! guard — not Q1's explicit gate — is what's under test.
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        t.set_heuristic_active(true);

        // --- Working: OutputObserved must not bump Working back through promotion.
        t.on_input(TrackerInput::Prompt(PromptMarker::CommandStart), now);
        assert_eq!(t.state(), TabState::Working);
        let _ = t.on_input(
            TrackerInput::OutputObserved,
            now + Duration::from_millis(100),
        );
        assert_eq!(
            t.state(),
            TabState::Working,
            "OutputObserved must not override Working"
        );

        // --- Waiting: drive Working→Waiting via the Tier-3 silence inference
        // (non-explicit), then OutputObserved must not pull it back.
        // tick fires Waiting after `heuristic_silence` (default 4s) of silence;
        // last OutputObserved was at +100ms, so tick at +5s is >4s of silence.
        let fired = t.tick(now + Duration::from_secs(5));
        assert!(
            fired,
            "heuristic-silence should infer Waiting (non-explicit)"
        );
        assert_eq!(t.state(), TabState::Waiting);
        let _ = t.on_input(TrackerInput::OutputObserved, now + Duration::from_secs(5));
        assert_eq!(
            t.state(),
            TabState::Waiting,
            "OutputObserved must not override Waiting"
        );
    }

    #[test]
    fn set_config_updates_heuristic_silence_threshold() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        t.set_heuristic_active(true);
        // Drive Working non-explicitly (shell prompt) — set_config tunes the Tier-3 silence threshold,
        // which only applies to non-self-reporting sessions (Q1).
        t.on_input(TrackerInput::Prompt(PromptMarker::CommandStart), now);
        assert_eq!(t.state(), TabState::Working);
        // Heuristic-silence path needs `last_output_at` to be set; otherwise
        // the `if let Some(last_out) = self.last_output_at` guard short-circuits
        // and tick() returns false even past the threshold. Inject an output
        // observation now so subsequent ticks compare against this baseline.
        t.on_input(TrackerInput::OutputObserved, now);
        // Reduce heuristic silence to 1000 ms.
        t.set_config(TrackerConfig {
            heuristic_silence: Duration::from_millis(1000),
            ..TrackerConfig::default()
        });
        // Tick at 800 ms — should NOT have transitioned (still under threshold).
        let changed_short = t.tick(now + Duration::from_millis(800));
        assert!(
            !changed_short,
            "should not transition within new 1000 ms threshold"
        );
        assert_eq!(t.state(), TabState::Working);
        // Tick at 1100 ms — should transition to Waiting.
        let changed_long = t.tick(now + Duration::from_millis(1100));
        assert!(changed_long, "should transition past new 1000 ms threshold");
        assert_eq!(t.state(), TabState::Waiting);
    }

    // --- Stage 13 Q1: explicit OSC 1338 (Tier-1) precedence tests ---

    #[test]
    fn explicit_frame_disables_heuristic_silence() {
        // Pre-fix: tick() would flip Working → Waiting on the silence timer even
        // after an explicit OSC 1338 frame had reported Working. Post-fix: once
        // explicit_seen is true, tick() returns false regardless.
        let mut t = AiStateTracker::new(TrackerConfig {
            heuristic_silence: Duration::from_millis(500),
            stale_state: Duration::from_secs(30),
            ..TrackerConfig::default()
        });
        let now = Instant::now();
        t.set_heuristic_active(true);
        // Feed an explicit OSC 1338 Working frame (sets explicit_seen = true).
        t.on_input(TrackerInput::AiFrame(Frame::new(State::Working)), now);
        // Also record an output observation so last_output_at is set and the
        // heuristic-silence branch has a baseline to compare against.
        t.on_input(TrackerInput::OutputObserved, now);
        assert_eq!(t.state(), TabState::Working);

        // Advance past heuristic_silence with no further input.
        let later = now + Duration::from_millis(600);
        let changed = t.tick(later);
        // Tier-1 is authoritative: heuristic must NOT have fired.
        assert!(!changed, "tick() must not fire when explicit_seen is true");
        assert_eq!(
            t.state(),
            TabState::Working,
            "state must remain Working (OSC-reported), not flip to Waiting"
        );
    }

    #[test]
    fn explicit_frame_disables_stale_timeout() {
        // Pre-fix: stale-state branch would flip Waiting → Active after stale_state.
        // Post-fix: once explicit_seen is true, tick() returns false regardless.
        let mut t = AiStateTracker::new(TrackerConfig {
            stale_state: Duration::from_millis(500),
            ..TrackerConfig::default()
        });
        let now = Instant::now();
        // Feed an explicit OSC 1338 Waiting frame (sets explicit_seen = true).
        t.on_input(TrackerInput::AiFrame(Frame::new(State::Waiting)), now);
        assert_eq!(t.state(), TabState::Waiting);

        // Advance past stale_state with no further input.
        let later = now + Duration::from_millis(600);
        let changed = t.tick(later);
        // Tier-1 is authoritative: stale-state must NOT have fired.
        assert!(!changed, "tick() must not fire when explicit_seen is true");
        assert_eq!(
            t.state(),
            TabState::Waiting,
            "state must remain Waiting (OSC-reported), not reset to Active"
        );
    }

    #[test]
    fn explicit_frame_disables_output_observed_promotion() {
        // Pre-fix: OutputObserved with heuristic_active would promote Active → Working
        // even after an explicit OSC 1338 Active frame. Post-fix: explicit_seen gates
        // the Tier-3 rapid-output→Working promotion.
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        t.set_heuristic_active(true);
        // Feed an explicit OSC 1338 Active frame (sets explicit_seen = true, state stays Active).
        let changed = t.on_input(TrackerInput::AiFrame(Frame::new(State::Active)), now);
        assert!(!changed, "Active→Active is not a state change");
        assert_eq!(t.state(), TabState::Active);

        // OutputObserved must NOT promote to Working because explicit_seen = true.
        let promoted = t.on_input(
            TrackerInput::OutputObserved,
            now + Duration::from_millis(200),
        );
        assert!(
            !promoted,
            "OutputObserved must not promote when explicit_seen is true"
        );
        assert_eq!(
            t.state(),
            TabState::Active,
            "state must remain Active (OSC-reported)"
        );
    }

    #[test]
    fn heuristic_still_works_without_any_explicit_frame() {
        // REGRESSION GUARD: explicit_seen starts false; all Tier-3 behavior must be
        // fully intact for sessions that never emit OSC 1338.
        let mut t = AiStateTracker::new(TrackerConfig {
            heuristic_silence: Duration::from_millis(500),
            stale_state: Duration::from_secs(30),
            ..TrackerConfig::default()
        });
        let now = Instant::now();
        t.set_heuristic_active(true);
        // No AiFrame ever — explicit_seen stays false.

        // OutputObserved from Active should promote → Working (Tier-3 output promotion).
        let promoted = t.on_input(TrackerInput::OutputObserved, now);
        assert!(
            promoted,
            "OutputObserved should promote Active→Working when explicit_seen is false"
        );
        assert_eq!(t.state(), TabState::Working);

        // Advance past heuristic_silence — Tier-3 silence → Waiting should fire.
        let t1 = now + Duration::from_millis(600);
        let silenced = t.tick(t1);
        assert!(
            silenced,
            "heuristic silence should fire Working→Waiting when explicit_seen is false"
        );
        assert_eq!(t.state(), TabState::Waiting);
    }

    #[test]
    fn explicit_stale_fuse_resets_working_to_active() {
        let mut t = AiStateTracker::new(TrackerConfig {
            explicit_stale_state: Duration::from_millis(500),
            ..TrackerConfig::default()
        });
        let now = Instant::now();
        t.on_input(TrackerInput::AiFrame(Frame::new(State::Working)), now);
        assert_eq!(t.state(), TabState::Working);
        assert!(!t.tick(now + Duration::from_millis(400)));
        assert_eq!(t.state(), TabState::Working);
        assert!(t.tick(now + Duration::from_millis(600)));
        assert_eq!(t.state(), TabState::Active);
    }

    #[test]
    fn explicit_stale_fuse_keeps_waiting_amber_but_dearms_explicit() {
        let mut t = AiStateTracker::new(TrackerConfig {
            explicit_stale_state: Duration::from_millis(500),
            stale_state: Duration::from_millis(50),
            ..TrackerConfig::default()
        });
        let now = Instant::now();
        t.on_input(TrackerInput::AiFrame(Frame::new(State::Waiting)), now);
        assert_eq!(t.state(), TabState::Waiting);
        assert!(!t.tick(now + Duration::from_millis(600)));
        assert_eq!(t.state(), TabState::Waiting);
        assert!(t.on_input(
            TrackerInput::Prompt(PromptMarker::CommandStart),
            now + Duration::from_millis(700),
        ));
        assert_eq!(t.state(), TabState::Working);

        let mut t2 = AiStateTracker::new(TrackerConfig {
            explicit_stale_state: Duration::from_millis(500),
            stale_state: Duration::from_millis(50),
            ..TrackerConfig::default()
        });
        let n2 = Instant::now();
        t2.on_input(TrackerInput::AiFrame(Frame::new(State::Waiting)), n2);
        assert!(!t2.tick(n2 + Duration::from_millis(600)));
        assert!(!t2.tick(n2 + Duration::from_millis(5000)));
        assert_eq!(
            t2.state(),
            TabState::Waiting,
            "amber must persist absent activity"
        );
    }

    #[test]
    fn explicit_stale_fuse_not_premature_when_frames_keep_arriving() {
        let mut t = AiStateTracker::new(TrackerConfig {
            explicit_stale_state: Duration::from_millis(500),
            ..TrackerConfig::default()
        });
        let now = Instant::now();
        t.on_input(TrackerInput::AiFrame(Frame::new(State::Working)), now);
        t.on_input(
            TrackerInput::AiFrame(Frame::new(State::Working)),
            now + Duration::from_millis(400),
        );
        assert!(!t.tick(now + Duration::from_millis(600)));
        assert_eq!(t.state(), TabState::Working);
    }

    #[test]
    fn explicit_stale_fuse_disabled_when_zero_keeps_q1_behavior() {
        let mut t = AiStateTracker::new(TrackerConfig {
            explicit_stale_state: Duration::ZERO,
            ..TrackerConfig::default()
        });
        let now = Instant::now();
        t.on_input(TrackerInput::AiFrame(Frame::new(State::Working)), now);
        assert!(!t.tick(now + Duration::from_secs(3600)));
        assert_eq!(t.state(), TabState::Working);
    }

    /// Helper: drive a Tier-3 (non-explicit) tracker into `Waiting` by arming the
    /// heuristic, observing output (→ Working), then ticking past the silence
    /// window (→ Waiting). Returns the `now` baseline used.
    fn tier3_into_waiting(t: &mut AiStateTracker) -> Instant {
        let now = Instant::now();
        t.set_heuristic_active(true);
        t.on_input(TrackerInput::OutputObserved, now);
        assert_eq!(t.state(), TabState::Working, "output should arm Working");
        assert!(t.tick(now + Duration::from_secs(5)), "silence should fire");
        assert_eq!(t.state(), TabState::Waiting, "silence → Waiting");
        now
    }

    // Issue #7 (v0.1.4): a Tier-3 (heuristic-detected, no OSC 1338) Waiting tab
    // is the headline "needs you" cue. It must NOT silently disappear after the
    // 30 s stale_state timeout the way other non-Active states do.
    #[test]
    fn tier3_waiting_persists_past_stale_state() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = tier3_into_waiting(&mut t);
        // Far past the 30 s stale_state, with no further input. Waiting holds.
        let changed = t.tick(now + Duration::from_secs(60));
        assert!(!changed, "stale_state must not demote Waiting");
        assert_eq!(t.state(), TabState::Waiting);
    }

    // Issue #7 recovery (user choice: "output resumes only"): when the AI tool
    // produces output again, a Tier-3 Waiting tab returns to Working — the
    // symmetric counterpart of the Working → Waiting silence rule.
    #[test]
    fn tier3_waiting_recovers_to_working_on_output() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = tier3_into_waiting(&mut t);
        let changed = t.on_input(TrackerInput::OutputObserved, now + Duration::from_secs(6));
        assert!(changed, "resumed output should recover Waiting → Working");
        assert_eq!(t.state(), TabState::Working);
    }

    // Issue #7 Change C: when the AI tool exits (heuristic flips active→inactive)
    // while a Tier-3 tab is Waiting, clear the amber — there's nothing left to
    // respond to. Prevents a stuck-amber-on-bare-shell state.
    #[test]
    fn tier3_waiting_clears_when_ai_tool_exits() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let _ = tier3_into_waiting(&mut t);
        let changed = t.set_heuristic_active(false);
        assert!(changed, "AI-tool exit should clear Tier-3 Waiting");
        assert_eq!(t.state(), TabState::Active);
    }

    // Change C is gated on `!explicit_seen`: an explicit OSC-1338 Waiting tab is
    // governed by the fuse, not the /proc heuristic, so a heuristic-inactive edge
    // must NOT clear it.
    #[test]
    fn explicit_waiting_not_cleared_by_heuristic_exit() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        t.set_heuristic_active(true);
        t.on_input(TrackerInput::AiFrame(Frame::new(State::Waiting)), now);
        assert_eq!(t.state(), TabState::Waiting);
        let changed = t.set_heuristic_active(false);
        assert!(
            !changed,
            "explicit Waiting must survive a heuristic-exit edge"
        );
        assert_eq!(t.state(), TabState::Waiting);
    }

    // Change C only clears `Waiting`; other Tier-3 states are untouched by a
    // heuristic-inactive edge (they keep their normal stale_state behavior).
    #[test]
    fn tier3_working_not_cleared_by_heuristic_exit() {
        let mut t = AiStateTracker::new(TrackerConfig::default());
        let now = Instant::now();
        t.set_heuristic_active(true);
        t.on_input(TrackerInput::OutputObserved, now); // Active → Working
        assert_eq!(t.state(), TabState::Working);
        let changed = t.set_heuristic_active(false);
        assert!(!changed, "heuristic exit must not clear non-Waiting states");
        assert_eq!(t.state(), TabState::Working);
    }
}
