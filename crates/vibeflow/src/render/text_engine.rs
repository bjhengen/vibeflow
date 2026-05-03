//! `TextEngine` — cosmic-text-backed glyph rasterizer + dynamic R8 glyph
//! atlas. Replaces the static fontdue atlas from Stage 5. Supports the full
//! Unicode range via cosmic-text's font fallback (system fonts via fontdb).
//!
//! Stage 7 ships monochrome (R8Unorm) only. Color-emoji rendering needs an
//! RGBA atlas + a dual-format sampling path — that's Stage 7.5.

#![allow(dead_code)] // first user is `Renderer` in Stage 7 Task 4

use anyhow::{Context, Result};
use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent};

/// Embedded primary font. Same JBM file used by Stage 5's fontdue atlas.
pub const PRIMARY_FONT: &[u8] = include_bytes!("../../assets/JetBrainsMono-Regular.ttf");

/// Stage 7 renders all glyphs at 16 px (matches Stage 5's `FONT_PX = 16.0`).
/// Configurable in Stage 9 (TOML config).
pub const FONT_PX: f32 = 16.0;

/// One rasterized glyph plus its placement metrics (relative to the cell origin).
#[derive(Debug, Clone)]
pub struct RasterImage {
    /// Width of the bitmap in pixels.
    pub width: u32,
    /// Height of the bitmap in pixels.
    pub height: u32,
    /// Offset from cell origin (cell top-left) to the bitmap top-left, in pixels.
    pub bearing_x: i32,
    pub bearing_y: i32,
    /// R8 alpha bytes, length = width * height.
    pub data: Vec<u8>,
}

/// Stateful cosmic-text wrapper. Heavyweight to construct (loads the embedded
/// font + system fonts via fontdb); cheap to query.
pub struct TextEngine {
    font_system: FontSystem,
    swash_cache: SwashCache,
    cell_w: u32,
    cell_h: u32,
    /// Baseline y-coordinate within a cell, in pixels from cell top.
    /// Read from cosmic-text's `LayoutRun::line_y` once at construction.
    baseline_y: u32,
}

impl TextEngine {
    /// Build a `TextEngine`. Loads the embedded JetBrainsMono and lets fontdb
    /// scan the user's system fonts for fallback (CJK, etc.). The cell metrics
    /// are derived from the primary font at `FONT_PX`.
    ///
    /// # Errors
    /// Fails if the embedded font is corrupt (which would mean a build bug).
    pub fn new() -> Result<Self> {
        // FontSystem::new() loads system fonts via fontdb. We add JBM after.
        let mut font_system = FontSystem::new();
        font_system.db_mut().load_font_data(PRIMARY_FONT.to_vec());
        let swash_cache = SwashCache::new();

        // Compute cell pitch from the primary font at FONT_PX. Shape a single
        // 'M' (the conventional widest monospace glyph) and read its advance.
        let metrics = Metrics::new(FONT_PX, FONT_PX * 1.4);
        let mut buffer = Buffer::new(&mut font_system, metrics);
        let attrs = Attrs::new().family(Family::Name("JetBrains Mono"));
        buffer.set_text(&mut font_system, "M", attrs, Shaping::Basic);
        buffer.shape_until_scroll(&mut font_system, false);

        let line = buffer
            .layout_runs()
            .next()
            .context("cosmic-text produced no layout runs for 'M'")?;
        let glyph = line
            .glyphs
            .first()
            .context("cosmic-text produced no glyphs for 'M'")?;
        let cell_w = glyph.w.ceil() as u32;
        let cell_h = (line.line_height).ceil() as u32;
        // `line.line_y` is the baseline y-coordinate within the line box, in
        // pixels. Stash it so glyph placement can position the bitmap
        // correctly relative to the baseline (NOT relative to cell top).
        let baseline_y = line.line_y.ceil() as u32;

        drop(buffer);

        Ok(Self {
            font_system,
            swash_cache,
            cell_w,
            cell_h,
            baseline_y,
        })
    }

