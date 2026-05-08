//! `CursorBlink` — configurable-period blink-state oracle. The renderer asks
//! `visible(now)` when building the active tab's cell instances; if `false`,
//! the cursor cell is drawn with the un-inverted (regular) fg/bg.
//!
//! Stage 9: blink period is per-instance and live-configurable; a value of 0
//! disables blinking (cursor renders solid).

use std::time::Instant;

/// Default blink period, in milliseconds. Half a second on / half off = 1 Hz blink.
/// Used as the initial value for `CursorBlink::blink_ms`; live config can change it.
pub const BLINK_PERIOD_MS: u128 = 500;

#[derive(Debug, Clone)]
pub struct CursorBlink {
    epoch: Instant,
    /// Milliseconds per blink half-period. 0 disables blink (always visible).
    blink_ms: u64,
}

impl CursorBlink {
    #[must_use]
    pub fn new() -> Self {
        Self {
            epoch: Instant::now(),
            blink_ms: BLINK_PERIOD_MS as u64,
        }
    }

    /// Update the blink period at runtime. 0 disables blink.
    pub fn set_blink_ms(&mut self, ms: u64) {
        self.blink_ms = ms;
    }

    /// True for the first half of every period; always true if `blink_ms == 0`.
    #[must_use]
    pub fn visible(&self, now: Instant) -> bool {
        if self.blink_ms == 0 {
            return true;
        }
        let elapsed = now.duration_since(self.epoch).as_millis();
        (elapsed / (self.blink_ms as u128)) % 2 == 0
    }
}

impl Default for CursorBlink {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn visible_starts_true() {
        let cb = CursorBlink::new();
        assert!(cb.visible(cb.epoch));
    }

    #[test]
    fn visible_flips_at_500ms() {
        let cb = CursorBlink::new();
        let half = cb.epoch + Duration::from_millis(250);
        let just_after_500 = cb.epoch + Duration::from_millis(500);
        assert!(cb.visible(half));
        assert!(!cb.visible(just_after_500));
    }

    #[test]
    fn visible_flips_back_at_1000ms() {
        let cb = CursorBlink::new();
        let just_after_1000 = cb.epoch + Duration::from_millis(1000);
        assert!(cb.visible(just_after_1000));
    }

    #[test]
    fn visible_when_blink_ms_zero_is_always_true() {
        let mut cb = CursorBlink::new();
        cb.set_blink_ms(0);
        assert!(cb.visible(cb.epoch));
        assert!(cb.visible(cb.epoch + Duration::from_millis(500)));
        assert!(cb.visible(cb.epoch + Duration::from_millis(10_000)));
    }

    #[test]
    fn visible_with_custom_blink_ms() {
        let mut cb = CursorBlink::new();
        cb.set_blink_ms(250);
        assert!(cb.visible(cb.epoch + Duration::from_millis(100)));
        assert!(!cb.visible(cb.epoch + Duration::from_millis(300)));
        assert!(cb.visible(cb.epoch + Duration::from_millis(550)));
    }
}
