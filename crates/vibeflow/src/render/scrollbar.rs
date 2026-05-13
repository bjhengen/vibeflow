//! Stage 12: scrollbar fade-state + rect-build for the right-edge thumb.
//! Pure logic; no wgpu, no winit.

#![allow(dead_code)] // Renderer integration lands in Task 8; clean up there.

use std::time::Instant;

/// Color cache for the scrollbar track + thumb. Wired through Stage 9's
/// `[colors]` schema in Task 4; defaults to very faint white that's visible
/// on the dark Stage 9 background.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollbarColors {
    pub track: [f32; 4],
    pub thumb: [f32; 4],
}

impl Default for ScrollbarColors {
    fn default() -> Self {
        Self {
            track: [1.0, 1.0, 1.0, 0.04],
            thumb: [1.0, 1.0, 1.0, 0.22],
        }
    }
}

/// Per-session fade-state for the scrollbar thumb. Stays at α=0 when the
/// session hasn't been scrolled recently; pops to α=1 the instant a scroll
/// happens; fades linearly back to 0 over `fade_ms` after the last activity.
#[derive(Debug, Clone)]
pub struct ScrollbarFade {
    last_scroll_at: Option<Instant>,
    fade_ms: u64,
}

impl ScrollbarFade {
    pub fn new(fade_ms: u64) -> Self {
        Self {
            last_scroll_at: None,
            fade_ms,
        }
    }

    pub fn mark_scrolled(&mut self, now: Instant) {
        self.last_scroll_at = Some(now);
    }

    /// 1.0 at the instant of activity, linearly decreasing to 0.0 at fade_ms.
    /// Returns 0.0 if never scrolled or fade has elapsed.
    pub fn alpha(&self, now: Instant) -> f32 {
        let Some(last) = self.last_scroll_at else {
            return 0.0;
        };
        let elapsed_ms = now.saturating_duration_since(last).as_millis() as u64;
        if elapsed_ms >= self.fade_ms {
            0.0
        } else {
            1.0 - (elapsed_ms as f32 / self.fade_ms as f32)
        }
    }

    /// Update the fade duration at runtime. Existing in-progress fade keeps
    /// its baseline (last_scroll_at) and is re-evaluated on next alpha() call.
    pub fn set_fade_ms(&mut self, fade_ms: u64) {
        self.fade_ms = fade_ms;
    }
}

const TRACK_WIDTH_PX: f32 = 8.0;
const THUMB_MIN_HEIGHT_PX: f32 = 20.0;
const TRACK_INSET_PX: f32 = 1.0;

/// Build the rect instances for the scrollbar at the current state. Returns
/// empty when there's nothing to draw (fade at 0 OR no scrollback content).
///
/// `display_offset` is how many lines into history the viewport is scrolled.
/// `history_size` is total history rows available.
/// `screen_lines` is the visible viewport height in rows.
/// `surface_size` is the wgpu surface size in physical pixels.
/// `bar_height_px` is the tab strip height (scrollbar starts below it).
pub fn build_scrollbar_rects(
    fade_alpha: f32,
    display_offset: usize,
    history_size: usize,
    screen_lines: usize,
    surface_size: (f32, f32),
    bar_height_px: f32,
    colors: ScrollbarColors,
) -> Vec<crate::render::tabs::RectInstance> {
    if fade_alpha <= 0.0 {
        return Vec::new();
    }
    if display_offset == 0 && history_size == 0 {
        return Vec::new();
    }

    let (surface_w, surface_h) = surface_size;
    let track_x = surface_w - TRACK_WIDTH_PX;
    let track_y = bar_height_px;
    let track_h = (surface_h - bar_height_px).max(0.0);
    if track_h < THUMB_MIN_HEIGHT_PX {
        return Vec::new();
    }

    let total_lines = (history_size + screen_lines).max(1) as f32;
    let visible_lines = screen_lines as f32;
    let thumb_h = (visible_lines / total_lines * track_h)
        .max(THUMB_MIN_HEIGHT_PX)
        .min(track_h);

    let max_thumb_y_offset = track_h - thumb_h;
    let history_for_fraction = history_size.max(1) as f32;
    let scroll_fraction = (display_offset as f32 / history_for_fraction).min(1.0);
    // display_offset == 0 -> thumb at bottom of track.
    // display_offset == history_size -> thumb at top.
    let thumb_y = track_y + max_thumb_y_offset * (1.0 - scroll_fraction);

    let track_color = scale_alpha(colors.track, fade_alpha);
    let thumb_color = scale_alpha(colors.thumb, fade_alpha);

    vec![
        crate::render::tabs::RectInstance::new(
            track_x,
            track_y,
            TRACK_WIDTH_PX,
            track_h,
            track_color,
        ),
        crate::render::tabs::RectInstance::new(
            track_x + TRACK_INSET_PX,
            thumb_y,
            TRACK_WIDTH_PX - 2.0 * TRACK_INSET_PX,
            thumb_h,
            thumb_color,
        ),
    ]
}

