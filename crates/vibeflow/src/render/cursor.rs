//! `CursorBlink` — 500 ms-period blink-state oracle. The renderer asks
//! `visible(now)` when building the active tab's cell instances; if `false`,
//! the cursor cell is drawn with the un-inverted (regular) fg/bg.

use std::time::Instant;

/// Blink period, in milliseconds. Half a second on / half off = 1 Hz blink.
pub const BLINK_PERIOD_MS: u128 = 500;

#[derive(Debug, Clone)]
pub struct CursorBlink {
    epoch: Instant,
}

impl CursorBlink {
    #[must_use]
    pub fn new() -> Self {
        Self {
            epoch: Instant::now(),
        }
    }

    /// True for the first half of every 1-second period.
    #[must_use]
    pub fn visible(&self, now: Instant) -> bool {
        let elapsed = now.duration_since(self.epoch).as_millis();
        (elapsed / BLINK_PERIOD_MS) % 2 == 0
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
}
