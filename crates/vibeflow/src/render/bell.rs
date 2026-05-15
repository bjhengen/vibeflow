//! `BellFlash` — 200 ms white-tint fade triggered by the VTE BEL action
//! (`0x07`). The renderer reads `tint_alpha(now)` per frame and overlays a
//! full-window white rect (via `TabBarPipeline`) when the alpha is > 0.

use std::time::{Duration, Instant};

/// How long the bell flash takes to fade out.
pub const FLASH_DURATION: Duration = Duration::from_millis(200);
/// Peak white-tint alpha at flash start.
pub const FLASH_PEAK_ALPHA: f32 = 0.5;

#[derive(Debug, Clone, Default)]
pub struct BellFlash {
    last_bell: Option<Instant>,
}

impl BellFlash {
    #[must_use]
    pub fn new() -> Self {
        Self { last_bell: None }
    }

    /// Record a bell event.
    pub fn note(&mut self, now: Instant) {
        self.last_bell = Some(now);
    }

    /// Tint alpha at time `now`. Linear fade from `FLASH_PEAK_ALPHA` → 0.0
    /// over `FLASH_DURATION`. Returns 0.0 if no bell has fired or it's faded.
    #[must_use]
    pub fn tint_alpha(&self, now: Instant) -> f32 {
        let Some(start) = self.last_bell else {
            return 0.0;
        };
        let elapsed = now.duration_since(start);
        if elapsed >= FLASH_DURATION {
            return 0.0;
        }
        let t = elapsed.as_secs_f32() / FLASH_DURATION.as_secs_f32();
        FLASH_PEAK_ALPHA * (1.0 - t)
    }
}

/// Stage 13: play the system bell sound via `paplay`. Spawned detached;
/// never blocks the event loop. If `paplay` isn't installed, logs at debug
/// level. If the sound file is missing, `paplay` exits silently (stderr
/// discarded). Either way the event loop is never blocked.
pub fn play_audible_bell() {
    use std::process::{Command, Stdio};
    let result = Command::new("paplay")
        .arg("/usr/share/sounds/freedesktop/stereo/bell.oga")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Err(e) = result {
        tracing::debug!("paplay not available; audible bell skipped: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tint_alpha_zero_before_any_bell() {
        let bell = BellFlash::new();
        assert_eq!(bell.tint_alpha(Instant::now()), 0.0);
    }

    #[test]
    fn tint_alpha_peak_at_t_zero() {
        let mut bell = BellFlash::new();
        let t0 = Instant::now();
        bell.note(t0);
        let a = bell.tint_alpha(t0);
        assert!((a - FLASH_PEAK_ALPHA).abs() < 0.001);
    }

    #[test]
    fn tint_alpha_zero_after_duration() {
        let mut bell = BellFlash::new();
        let t0 = Instant::now();
        bell.note(t0);
        let after = t0 + FLASH_DURATION + Duration::from_millis(1);
        assert_eq!(bell.tint_alpha(after), 0.0);
    }

    #[test]
    fn tint_alpha_linear_at_midpoint() {
        let mut bell = BellFlash::new();
        let t0 = Instant::now();
        bell.note(t0);
        let mid = t0 + FLASH_DURATION / 2;
        let a = bell.tint_alpha(mid);
        assert!((a - FLASH_PEAK_ALPHA / 2.0).abs() < 0.01);
    }
}
