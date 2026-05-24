//! Per-knob bounds for user-supplied config values.
//!
//! Out-of-range values are clamped to the nearest bound, with a single
//! `tracing::warn!` naming the violated key + provided value + applied
//! bound. Config still loads — bound violations never block a load.
//!
//! Closes Codex review #8 (config-value bounds). See Spec B for the
//! per-knob rationale.

use std::fmt::Display;

/// Generic clamp + warn helper. Returns `min` if `value < min`, `max` if
/// `value > max`, otherwise returns `value` unchanged. On either clamp,
/// emits a `tracing::warn!` carrying the key name + the violating value
/// + the bound that was applied.
pub fn clamp_with_warn<T>(key: &str, value: T, min: T, max: T) -> T
where
    T: PartialOrd + Copy + Display,
{
    if value < min {
        tracing::warn!(
            key = %key, value = %value, min = %min,
            "config value below minimum; clamped to min"
        );
        min
    } else if value > max {
        tracing::warn!(
            key = %key, value = %value, max = %max,
            "config value above maximum; clamped to max"
        );
        max
    } else {
        value
    }
}

// ── AI section bounds (all u64 to match AiSection field types) ──────────
pub const AI_HEURISTIC_SILENCE_MS_MIN: u64 = 100;
pub const AI_HEURISTIC_SILENCE_MS_MAX: u64 = 60_000;
pub const AI_STALE_STATE_TIMEOUT_S_MIN: u64 = 5;
pub const AI_STALE_STATE_TIMEOUT_S_MAX: u64 = 3_600;
pub const AI_DEBOUNCE_MS_MIN: u64 = 50;
pub const AI_DEBOUNCE_MS_MAX: u64 = 10_000;
pub const AI_FOREGROUND_CHECK_INTERVAL_MS_MIN: u64 = 100;
pub const AI_FOREGROUND_CHECK_INTERVAL_MS_MAX: u64 = 30_000;
pub const AI_EXPLICIT_STALE_STATE_S_MIN: u64 = 10;
pub const AI_EXPLICIT_STALE_STATE_S_MAX: u64 = 3_600;

// ── Bell section bounds (u64) ───────────────────────────────────────────
pub const BELL_DEBOUNCE_MS_MIN: u64 = 50;
pub const BELL_DEBOUNCE_MS_MAX: u64 = 10_000;

// ── Scrollback section bounds (u32 for line-counts, u64 for ms) ─────────
pub const SCROLLBACK_HISTORY_LINES_MIN: u32 = 1;
pub const SCROLLBACK_HISTORY_LINES_MAX: u32 = 1_000_000;
pub const SCROLLBACK_WHEEL_LINES_PER_DETENT_MIN: u32 = 1;
pub const SCROLLBACK_WHEEL_LINES_PER_DETENT_MAX: u32 = 100;
pub const SCROLLBACK_SCROLLBAR_FADE_MS_MIN: u64 = 0;
pub const SCROLLBACK_SCROLLBAR_FADE_MS_MAX: u64 = 60_000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_below_min_returns_min() {
        assert_eq!(clamp_with_warn("test", 5_u64, 10, 100), 10);
    }

    #[test]
    fn clamp_above_max_returns_max() {
        assert_eq!(clamp_with_warn("test", 500_u64, 10, 100), 100);
    }

    #[test]
    fn clamp_within_range_returns_value_unchanged() {
        assert_eq!(clamp_with_warn("test", 50_u64, 10, 100), 50);
    }

    #[test]
    fn clamp_at_exact_min_returns_value() {
        assert_eq!(clamp_with_warn("test", 10_u64, 10, 100), 10);
    }

    #[test]
    fn clamp_at_exact_max_returns_value() {
        assert_eq!(clamp_with_warn("test", 100_u64, 10, 100), 100);
    }
}
