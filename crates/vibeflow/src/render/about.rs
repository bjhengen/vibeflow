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
}
