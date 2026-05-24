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

use crate::render::tabs::RectInstance;

/// Resolved palette for the About overlay. The renderer assembles this from
/// the active session's theme colours (or the engine fallbacks) before
/// calling `build_about_rects` / `build_about_glyphs`, so the about module
/// itself stays free of `alacritty_terminal` types.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AboutColors {
    /// Full-window translucent overlay behind the panel.
    pub backdrop: [f32; 4],
    /// Panel-body fill (opaque). Active theme's `NamedColor::Background`
    /// when set, else the engine fallback.
    pub panel_bg: [f32; 4],
    /// Panel border (1 rect per edge × 4 edges = 4 rects, 2 px thick each).
    pub border_fg: [f32; 4],
    /// Text colour for the five lines.
    pub text_fg: [f32; 4],
}

/// Build the per-frame rect list for the About overlay.
///
/// Order (DRAW ORDER — first rect paints first):
/// - 0. Full-window backdrop dim.
/// - 1. Panel body (centred via `panel_rect`).
/// - 2-5. Four 2-px border edges (top, bottom, left, right).
///
/// Pushed onto the master rect buffer in `render::mod.rs` AFTER the context
/// menu's rects so the panel sits on top of every other layer.
pub fn build_about_rects(window_size: (u32, u32), colors: &AboutColors) -> Vec<RectInstance> {
    const BORDER_PX: f32 = 2.0;
    let (px, py, pw, ph) = panel_rect(window_size);
    let window_w = window_size.0 as f32;
    let window_h = window_size.1 as f32;

    vec![
        // 0. Backdrop dim — full window.
        RectInstance::new(0.0, 0.0, window_w, window_h, colors.backdrop),
        // 1. Panel body.
        RectInstance::new(px, py, pw, ph, colors.panel_bg),
        // 2. Top border.
        RectInstance::new(px, py, pw, BORDER_PX, colors.border_fg),
        // 3. Bottom border.
        RectInstance::new(px, py + ph - BORDER_PX, pw, BORDER_PX, colors.border_fg),
        // 4. Left border.
        RectInstance::new(px, py, BORDER_PX, ph, colors.border_fg),
        // 5. Right border.
        RectInstance::new(
            px + pw - BORDER_PX,
            py,
            BORDER_PX,
            ph,
            colors.border_fg,
        ),
    ]
}

use crate::render::quad::QuadInstance;
use crate::render::text_engine::TextEngine;

/// Build the glyph quads for the About overlay's five text lines. Each
/// non-empty line is laid out horizontally centred within the panel's inner
/// padding box, vertically stacked with even spacing. Line 2 is intentionally
/// empty and contributes no glyphs.
///
/// Called from `Renderer::render` AFTER the context-menu glyph batch so the
/// overlay's text paints above every other glyph layer.
pub fn build_about_glyphs(
    window_size: (u32, u32),
    text_engine: &mut TextEngine,
    colors: &AboutColors,
) -> Vec<QuadInstance> {
    const INNER_PADDING_TOP: f32 = 16.0;
    const INNER_PADDING_BOTTOM: f32 = 16.0;
    const INNER_PADDING_X: f32 = 24.0;

    let (px, py, pw, ph) = panel_rect(window_size);
    let lines = about_lines();
    let line_count = lines.len() as f32;

    let inner_top = py + INNER_PADDING_TOP;
    let inner_h = (ph - INNER_PADDING_TOP - INNER_PADDING_BOTTOM).max(0.0);
    let line_pitch = if line_count > 0.0 {
        inner_h / line_count
    } else {
        0.0
    };

    let (cell_w, cell_h) = text_engine.cell_metrics();
    let cell_w_f = cell_w as f32;
    let cell_h_f = cell_h as f32;
    let inner_left = px + INNER_PADDING_X;
    let inner_right = px + pw - INNER_PADDING_X;
    let inner_width = (inner_right - inner_left).max(0.0);

    let mut glyphs: Vec<QuadInstance> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let text_w = line.chars().count() as f32 * cell_w_f;
        let line_x = inner_left + ((inner_width - text_w) / 2.0).max(0.0);
        // Vertical centre of this row.
        let row_top = inner_top + i as f32 * line_pitch;
        let line_y = row_top + ((line_pitch - cell_h_f) / 2.0).max(0.0);
        let max_x = (inner_right).floor() as u32;
        crate::render::tabs::push_text_glyphs(
            &mut glyphs,
            text_engine,
            line,
            (line_x, line_y),
            cell_w_f,
            colors.text_fg,
            colors.panel_bg,
            max_x,
        );
    }
    glyphs
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

    // ---- build_about_rects -----------------------------------------------

    fn test_colors() -> AboutColors {
        AboutColors {
            backdrop: [0.0, 0.0, 0.0, 0.5],
            panel_bg: [0.05, 0.05, 0.07, 1.0],
            border_fg: [0.9, 0.9, 0.9, 1.0],
            text_fg: [0.9, 0.9, 0.9, 1.0],
        }
    }

    #[test]
    fn build_about_rects_emits_backdrop_plus_panel_plus_four_borders() {
        let window = (1920_u32, 1080_u32);
        let rects = build_about_rects(window, &test_colors());
        // 1 backdrop + 1 panel body + 4 border edges = 6 rects.
        assert_eq!(rects.len(), 6, "expected 6 rects, got {}", rects.len());
    }

    #[test]
    fn build_about_rects_first_rect_is_full_window_backdrop() {
        let window = (1920_u32, 1080_u32);
        let rects = build_about_rects(window, &test_colors());
        let first = &rects[0];
        // RectInstance stores pos+size as [x, y, w, h] in `pos_size`.
        assert_eq!(first.pos_size, [0.0, 0.0, 1920.0, 1080.0]);
        assert_eq!(first.color, [0.0, 0.0, 0.0, 0.5]);
    }

    #[test]
    fn build_about_rects_panel_body_matches_panel_rect() {
        let window = (1920_u32, 1080_u32);
        let (px, py, pw, ph) = panel_rect(window);
        let rects = build_about_rects(window, &test_colors());
        let body = &rects[1];
        assert_eq!(body.pos_size, [px, py, pw, ph]);
        assert_eq!(body.color, [0.05, 0.05, 0.07, 1.0]);
    }

    #[test]
    fn build_about_rects_border_edges_use_border_color() {
        let rects = build_about_rects((1920, 1080), &test_colors());
        // Rects 2..6 are the four borders. Each must use border_fg.
        for (i, r) in rects.iter().enumerate().skip(2) {
            assert_eq!(
                r.color,
                [0.9, 0.9, 0.9, 1.0],
                "rect index {i} should use border colour"
            );
        }
    }

    // ---- build_about_glyphs (signature + invariant smoke) ----------------

    #[test]
    fn build_about_glyphs_signature_compiles_and_panel_metrics_consistent() {
        // We can't construct a real TextEngine here (needs a wgpu Device).
        // This test just pins the panel-metric invariants that the glyph
        // builder will rely on: panel width is large enough to hold the
        // canonical repo-URL line at a reasonable cell width.
        let (_, _, pw, _) = panel_rect((1920, 1080));
        let url_line = &about_lines()[3];
        let approx_min_cell_w = 6.0; // most fonts at default size are >= 6 px wide
        let needed_w = url_line.chars().count() as f32 * approx_min_cell_w;
        assert!(
            needed_w < pw,
            "panel width {pw} must fit url line ({needed_w} px at 6 px/cell)"
        );
    }
}