fn scale_alpha(mut c: [f32; 4], factor: f32) -> [f32; 4] {
    c[3] *= factor;
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn fade_returns_zero_when_never_scrolled() {
        let f = ScrollbarFade::new(1500);
        assert_eq!(f.alpha(Instant::now()), 0.0);
    }

    #[test]
    fn fade_returns_one_at_scroll_instant() {
        let mut f = ScrollbarFade::new(1500);
        let now = Instant::now();
        f.mark_scrolled(now);
        assert_eq!(f.alpha(now), 1.0);
    }

    #[test]
    fn fade_decreases_linearly_to_zero() {
        let mut f = ScrollbarFade::new(1000);
        let now = Instant::now();
        f.mark_scrolled(now);
        let half = f.alpha(now + Duration::from_millis(500));
        assert!((half - 0.5).abs() < 0.05, "half-elapsed alpha = {half}");
        let done = f.alpha(now + Duration::from_millis(1100));
        assert_eq!(done, 0.0);
    }

    #[test]
    fn set_fade_ms_updates_threshold() {
        let mut f = ScrollbarFade::new(1000);
        let now = Instant::now();
        f.mark_scrolled(now);
        f.set_fade_ms(500);
        // Past the new threshold but under the old.
        assert_eq!(f.alpha(now + Duration::from_millis(600)), 0.0);
    }

    #[test]
    fn build_scrollbar_rects_empty_when_alpha_zero() {
        let rects = build_scrollbar_rects(
            0.0,
            100,
            5000,
            24,
            (800.0, 600.0),
            40.0,
            ScrollbarColors::default(),
        );
        assert!(rects.is_empty());
    }

    #[test]
    fn build_scrollbar_rects_empty_when_at_bottom_no_history() {
        let rects = build_scrollbar_rects(
            1.0,
            0,
            0,
            24,
            (800.0, 600.0),
            40.0,
            ScrollbarColors::default(),
        );
        assert!(rects.is_empty());
    }

    #[test]
    fn build_scrollbar_rects_returns_two_rects_for_normal_state() {
        let rects = build_scrollbar_rects(
            1.0,
            100,
            5000,
            24,
            (800.0, 600.0),
            40.0,
            ScrollbarColors::default(),
        );
        assert_eq!(rects.len(), 2);
    }

    #[test]
    fn build_scrollbar_rects_thumb_at_bottom_when_display_offset_zero() {
        let rects = build_scrollbar_rects(
            1.0,
            0,
            5000,
            24,
            (800.0, 600.0),
            40.0,
            ScrollbarColors::default(),
        );
        assert_eq!(rects.len(), 2);
        // RectInstance fields are (x, y, w, h, color). Track is rects[0]; thumb is rects[1].
        let track_y = rects[0].pos_size[1];
        let track_h = rects[0].pos_size[3];
        let thumb_y = rects[1].pos_size[1];
        let thumb_h = rects[1].pos_size[3];
        let track_bottom = track_y + track_h;
        let thumb_bottom = thumb_y + thumb_h;
        assert!(
            (thumb_bottom - track_bottom).abs() < 1.0,
            "thumb should sit at bottom of track when display_offset=0; track_bottom={track_bottom}, thumb_bottom={thumb_bottom}"
        );
    }

    #[test]
    fn build_scrollbar_rects_thumb_min_height_clamps() {
        let rects = build_scrollbar_rects(
            1.0,
            5000,
            10000,
            24,
            (800.0, 600.0),
            40.0,
            ScrollbarColors::default(),
        );
        let thumb_h = rects[1].pos_size[3];
        assert!(
            thumb_h >= THUMB_MIN_HEIGHT_PX,
            "thumb should be >= MIN_HEIGHT_PX (20); got {thumb_h}"
        );
    }

    #[test]
    fn build_scrollbar_rects_alpha_scales_track_and_thumb() {
        let rects = build_scrollbar_rects(
            0.5,
            100,
            5000,
            24,
            (800.0, 600.0),
            40.0,
            ScrollbarColors::default(),
        );
        let track_alpha = rects[0].color[3];
        let thumb_alpha = rects[1].color[3];
        // Defaults: track=0.04, thumb=0.22. At fade=0.5: track=0.02, thumb=0.11.
        assert!(
            (track_alpha - 0.02).abs() < 0.001,
            "track_alpha={track_alpha}"
        );
        assert!(
            (thumb_alpha - 0.11).abs() < 0.001,
            "thumb_alpha={thumb_alpha}"
        );
    }
}
