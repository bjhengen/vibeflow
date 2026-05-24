//! v0.1.2: About-overlay content + layout + rect/glyph builders.
//!
//! Mirrors `render/context_menu.rs`: pure layout + render builders live here;
//! input wiring and state (`WindowApp.about_open: bool`) live in `window.rs`.
//! No new render pass — the overlay's rects flow through `TabBarPipeline` and
//! the glyphs through `QuadPipeline`, both already in use by every other layer.

const TAGLINE: &str =
    "GPU-accelerated Linux terminal that knows when your AI tool is waiting on you.";
const LICENSE: &str = "Dual-licensed: MIT OR Apache-2.0";
const REPO_URL: &str = "https://github.com/bjhengen/vibeflow";

/// Five lines, ordered top→bottom. Line 1 is the intentional visual gap
/// between the version and the tagline — see the panel-height invariant
/// comments in `panel_rect`. Fixed `[String; 5]` so the layout math is
/// constant; tests pin both the count and the content invariants.
pub fn about_lines() -> [String; 5] {
    [
        format!("vibeflow {}", env!("CARGO_PKG_VERSION")),
        String::new(),
        TAGLINE.to_string(),
        format!("{LICENSE}  ·  {REPO_URL}"),
        "Press ESC, click outside, or click the panel to close".to_string(),
    ]
}

/// Returns `(x, y, w, h)` in logical pixels for the centred About panel.
///
/// Sizing rules (from the design spec §4.2):
/// - Default: 560×200 px.
/// - When the window is smaller than 600×240, clamp to `window - 40` on each
///   axis (20 px margin all sides).
/// - When the window is smaller than 200×120, drop to an 8 px margin so the
///   panel still has STRICTLY positive size on tiny windows.
/// - Zero-sized windows return a zero-sized panel rather than panicking
///   (defensive against transient zero sizes during resize).
pub fn panel_rect(window_size: (u32, u32)) -> (f32, f32, f32, f32) {
    const DEFAULT_W: f32 = 560.0;
    const DEFAULT_H: f32 = 200.0;
    const CLAMP_THRESHOLD_W: f32 = 600.0;
    const CLAMP_THRESHOLD_H: f32 = 240.0;
    const TINY_THRESHOLD_W: f32 = 200.0;
    const TINY_THRESHOLD_H: f32 = 120.0;
    const STANDARD_MARGIN: f32 = 20.0;
    const TINY_MARGIN: f32 = 8.0;

    let window_w = window_size.0 as f32;
    let window_h = window_size.1 as f32;

    let margin = if window_w < TINY_THRESHOLD_W || window_h < TINY_THRESHOLD_H {
        TINY_MARGIN
    } else {
        STANDARD_MARGIN
    };

    let w = if window_w < CLAMP_THRESHOLD_W {
        (window_w - 2.0 * margin).max(0.0)
    } else {
        DEFAULT_W
    };
    let h = if window_h < CLAMP_THRESHOLD_H {
        (window_h - 2.0 * margin).max(0.0)
    } else {
        DEFAULT_H
    };

    let x = ((window_w - w) / 2.0).max(0.0);
    let y = ((window_h - h) / 2.0).max(0.0);
    (x, y, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_lines_has_five_lines() {
        assert_eq!(about_lines().len(), 5);
    }

    #[test]
    fn about_lines_first_line_starts_with_vibeflow_and_includes_version() {
        let lines = about_lines();
        let first = &lines[0];
        assert!(
            first.starts_with("vibeflow "),
            "expected first line to start with `vibeflow `, got {first:?}"
        );
        assert!(
            first.contains(env!("CARGO_PKG_VERSION")),
            "expected first line to contain CARGO_PKG_VERSION, got {first:?}"
        );
    }

    #[test]
    fn about_lines_second_line_is_empty_visual_gap() {
        assert_eq!(about_lines()[1], "");
    }

    #[test]
    fn about_lines_includes_canonical_repo_url() {
        let lines = about_lines();
        let url_line = &lines[3];
        assert!(
            url_line.contains("https://github.com/bjhengen/vibeflow"),
            "expected canonical repo URL, got {url_line:?}"
        );
    }

    #[test]
    fn about_lines_dismissal_hint_mentions_esc() {
        let hint = &about_lines()[4];
        assert!(hint.contains("ESC"), "dismissal hint should mention ESC, got {hint:?}");
    }

    // ---- panel_rect -------------------------------------------------------

    #[test]
    fn panel_rect_centres_within_window_at_default_size() {
        // 1920×1080 has plenty of room for the default 560×200 panel.
        let (x, y, w, h) = panel_rect((1920, 1080));
        assert_eq!((w, h), (560.0, 200.0));
        // Centre of the panel == centre of the window.
        assert_eq!(x + w / 2.0, 1920.0 / 2.0);
        assert_eq!(y + h / 2.0, 1080.0 / 2.0);
    }

    #[test]
    fn panel_rect_clamps_in_small_window_below_600x240() {
        // 400×200 forces clamp: w = 400 - 40 = 360, h = 200 - 40 = 160.
        let (x, y, w, h) = panel_rect((400, 200));
        assert_eq!((w, h), (360.0, 160.0));
        assert_eq!(x, 20.0);
        assert_eq!(y, 20.0);
    }

    #[test]
    fn panel_rect_handles_tiny_window_lower_bound() {
        // 100×60 is below the 200×120 lower-bound floor; panel uses 8 px
        // margin all sides. Result must have STRICTLY positive w and h.
        let (x, y, w, h) = panel_rect((100, 60));
        assert!(w > 0.0, "w must be positive, got {w}");
        assert!(h > 0.0, "h must be positive, got {h}");
        assert!(x >= 0.0 && y >= 0.0, "origin must be non-negative");
        assert!(
            x + w <= 100.0 && y + h <= 60.0,
            "panel must fit inside the window"
        );
    }

    #[test]
    fn panel_rect_zero_sized_window_does_not_panic_and_returns_non_negative_w_h() {
        // Defensive: a zero-sized window is theoretically possible during
        // resize. The function must not panic and must return non-negative
        // dimensions so downstream rect-builder math stays sane.
        let (_x, _y, w, h) = panel_rect((0, 0));
        assert!(w >= 0.0, "w must be non-negative, got {w}");
        assert!(h >= 0.0, "h must be non-negative, got {h}");
    }
}