    /// Per-cell pixel pitch (advance × line-height of the primary font).
    /// Stable for the life of the engine.
    #[must_use]
    pub fn cell_metrics(&self) -> (u32, u32) {
        (self.cell_w, self.cell_h)
    }

    /// Baseline y within a cell (pixels from cell top to baseline). Glyph
    /// placement: `screen_y_for_glyph_top = cell_top + baseline_y - bearing_y`
    /// where `bearing_y` is swash's `Placement::top` (positive for ascenders).
    #[must_use]
    pub fn baseline_y(&self) -> u32 {
        self.baseline_y
    }

    /// Rasterize a single character. Returns `Some` for any glyph that
    /// cosmic-text + the font stack can render; `None` only if every fallback
    /// font lacks coverage.
    ///
    /// Stage 7 returns R8 alpha. Color emoji glyphs (which swash returns as
    /// `SwashContent::Color`) are skipped — they're Stage 7.5 territory.
    pub fn rasterize(&mut self, c: char) -> Option<RasterImage> {
        let metrics = Metrics::new(FONT_PX, FONT_PX * 1.4);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let attrs = Attrs::new().family(Family::Name("JetBrains Mono"));
        buffer.set_text(&mut self.font_system, &c.to_string(), attrs, Shaping::Basic);
        buffer.shape_until_scroll(&mut self.font_system, false);

        let run = buffer.layout_runs().next()?;
        let glyph = run.glyphs.first()?;
        let physical = glyph.physical((0.0, 0.0), 1.0);
        let cache_key = physical.cache_key;

        let image = self
            .swash_cache
            .get_image(&mut self.font_system, cache_key)
            .as_ref()?;
        if image.content == SwashContent::Color {
            // Color emoji — Stage 7.5 task. Skip for now so the caller can
            // substitute a tofu glyph or fall back to '?'.
            return None;
        }

        Some(RasterImage {
            width: image.placement.width,
            height: image.placement.height,
            bearing_x: image.placement.left,
            bearing_y: image.placement.top,
            data: image.data.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_metrics_returns_jbm_pitch_at_16px() {
        let engine = TextEngine::new().unwrap();
        let (w, h) = engine.cell_metrics();
        // JBM Regular at 16 px: advance ≈ 9.6 → ceil 10; line at 1.4× ≈ 22.4 → ceil 23.
        // We don't pin exact values (cosmic-text's metrics may differ from fontdue's
        // by a pixel) but assert plausible bounds.
        assert!(
            (8..=14).contains(&w),
            "cell_w {} outside expected range 8..=14",
            w
        );
        assert!(
            (18..=28).contains(&h),
            "cell_h {} outside expected range 18..=28",
            h
        );
    }

    #[test]
    fn rasterize_ascii_letter_returns_image() {
        let mut engine = TextEngine::new().unwrap();
        let img = engine.rasterize('A').unwrap();
        assert!(img.width > 0);
        assert!(img.height > 0);
        assert_eq!(img.data.len(), (img.width * img.height) as usize);
        // Should have non-zero alpha somewhere.
        assert!(
            img.data.iter().any(|&a| a > 0),
            "rasterized 'A' is entirely transparent"
        );
    }

    #[test]
    fn rasterize_space_returns_none_or_empty_image() {
        let mut engine = TextEngine::new().unwrap();
        let img = engine.rasterize(' ');
        // cosmic-text returns either no image or an empty one for whitespace.
        if let Some(img) = img {
            assert_eq!(img.data.iter().filter(|&&a| a > 0).count(), 0);
        }
    }

    #[test]
    fn rasterize_cjk_uses_system_fallback() {
        let mut engine = TextEngine::new().unwrap();
        // 中 (U+4E2D) — JBM doesn't carry CJK. fontdb should find a system font.
        // If the test env has no CJK font, this returns None — assert either
        // outcome works, just that we don't panic.
        let _img = engine.rasterize('中');
    }
}
