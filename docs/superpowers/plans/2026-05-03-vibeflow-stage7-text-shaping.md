# vibeflow Stage 7 Implementation Plan: cosmic-text font shaping + subtitle tint + cursor blink + bell flash

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `fontdue` with `cosmic-text` so vibeflow can render the full Unicode range with system-font fallback. Merge the cell-grid and text pipelines into a single `QuadPipeline` (per-instance atlas rect + screen rect, no more glyph-index addressing). Tint subtitle text by tracker state. Add a 500 ms cursor blink and a 200 ms bell visual flash.

**Architecture:**

- `crates/vibeflow/src/render/text_engine.rs` (new) — `TextEngine` owns a `cosmic_text::FontSystem`, a `cosmic_text::SwashCache`, a wgpu R8Unorm texture used as a dynamic glyph atlas, and a `HashMap<GlyphKey, AtlasRect>` cache. Public API: `cell_metrics() -> (u32, u32)`, `glyph_for(c: char) -> GlyphRef`, `view: &TextureView`, `sampler: &Sampler`. `glyph_for` is the hot path — cache hit is one hashmap lookup; cache miss synchronously rasterizes via swash, packs into the atlas (shelf packer), and returns the new rect.
- `crates/vibeflow/src/render/atlas.rs` — **deleted**. The Stage 5 `GlyphAtlas` (static 16×6 ASCII pre-render via fontdue) is fully superseded.
- `crates/vibeflow/src/render/grid.rs`, `crates/vibeflow/src/render/grid.wgsl` — **deleted**. The cell-coordinate-addressed pipeline merges into the unified quad pipeline.
- `crates/vibeflow/src/render/quad.rs` (new, replaces `text.rs`) — `QuadPipeline` renders per-instance textured quads. `QuadInstance` carries `screen_rect_px: [f32; 4]` (`x, y, w, h`), `atlas_rect_px: [f32; 4]` (`x, y, w, h`), `fg: [f32; 4]`, `bg: [f32; 4]`. The shader is the only piece that talks to the atlas texture. Used by cells, tab title/subtitle text, dead-tab banner.
- `crates/vibeflow/src/render/quad.wgsl` (new, replaces `text.wgsl`) — vertex expansion + fragment that mixes `bg → fg` by atlas alpha (R8Unorm `.r`).
- `crates/vibeflow/src/render/cursor.rs` (new) — `CursorBlink` keeps an `Instant` epoch and a `visible(now) -> bool` helper (`((now - epoch).as_millis() / 500) % 2 == 0`). The renderer skips drawing the cursor cell when `!visible`.
- `crates/vibeflow/src/render/bell.rs` (new) — `BellFlash` keeps `last_bell: Option<Instant>` and a `tint_alpha(now) -> f32` helper (linear fade from 0.5 → 0.0 over 200 ms). The renderer overlays a full-window white rect via `TabBarPipeline` when the alpha is non-zero.
- `crates/vibeflow/src/render/mod.rs` — `Renderer` swaps `atlas: GlyphAtlas` for `text_engine: TextEngine`, `grid_pipeline: GridPipeline` and `text_pipeline: TextPipeline` for a single `quad_pipeline: QuadPipeline`. Adds `cursor: CursorBlink` and `bell: BellFlash`. `render(...)` orchestration: bell flash detection (read from `App`), cell-grid pass via `QuadPipeline` (with cursor toggling), tab-bar pass via `TabBarPipeline`, tab text + dead-tab banner via `QuadPipeline`, optional bell-tint pass via `TabBarPipeline`.
- `crates/vibeflow/src/render/tabs.rs` — `TabBarRenderer::build_glyphs` switches from `glyph_index(c)` to `text_engine.glyph_for(c)`. Subtitle fg becomes `indicator_color(state)` (full alpha) when `state != Active`, otherwise `FG_ACTIVE`/`FG_INACTIVE` as today. The unused `_glyph_count` from Stage 6 is gone (banner code already cleaned up in commit `0adc62f`, but re-verify).
- `crates/vibeflow/src/session/session.rs` — `PtySession` parses VTE bell (`0x07`) actions via the existing `vte::ansi::Processor`. New `SessionEvent::Bell { tab_idx }` (added downstream by `App::poll_all`). The simplest plumbing: implement `Handler::bell` on a thin wrapper around our existing handler chain; the wrapper sets a `bell_pending: bool` field on `PtySession`; `poll(...)` drains the flag into a `SessionEvent::Bell`.
- `crates/vibeflow/src/window.rs` — `handle_session_event`'s arm for `SessionEvent::Bell` calls `renderer.note_bell()`. About-to-wait cadence stays at 16 ms while any tab is `Waiting`; otherwise the cursor blink + bell fade decide whether to redraw.

**Tech Stack:**
- `cosmic-text = "0.12"` (replaces `fontdue`).
- `cosmic-text` pulls in `swash`, `fontdb`, `rustybuzz`, `ttf-parser` transitively. Net dependency growth ~5 MB compiled.
- `wgpu = "0.20"` unchanged; texture format stays R8Unorm.
- No new fonts shipped (system fonts cover Unicode/CJK).

**Stage scope:** Stage 7 ends with: a vibeflow that renders any Unicode codepoint the user's installed fonts cover, with proper monospace cell width (CJK glyphs span two cells), per-state subtitle tint (amber for `waiting`, blue for `working`, gray for `idle`, etc.), a blinking block cursor on the active tab, and a brief white tint when the shell rings the bell. Color emoji renders as monochrome outlines (or tofu) — Stage 7.5 adds a parallel RGBA atlas path.

**Out of scope (deferred):**
- Color emoji rendering (RGBA atlas + dual-format sampler) — Stage 7.5.
- Subtitle italics for `Waiting`/`Working` state visibility — Stage 8 polish.
- Bidi text — Stage 8+.
- Programming ligatures (e.g. `==>`, `!=`) — Stage 8+ (cosmic-text supports them via `Shaping::Advanced`; we use `Shaping::Basic` for monospace correctness).
- Configurable font family — Stage 9 (TOML config).
- Configurable cursor style (block/bar/underline) — Stage 8.
- Configurable bell flash color/duration — Stage 9.
- Selection rendering on mouse drag — separate stage.
- Scrollback rendering on mouse wheel — separate stage.
- Hyperlinks — Stage 8+.

**Lessons carried forward from Stages 1–6:**
- Pre-execution senior review of plan code is high-value when introducing new dependency stacks. Stage 6's `pulse_alpha` math bug + 3 minor issues, Stage 5's three compile-blockers — all caught pre-dispatch. Run a Sonnet review pass on this plan before dispatching tasks.
- Per-task Haiku reviewers consistently miss whole-stage issues (Stage 6's senior review caught 2 IMPORTANT items missed by per-task review). Always run a final senior-tier holistic review before merging.
- Implementers will sometimes use refactor tasks to rewrite UNRELATED tests with fabricated justifications (Stage 5 Task 2). Compare test-name lists before/after every multi-file refactor: `git show <pre>:<file> | grep -E '^\s*fn '` vs `git show <post>:<file> | grep -E '^\s*fn '`. Add an explicit "DO NOT MODIFY OR DELETE EXISTING TESTS" guard to every refactor-task dispatch prompt — Stage 7 deletes `atlas.rs` and `grid.rs` outright, so the implementer needs to be precise about WHICH tests are intentionally removed (only `atlas::*` and `grid::*` tests; everything else stays).
- WGSL bugs only surface at runtime when `Renderer::new` calls the pipeline. Smoke run is the validation gate.
- For PTY tests: python3 + `bytes([...])` not dash printf.
- Uniform-buffer alignment math: count vec2/vec4/vec4<u32> bytes carefully and verify struct size is a multiple of 16. The Stage 6 lesson stands.
- Plan-verbatim Rust must be rustfmt-clean.
- intra-doc links: `[`Self::method`]` not `[`method`]`. Bare type-param syntax in doc comments (e.g. `vec4<f32>`) needs backticks: `` `vec4<f32>` ``.
- User smoke testing surfaces real bugs that escape every reviewer tier (Stage 6: spacebar + cell-grid clipping). Build the smoke checklist into the plan and exercise it before tagging.

---

## File Structure

| Path | Responsibility |
|---|---|
| `crates/vibeflow/Cargo.toml` (modify) | Replace `fontdue = "0.9"` with `cosmic-text = "0.12"`. |
| `crates/vibeflow/src/render/text_engine.rs` (new) | `TextEngine` (FontSystem + SwashCache + dynamic R8 atlas + glyph cache). ~400 LOC. |
| `crates/vibeflow/src/render/atlas.rs` (delete) | Static 16×6 ASCII fontdue atlas — superseded. |
| `crates/vibeflow/src/render/grid.rs` (delete) | Cell-coordinate pipeline — folded into `quad.rs`. |
| `crates/vibeflow/src/render/grid.wgsl` (delete) | Cell shader — folded into `quad.wgsl`. |
| `crates/vibeflow/src/render/text.rs` (rename to `quad.rs`) | `QuadPipeline` (per-instance textured quad: screen rect + atlas rect + fg + bg). ~250 LOC. |
| `crates/vibeflow/src/render/text.wgsl` (rename to `quad.wgsl`) | Quad shader. ~50 LOC. |
| `crates/vibeflow/src/render/cursor.rs` (new) | `CursorBlink { epoch: Instant }` + `visible(now)`. ~30 LOC. |
| `crates/vibeflow/src/render/bell.rs` (new) | `BellFlash { last_bell: Option<Instant> }` + `tint_alpha(now)` + `note_bell()`. ~40 LOC. |
| `crates/vibeflow/src/render/mod.rs` (modify) | Renderer field swap. New `render()` orchestration. ~150 LOC of net change. |
| `crates/vibeflow/src/render/tabs.rs` (modify) | `TabBarRenderer::build_glyphs` uses `text_engine.glyph_for`; subtitle tint per tracker state. |
| `crates/vibeflow/src/session/session.rs` (modify) | Bell-action plumbing. New `SessionEvent::Bell` variant. |
| `crates/vibeflow/src/app.rs` (modify) | `App::poll_all` propagates `SessionEvent::Bell` per tab. |
| `crates/vibeflow/src/window.rs` (modify) | Handle `SessionEvent::Bell` → `renderer.note_bell()`. About-to-wait cadence accounts for cursor blink + bell fade. |
| `assets/JetBrainsMono-Regular.ttf` | Unchanged; loaded into cosmic-text's FontSystem at startup. |
| `docs/TESTING.md` (extend) | Append Stage 7 manual smoke checklist. |

**Net delete:** `atlas.rs` + `grid.rs` + `grid.wgsl` (≈ 600 LOC).
**Net add:** `text_engine.rs` + `cursor.rs` + `bell.rs` (≈ 470 LOC).
**Net modify:** `quad.rs`/`quad.wgsl` (renamed, internal restructure ≈ 100 LOC delta), `mod.rs` (≈ 150 LOC), `tabs.rs` (≈ 50 LOC), `session.rs` + `app.rs` + `window.rs` (≈ 80 LOC).

---

## Task 0: Add cosmic-text dep + module declarations + stubs

**Files:**
- Modify: `crates/vibeflow/Cargo.toml`
- Modify: `crates/vibeflow/src/render/mod.rs`
- Create: `crates/vibeflow/src/render/text_engine.rs` (stub)
- Create: `crates/vibeflow/src/render/cursor.rs` (stub)
- Create: `crates/vibeflow/src/render/bell.rs` (stub)
- Rename: `crates/vibeflow/src/render/text.rs` → `crates/vibeflow/src/render/quad.rs`
- Rename: `crates/vibeflow/src/render/text.wgsl` → `crates/vibeflow/src/render/quad.wgsl`

This task only does the bookkeeping. The renames preserve git history (use `git mv`). No fontdue removal yet — Task 1 swaps it out. No `atlas.rs` / `grid.rs` deletion yet — Tasks 4 and 5 do those when their callers are gone.

- [ ] **Step 1: Add cosmic-text to Cargo.toml**

In `crates/vibeflow/Cargo.toml`, find the existing `fontdue = "0.9.3"` line in `[dependencies]` and add (do NOT yet remove fontdue — both must coexist until Task 1's TextEngine is ready):

```toml
cosmic-text = "0.12"
```

The actual remove of `fontdue` is at Task 4 Step 5 (when no caller remains).

- [ ] **Step 2: Update module declarations**

In `crates/vibeflow/src/render/mod.rs`, the current top-of-file module block (after Stage 6) is:

```rust
pub mod atlas;
pub mod colors;
pub mod grid;
pub mod tabs;
pub mod text;
```

Replace with:

```rust
pub mod atlas; // deleted in Task 4 Step 4 once grid.rs is also gone
pub mod bell;
pub mod colors;
pub mod cursor;
pub mod grid; // deleted in Task 4 Step 4
pub mod quad; // formerly `text` — see Step 3
pub mod tabs;
pub mod text_engine;
```

The `pub mod text;` line is removed because Step 3 renames it. The deletes for `atlas` and `grid` happen later — for now both still need to compile.

- [ ] **Step 3: Rename `text.{rs,wgsl}` → `quad.{rs,wgsl}`**

```bash
cd /home/bhengen/dev/vibeflow
git mv crates/vibeflow/src/render/text.rs crates/vibeflow/src/render/quad.rs
git mv crates/vibeflow/src/render/text.wgsl crates/vibeflow/src/render/quad.wgsl
```

Inside `crates/vibeflow/src/render/quad.rs`, fix the `include_str!("text.wgsl")` reference:

```rust
source: wgpu::ShaderSource::Wgsl(include_str!("quad.wgsl").into()),
```

(There's exactly one occurrence in `TextPipeline::new`. Search-and-replace `"text.wgsl"` → `"quad.wgsl"` and `"vibeflow-text-shader"` → `"vibeflow-quad-shader"` for the label as a stretch — but only if the label change feels safe. Plan-verbatim policy: change only the `include_str!` string for now; rename labels in Task 3 when the type name itself becomes `QuadPipeline`.)

In `crates/vibeflow/src/render/mod.rs`, fix every `crate::render::text::` reference to `crate::render::quad::`. There are several in `Renderer::new` and `Renderer::render`:

```rust
text_pipeline: crate::render::text::TextPipeline,
```
becomes
```rust
text_pipeline: crate::render::quad::TextPipeline,
```

(The struct name `TextPipeline` stays for now — Task 3 renames it to `QuadPipeline`.)

In `crates/vibeflow/src/render/tabs.rs`, fix the `use crate::render::text::GlyphInstance;` line:

```rust
use crate::render::quad::GlyphInstance;
```

- [ ] **Step 4: Stub the new files**

Create `crates/vibeflow/src/render/text_engine.rs`:

```rust
//! `TextEngine` — cosmic-text-backed glyph rasterizer + dynamic R8 glyph
//! atlas. Replaces the static fontdue atlas from Stage 5. Supports the full
//! Unicode range via cosmic-text's font fallback (system fonts via fontdb).
//!
//! Stage 7 ships monochrome (R8Unorm) only. Color-emoji rendering needs an
//! RGBA atlas + a dual-format sampling path — that's Stage 7.5.

#![allow(dead_code)] // first user is `Renderer` in Stage 7 Task 4
```

Create `crates/vibeflow/src/render/cursor.rs`:

```rust
//! `CursorBlink` — 500 ms-period blink-state oracle. The renderer asks
//! `visible(now)` when building the active tab's cell instances; if `false`,
//! the cursor cell is drawn with the un-inverted (regular) fg/bg.

#![allow(dead_code)] // first user is `Renderer` in Stage 7 Task 7
```

Create `crates/vibeflow/src/render/bell.rs`:

```rust
//! `BellFlash` — 200 ms white-tint fade triggered by the VTE BEL action
//! (`0x07`). The renderer reads `tint_alpha(now)` per frame and overlays a
//! full-window white rect (via `TabBarPipeline`) when the alpha is > 0.

#![allow(dead_code)] // first user is `Renderer` in Stage 7 Task 8
```

- [ ] **Step 5: Verify build + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo build -p vibeflow
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: clean build (`fontdue` and `cosmic-text` both pulled in, no callers of the new modules yet so `#[allow(dead_code)]` keeps clippy quiet).

- [ ] **Step 6: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/Cargo.toml crates/vibeflow/Cargo.lock \
        crates/vibeflow/src/render/mod.rs \
        crates/vibeflow/src/render/quad.rs \
        crates/vibeflow/src/render/quad.wgsl \
        crates/vibeflow/src/render/text_engine.rs \
        crates/vibeflow/src/render/cursor.rs \
        crates/vibeflow/src/render/bell.rs \
        crates/vibeflow/src/render/tabs.rs
git commit -m "chore(render): add cosmic-text dep, rename text→quad, stub text_engine/cursor/bell"
```

---

## Task 1: `TextEngine` foundation — cosmic-text wrapper + cell metrics (TDD)

**Files:**
- Modify: `crates/vibeflow/src/render/text_engine.rs`

This task implements the cosmic-text-backed half of `TextEngine`: load the embedded JetBrainsMono into a `FontSystem`, produce `cell_metrics()`, and expose a low-level `rasterize(c) -> RasterImage` that returns the bitmap + placement for any character. No wgpu atlas yet — that's Task 2.

- [ ] **Step 1: Define the public types**

Replace the contents of `crates/vibeflow/src/render/text_engine.rs` (currently the doc-comment stub):

```rust
//! `TextEngine` — cosmic-text-backed glyph rasterizer + dynamic R8 glyph
//! atlas. Replaces the static fontdue atlas from Stage 5. Supports the full
//! Unicode range via cosmic-text's font fallback (system fonts via fontdb).
//!
//! Stage 7 ships monochrome (R8Unorm) only. Color-emoji rendering needs an
//! RGBA atlas + a dual-format sampling path — that's Stage 7.5.

use std::collections::HashMap;

use anyhow::{Context, Result};
use cosmic_text::{
    Attrs, Buffer, CacheKey, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent,
};

/// Embedded primary font. Same JBM file used by Stage 5's fontdue atlas.
pub const PRIMARY_FONT: &[u8] = include_bytes!("../../../../assets/JetBrainsMono-Regular.ttf");

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
        let attrs = Attrs::new()
            .family(Family::Name("JetBrains Mono"))
            .monospaced(true);
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

        drop(buffer);

        Ok(Self {
            font_system,
            swash_cache,
            cell_w,
            cell_h,
        })
    }

    /// Per-cell pixel pitch (advance × line-height of the primary font).
    /// Stable for the life of the engine.
    #[must_use]
    pub fn cell_metrics(&self) -> (u32, u32) {
        (self.cell_w, self.cell_h)
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
        let attrs = Attrs::new()
            .family(Family::Name("JetBrains Mono"))
            .monospaced(true);
        buffer.set_text(
            &mut self.font_system,
            &c.to_string(),
            attrs,
            Shaping::Basic,
        );
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
```

Notes:
- `cosmic_text::Metrics::new(font_size, line_height)` — we use 1.4× font size for line height, matching common terminal conventions (Stage 5 used fontdue's reported `new_line_size`, which was ~1.35×).
- `Shaping::Basic` (not `Advanced`) skips programming ligatures so `==>` renders as three cells. Stage 8+ may switch to `Advanced` once the cell engine handles cluster widths.
- Use `Attrs::monospaced(true)` so cosmic-text picks monospace fonts when JBM isn't available.

- [ ] **Step 2: Add tests**

Append to the same file (above any `#[cfg(test)] mod tests` block; if none exists, add one):

```rust
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
```

- [ ] **Step 3: Verify**

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow --lib render::text_engine
cargo test -p vibeflow
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: 4 new tests pass; total lib count rises to 130 (post-Stage-6 was 126; Task 1 adds 4).

If `cosmic-text 0.12` produced a different API than this plan assumes, STOP and report — don't paper over with `#[allow(...)]`. The plan's API references (`Buffer`, `Metrics::new`, `Attrs::new().family().monospaced()`, `Shaping::Basic`, `swash_cache.get_image()`, `physical((0.0, 0.0), 1.0).cache_key`, `SwashContent::Color`) are all from cosmic-text 0.12.x. If 0.12 has been replaced, dial back the version in Cargo.toml.

- [ ] **Step 4: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/render/text_engine.rs
git commit -m "feat(render): TextEngine — cosmic-text wrapper, cell metrics, glyph rasterize (TDD)"
```

---

## Task 2: `TextEngine` dynamic R8 atlas — shelf packer + growable wgpu texture

**Files:**
- Modify: `crates/vibeflow/src/render/text_engine.rs`

`TextEngine` gains a wgpu R8Unorm texture, a shelf-packed allocator, and a `glyph_for(c) -> GlyphRef` API. Cache hit is a hashmap lookup; miss rasterizes via Task 1's path, copies into the atlas, returns the new rect.

The atlas starts at 256×256 px and grows by doubling height when shelves fill. Width stays at 256 (matches a 16×16 grid of 16-px-wide glyphs — plenty for ASCII; non-ASCII just uses more vertical space).

- [ ] **Step 1: Add atlas fields + GlyphRef**

In `crates/vibeflow/src/render/text_engine.rs`, add to the top imports:

```rust
use std::sync::Arc;
```

Add types after `RasterImage`:

```rust
/// Reference to a rasterized glyph in the atlas. Returned by `glyph_for`.
/// All fields in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlyphRef {
    pub atlas_x: u32,
    pub atlas_y: u32,
    pub atlas_w: u32,
    pub atlas_h: u32,
    /// Bearing relative to the cell top-left. Negative values mean the
    /// glyph starts to the left/above the cell origin.
    pub bearing_x: i32,
    pub bearing_y: i32,
}

/// Shelf packer state. Each shelf is a horizontal strip of fixed height
/// (the height of the tallest glyph placed in it). New glyphs go to the
/// active shelf if they fit; otherwise a new shelf opens below.
struct Shelf {
    y: u32,
    height: u32,
    next_x: u32,
}

const ATLAS_INITIAL_W: u32 = 256;
const ATLAS_INITIAL_H: u32 = 256;
```

Extend the `TextEngine` struct:

```rust
pub struct TextEngine {
    font_system: FontSystem,
    swash_cache: SwashCache,
    cell_w: u32,
    cell_h: u32,
    // Atlas state.
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    atlas_w: u32,
    atlas_h: u32,
    shelves: Vec<Shelf>,
    cache: HashMap<char, Option<GlyphRef>>, // None = unrenderable (e.g. color emoji), do not retry
    queue: Arc<wgpu::Queue>,
    device: Arc<wgpu::Device>,
}
```

(`Arc<Device>` and `Arc<Queue>` are passed in by `Renderer` so the engine can grow the texture without an external borrow gymnastics. `Renderer` already holds `Arc`s for these from Stage 4.)

- [ ] **Step 2: Refactor the constructor**

Replace `TextEngine::new()` with a version that takes the wgpu device + queue:

```rust
impl TextEngine {
    /// Build a `TextEngine`. Allocates the initial 256×256 R8Unorm atlas
    /// texture + sampler.
    ///
    /// # Errors
    /// Fails if the embedded font is corrupt or the wgpu allocator fails.
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Result<Self> {
        let mut font_system = FontSystem::new();
        font_system.db_mut().load_font_data(PRIMARY_FONT.to_vec());
        let swash_cache = SwashCache::new();

        let metrics = Metrics::new(FONT_PX, FONT_PX * 1.4);
        let mut buffer = Buffer::new(&mut font_system, metrics);
        let attrs = Attrs::new()
            .family(Family::Name("JetBrains Mono"))
            .monospaced(true);
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
        let cell_h = line.line_height.ceil() as u32;
        drop(buffer);

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vibeflow-text-engine-atlas"),
            size: wgpu::Extent3d {
                width: ATLAS_INITIAL_W,
                height: ATLAS_INITIAL_H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("vibeflow-text-engine-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Ok(Self {
            font_system,
            swash_cache,
            cell_w,
            cell_h,
            texture,
            view,
            sampler,
            atlas_w: ATLAS_INITIAL_W,
            atlas_h: ATLAS_INITIAL_H,
            shelves: Vec::new(),
            cache: HashMap::new(),
            queue,
            device,
        })
    }
```

- [ ] **Step 3: Implement `glyph_for` + atlas growth**

Add to `impl TextEngine`:

```rust
    /// Look up (or rasterize + atlas) the glyph for `c`. Returns `None` for
    /// characters the font stack can't render or color-emoji codepoints.
    /// The cache memoises both successes and failures.
    pub fn glyph_for(&mut self, c: char) -> Option<GlyphRef> {
        if let Some(cached) = self.cache.get(&c) {
            return *cached;
        }
        let result = self.try_atlas(c);
        self.cache.insert(c, result);
        result
    }

    fn try_atlas(&mut self, c: char) -> Option<GlyphRef> {
        let img = self.rasterize(c)?;
        if img.width == 0 || img.height == 0 {
            // Whitespace — record an "empty" rect at (0, 0) so the cache hit
            // path still works. The renderer will skip drawing zero-sized
            // quads via `if w * h > 0`.
            return Some(GlyphRef {
                atlas_x: 0,
                atlas_y: 0,
                atlas_w: 0,
                atlas_h: 0,
                bearing_x: 0,
                bearing_y: 0,
            });
        }
        let (x, y) = self.allocate(img.width, img.height);
        self.upload_to_atlas(x, y, img.width, img.height, &img.data);
        Some(GlyphRef {
            atlas_x: x,
            atlas_y: y,
            atlas_w: img.width,
            atlas_h: img.height,
            bearing_x: img.bearing_x,
            bearing_y: img.bearing_y,
        })
    }

    /// Shelf-pack: place a `w × h` rect into the atlas, growing as needed.
    fn allocate(&mut self, w: u32, h: u32) -> (u32, u32) {
        // Try existing shelves.
        if let Some(shelf) = self
            .shelves
            .iter_mut()
            .find(|s| s.height >= h && s.next_x + w <= self.atlas_w)
        {
            let x = shelf.next_x;
            let y = shelf.y;
            shelf.next_x += w;
            return (x, y);
        }
        // Open a new shelf at the bottom.
        let shelf_y = self
            .shelves
            .iter()
            .map(|s| s.y + s.height)
            .max()
            .unwrap_or(0);
        if shelf_y + h > self.atlas_h {
            self.grow_atlas(shelf_y + h);
        }
        self.shelves.push(Shelf {
            y: shelf_y,
            height: h,
            next_x: w,
        });
        (0, shelf_y)
    }

    /// Double the atlas height until the requested `min_height` fits.
    /// Allocates a new texture, copies the old contents, swaps fields.
    fn grow_atlas(&mut self, min_height: u32) {
        let mut new_h = self.atlas_h;
        while new_h < min_height {
            new_h *= 2;
        }
        let new_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vibeflow-text-engine-atlas"),
            size: wgpu::Extent3d {
                width: self.atlas_w,
                height: new_h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        // Copy old contents into the new texture.
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("vibeflow-atlas-grow-copy"),
            });
        encoder.copy_texture_to_texture(
            wgpu::ImageCopyTexture {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyTexture {
                texture: &new_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: self.atlas_w,
                height: self.atlas_h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(std::iter::once(encoder.finish()));

        self.texture = new_texture;
        self.view = self.texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.atlas_h = new_h;
        // The bind group in QuadPipeline is now stale — Renderer will need
        // to rebuild it. We expose `texture_dirty` so the caller can detect
        // growth and refresh the bind group.
    }

    fn upload_to_atlas(&self, x: u32, y: u32, w: u32, h: u32, data: &[u8]) {
        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(w),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Pixel size of the current atlas texture. Used by the shader's UV math.
    #[must_use]
    pub fn atlas_size(&self) -> (u32, u32) {
        (self.atlas_w, self.atlas_h)
    }

    /// True iff the atlas texture has been re-allocated since the last call.
    /// `QuadPipeline` polls this each frame to know when to rebuild its
    /// bind group. Resets the flag on read.
    pub fn texture_dirty(&mut self) -> bool {
        let dirty = self.atlas_dirty;
        self.atlas_dirty = false;
        dirty
    }
```

Add `atlas_dirty: bool` to the struct, set it `true` in `grow_atlas`, initialise to `false` in `new`. The flag is the simplest way to tell the caller "your bind group is now stale" without making `TextEngine` know about pipelines.

- [ ] **Step 4: Add tests**

Append to the `#[cfg(test)] mod tests` block:

```rust
    fn test_engine() -> TextEngine {
        // Use wgpu's null backend so tests don't need a GPU.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::SECONDARY, // null backend on Linux test runners
            flags: wgpu::InstanceFlags::default(),
            dx12_shader_compiler: wgpu::Dx12Compiler::default(),
            gles_minor_version: wgpu::Gles3MinorVersion::default(),
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: None,
            force_fallback_adapter: true,
        }))
        .expect("null adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor::default(),
            None,
        ))
        .expect("null device");
        TextEngine::new(Arc::new(device), Arc::new(queue)).unwrap()
    }

    #[test]
    fn glyph_for_caches_repeat_lookups() {
        let mut engine = test_engine();
        let r1 = engine.glyph_for('A').unwrap();
        let r2 = engine.glyph_for('A').unwrap();
        assert_eq!(r1, r2);
    }

    #[test]
    fn glyph_for_assigns_distinct_atlas_rects() {
        let mut engine = test_engine();
        let a = engine.glyph_for('A').unwrap();
        let b = engine.glyph_for('B').unwrap();
        // Different glyphs must occupy different rects.
        assert_ne!(
            (a.atlas_x, a.atlas_y),
            (b.atlas_x, b.atlas_y),
            "A and B got identical atlas positions"
        );
    }

    #[test]
    fn allocate_grows_atlas_when_shelves_fill() {
        let mut engine = test_engine();
        let initial_h = engine.atlas_size().1;
        // Force many distinct glyphs. cosmic-text needs each to be unique to
        // bypass the cache; ASCII letters give us 52 unique rasters which
        // should fit in the initial 256×256 atlas. Use Greek letters too to
        // exhaust shelf space.
        for c in 'A'..='Z' {
            engine.glyph_for(c);
        }
        for c in 'a'..='z' {
            engine.glyph_for(c);
        }
        for c in 'Α'..='Ω' {
            engine.glyph_for(c);
        }
        let (_, h_after) = engine.atlas_size();
        // Either fits in original size, or grew. Both are valid; we just
        // assert no panic and that the size is a power-of-two multiple.
        assert!(
            h_after >= initial_h && h_after % initial_h == 0,
            "atlas height {} is not a power-of-two multiple of {}",
            h_after,
            initial_h
        );
    }
```

You'll need `pollster = "0.3"` as a dev-dependency for `block_on`. Add it to `[dev-dependencies]` in `Cargo.toml` if not already present (Stage 4 added it for similar tests; check `Cargo.toml` first).

If creating a wgpu null adapter in CI is flaky on the test machine, gate the GPU-touching tests behind `#[cfg(feature = "gpu_tests")]` or fall back to `#[ignore]` with a comment that they're smoke-only. Plan-default: keep them on, mark `#[ignore]` only if the test suite breaks.

- [ ] **Step 5: Verify**

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow --lib render::text_engine
cargo test -p vibeflow
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: 4 (Task 1) + 3 (Task 2) = 7 text_engine tests pass; total lib count rises to 133.

If the wgpu null adapter setup fails on CI, mark the three GPU-touching tests as `#[ignore]` and re-run. The `cell_metrics_returns_jbm_pitch_at_16px` and `rasterize_*` tests from Task 1 don't need wgpu and should pass unconditionally.

- [ ] **Step 6: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/render/text_engine.rs crates/vibeflow/Cargo.toml crates/vibeflow/Cargo.lock
git commit -m "feat(render): TextEngine — dynamic R8 atlas + shelf packer + glyph_for cache (TDD)"
```

---

## Task 3: `QuadPipeline` — unified textured-quad pipeline (rewrite of `quad.rs`/`quad.wgsl`)

**Files:**
- Modify: `crates/vibeflow/src/render/quad.rs` (formerly `text.rs`)
- Modify: `crates/vibeflow/src/render/quad.wgsl` (formerly `text.wgsl`)

The Stage 6 `TextPipeline` took a glyph-index per instance + atlas-grid uniforms. Stage 7 throws that out: every quad carries its own `(screen_rect_px, atlas_rect_px, fg, bg)`. The shader is dumber and shorter — no atlas-grid math, no glyph-index lookup.

The struct is renamed from `TextPipeline` to `QuadPipeline` and `GlyphInstance` to `QuadInstance`. A type alias `pub type TextPipeline = QuadPipeline;` and `pub type GlyphInstance = QuadInstance;` smooths the migration for callers in this same task; later tasks remove the aliases.

- [ ] **Step 1: Rewrite `quad.wgsl`**

Replace the contents of `crates/vibeflow/src/render/quad.wgsl`:

```wgsl
// vibeflow Stage 7 unified quad shader. Replaces grid.wgsl + text.wgsl.
//
// Per-instance buffer carries:
//   .xyzw screen_rect_px (x, y, w, h in surface pixels)
//   .xyzw atlas_rect_px  (x, y, w, h in atlas pixels)
//   .rgba fg
//   .rgba bg
// Vertex shader expands 6 vertices per instance into a screen-space
// rectangle with linear UV across the atlas rect. Fragment shader samples
// R8Unorm `.r` as alpha and `mix(bg, fg, alpha)`.

struct QuadUniform {
    surface_size_px: vec2<f32>,
    atlas_size_px:   vec2<f32>,
};

@group(0) @binding(0) var<uniform> u: QuadUniform;
@group(0) @binding(1) var atlas_texture: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct VsIn {
    @builtin(vertex_index) vertex_id: u32,
    @location(0) screen_rect_px: vec4<f32>,
    @location(1) atlas_rect_px:  vec4<f32>,
    @location(2) fg:             vec4<f32>,
    @location(3) bg:             vec4<f32>,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv:             vec2<f32>,
    @location(1) fg:             vec4<f32>,
    @location(2) bg:             vec4<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var quad_offsets = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    let corner = quad_offsets[in.vertex_id];

    let screen_pos_px = in.screen_rect_px.xy + corner * in.screen_rect_px.zw;
    let ndc = (screen_pos_px / u.surface_size_px) * 2.0 - vec2<f32>(1.0, 1.0);
    let clip_pos = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);

    let atlas_pos_px = in.atlas_rect_px.xy + corner * in.atlas_rect_px.zw;
    let uv = atlas_pos_px / u.atlas_size_px;

    var out: VsOut;
    out.clip_pos = clip_pos;
    out.uv       = uv;
    out.fg       = in.fg;
    out.bg       = in.bg;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let alpha = textureSample(atlas_texture, atlas_sampler, in.uv).r;
    let rgb   = mix(in.bg.rgb, in.fg.rgb, alpha);
    return vec4<f32>(rgb, 1.0);
}
```

- [ ] **Step 2: Rewrite `quad.rs`**

Replace the contents of `crates/vibeflow/src/render/quad.rs`:

```rust
//! Unified textured-quad pipeline. Per-instance: screen rect + atlas rect +
//! fg + bg. Used for cell glyphs, tab title/subtitle text, and the dead-tab
//! banner. Stage 7 replaces both Stage 5's `GridPipeline` and Stage 6's
//! `TextPipeline` with this single pipeline.

use anyhow::Result;
use bytemuck::{Pod, Zeroable};

/// One textured quad. 64 bytes total.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct QuadInstance {
    /// Top-left + size in surface pixels.
    pub screen_rect_px: [f32; 4],
    /// Top-left + size in atlas pixels.
    pub atlas_rect_px: [f32; 4],
    pub fg: [f32; 4],
    pub bg: [f32; 4],
}

impl QuadInstance {
    #[must_use]
    pub fn new(
        screen_x: f32,
        screen_y: f32,
        screen_w: f32,
        screen_h: f32,
        atlas_x: f32,
        atlas_y: f32,
        atlas_w: f32,
        atlas_h: f32,
        fg: [f32; 4],
        bg: [f32; 4],
    ) -> Self {
        Self {
            screen_rect_px: [screen_x, screen_y, screen_w, screen_h],
            atlas_rect_px: [atlas_x, atlas_y, atlas_w, atlas_h],
            fg,
            bg,
        }
    }
}

/// 16-byte uniform: surface size + atlas size.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct QuadUniform {
    surface_size_px: [f32; 2],
    atlas_size_px: [f32; 2],
}

pub struct QuadPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    instance_capacity: u64,
}

const INITIAL_QUAD_CAPACITY: u64 = 80 * 24; // matches default Term size
const QUAD_STRIDE: u64 = std::mem::size_of::<QuadInstance>() as u64;

impl QuadPipeline {
    /// Build the pipeline. The `atlas_view` and `atlas_sampler` come from
    /// `TextEngine`; the bind group is rebuilt by `rebind_atlas` whenever
    /// the engine grows the texture.
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        atlas_view: &wgpu::TextureView,
        atlas_sampler: &wgpu::Sampler,
    ) -> Result<Self> {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vibeflow-quad-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("quad.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vibeflow-quad-bind-group-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vibeflow-quad-uniform"),
            size: std::mem::size_of::<QuadUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = Self::make_bind_group(
            device,
            &bind_group_layout,
            &uniform_buffer,
            atlas_view,
            atlas_sampler,
        );

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vibeflow-quad-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vibeflow-quad-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: QUAD_STRIDE,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x4,
                        },
                        wgpu::VertexAttribute {
                            offset: 16,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x4,
                        },
                        wgpu::VertexAttribute {
                            offset: 32,
                            shader_location: 2,
                            format: wgpu::VertexFormat::Float32x4,
                        },
                        wgpu::VertexAttribute {
                            offset: 48,
                            shader_location: 3,
                            format: wgpu::VertexFormat::Float32x4,
                        },
                    ],
                }],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vibeflow-quad-instances"),
            size: QUAD_STRIDE * INITIAL_QUAD_CAPACITY,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            pipeline,
            bind_group_layout,
            bind_group,
            uniform_buffer,
            instance_buffer,
            instance_capacity: INITIAL_QUAD_CAPACITY,
        })
    }

    fn make_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        uniform_buffer: &wgpu::Buffer,
        atlas_view: &wgpu::TextureView,
        atlas_sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vibeflow-quad-bind-group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(atlas_sampler),
                },
            ],
        })
    }

    /// Rebuild the bind group with a new atlas view (after `TextEngine` grew
    /// the texture). Caller polls `TextEngine::texture_dirty()` and calls
    /// this when it returns `true`.
    pub fn rebind_atlas(
        &mut self,
        device: &wgpu::Device,
        atlas_view: &wgpu::TextureView,
        atlas_sampler: &wgpu::Sampler,
    ) {
        self.bind_group = Self::make_bind_group(
            device,
            &self.bind_group_layout,
            &self.uniform_buffer,
            atlas_view,
            atlas_sampler,
        );
    }

    pub fn ensure_instance_capacity(&mut self, device: &wgpu::Device, needed: u64) {
        if needed <= self.instance_capacity {
            return;
        }
        let mut new_capacity = self.instance_capacity;
        while new_capacity < needed {
            new_capacity *= 2;
        }
        self.instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vibeflow-quad-instances"),
            size: QUAD_STRIDE * new_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.instance_capacity = new_capacity;
    }

    pub fn draw<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        queue: &wgpu::Queue,
        instances: &[QuadInstance],
        surface_size_px: (u32, u32),
        atlas_size_px: (u32, u32),
    ) {
        if instances.is_empty() {
            return;
        }
        let uniform = QuadUniform {
            surface_size_px: [surface_size_px.0 as f32, surface_size_px.1 as f32],
            atlas_size_px: [atlas_size_px.0 as f32, atlas_size_px.1 as f32],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
        queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(instances));

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
        pass.draw(0..6, 0..(instances.len() as u32));
    }
}

// Migration aliases. Removed in Tasks 4–5 once callers migrate.
#[deprecated(note = "use QuadPipeline directly")]
pub type TextPipeline = QuadPipeline;
#[deprecated(note = "use QuadInstance directly")]
pub type GlyphInstance = QuadInstance;
```

This file goes from ~250 lines (Stage 6) to ~250 lines but with a totally different shape. The aliases at the bottom let `Renderer::render` and `tabs::build_glyphs` keep building during the migration; Tasks 4 and 5 swap the call sites and Step 4 of Task 5 deletes the aliases.

- [ ] **Step 3: Verify build (no test changes — pipeline is GPU-validated by smoke run later)**

```bash
cd /home/bhengen/dev/vibeflow
cargo build -p vibeflow
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: clean. The `#[deprecated]` aliases produce warnings on usage; we accept those for one task, then remove them.

If clippy errors on deprecation warnings (`-D deprecated`), narrow the rejection: switch to `#[allow(deprecated)]` on `Renderer::render`'s call site temporarily, then remove the allow when the alias is gone.

- [ ] **Step 4: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/render/quad.rs crates/vibeflow/src/render/quad.wgsl
git commit -m "feat(render): unified QuadPipeline (rewrites text→quad with screen+atlas rects)"
```

---

## Task 4: Migrate cell rendering to `QuadPipeline` (delete `grid.rs`/`grid.wgsl`/`atlas.rs`/fontdue)

**Files:**
- Modify: `crates/vibeflow/src/render/mod.rs`
- Delete: `crates/vibeflow/src/render/grid.rs`, `crates/vibeflow/src/render/grid.wgsl`, `crates/vibeflow/src/render/atlas.rs`
- Modify: `crates/vibeflow/Cargo.toml` (remove `fontdue`)

This is the central migration: `Renderer::build_cell_instances` switches from `CellInstance` (cell coords + glyph idx) to `QuadInstance` (pixel coords + atlas rect). The `GridPipeline` field disappears; `Renderer` now owns one `QuadPipeline` (used by everyone) plus the `TabBarPipeline` for solid rects.

This task touches 250+ LOC in `mod.rs`. The implementer should READ the file before editing — Stage 6's render orchestration is the working starting point.

- [ ] **Step 1: Update `Renderer` struct + `Renderer::new`**

Read `crates/vibeflow/src/render/mod.rs`. Locate the `pub struct Renderer { ... }` block (post-Stage-6 it has `surface`, `device`, `queue`, `surface_config`, `atlas`, `grid_pipeline`, `text_pipeline`, `tab_bar_pipeline`, `tab_bar`).

Replace the field block with:

```rust
pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    surface_config: wgpu::SurfaceConfiguration,
    /// cosmic-text-backed glyph rasterizer + dynamic atlas. Replaces Stage 5's
    /// static `GlyphAtlas` (fontdue).
    text_engine: crate::render::text_engine::TextEngine,
    /// Unified textured-quad pipeline used for cells, tab text, and the
    /// dead-tab banner. Replaces Stage 5's `GridPipeline` + Stage 6's
    /// `TextPipeline`.
    quad_pipeline: crate::render::quad::QuadPipeline,
    /// Solid-color rectangle pipeline (tab backgrounds, indicator stripes,
    /// button bodies, bell flash overlay).
    tab_bar_pipeline: crate::render::tabs::TabBarPipeline,
    /// Tab-bar layout glue with pulse animation.
    tab_bar: crate::render::tabs::TabBarRenderer,
    /// Cursor blink state.
    cursor: crate::render::cursor::CursorBlink,
    /// Bell flash state.
    bell: crate::render::bell::BellFlash,
}
```

`Arc<Device>` and `Arc<Queue>` were already present in Stage 4 (passed into `GlyphAtlas::new`). If Stage 6 inadvertently broke the Arc setup, restore it: `let device = Arc::new(device);` and `let queue = Arc::new(queue);` after `request_device`.

In `Renderer::new`, replace the atlas + pipeline construction. The Stage 6 sequence was:

```rust
let atlas = GlyphAtlas::new(&device, &queue)?;
let grid_pipeline = GridPipeline::new(&device, format, &atlas)?;
let text_pipeline = crate::render::text::TextPipeline::new(&device, format, &atlas)?;
let tab_bar_pipeline = crate::render::tabs::TabBarPipeline::new(&device, format)?;
let tab_bar = crate::render::tabs::TabBarRenderer::new();
```

Replace with:

```rust
let text_engine =
    crate::render::text_engine::TextEngine::new(Arc::clone(&device), Arc::clone(&queue))?;
let quad_pipeline = crate::render::quad::QuadPipeline::new(
    &device,
    format,
    &text_engine.view,
    &text_engine.sampler,
)?;
let tab_bar_pipeline = crate::render::tabs::TabBarPipeline::new(&device, format)?;
let tab_bar = crate::render::tabs::TabBarRenderer::new();
let cursor = crate::render::cursor::CursorBlink::new();
let bell = crate::render::bell::BellFlash::new();
```

Update the `Ok(Self { ... })` block to use the new field names.

The `Renderer::cell_pitch()` accessor (added in Stage 6 Task 1) now reads from `text_engine`:

```rust
#[must_use]
pub fn cell_pitch(&self) -> (u32, u32) {
    self.text_engine.cell_metrics()
}
```

- [ ] **Step 2: Rewrite `Renderer::render` to use `QuadPipeline`**

Find the `Renderer::render` body. The current Stage 6 sequence is (rough outline):
1. `let (cell_w, cell_h) = self.atlas.cell_pitch();`
2. Begin render pass.
3. Cell-grid pass via `self.grid_pipeline.draw(... layout.bar_height_px)`.
4. Tab-bar pass via `self.tab_bar_pipeline`.
5. Tab text pass via `self.text_pipeline.draw(...)`.
6. Dead-tab banner via `self.tab_bar_pipeline` + `self.text_pipeline`.

Replace with (full body — paste plan-verbatim):

```rust
pub fn render(
    &mut self,
    term: Option<&alacritty_terminal::term::Term<alacritty_terminal::event::VoidListener>>,
    app: &crate::app::App,
) -> std::result::Result<(), wgpu::SurfaceError> {
    use crate::render::tabs::TabBarLayout;

    // Pull metrics + atlas size up front. `atlas_size` may have changed since
    // last frame if a glyph cache miss grew the texture.
    let (cell_w, cell_h) = self.text_engine.cell_metrics();
    let surface_size = (self.surface_config.width, self.surface_config.height);
    let layout = TabBarLayout::compute(surface_size.0, cell_h, app.tabs().len());
    let now = std::time::Instant::now();

    // Banner detection — same DRY pattern as Stage 6 commit 0adc62f.
    const BANNER_TEXT: &str = "session died -- press Ctrl+Shift+R to retry";
    let banner_glyph_count = app
        .tabs()
        .get(app.active())
        .filter(|s| !s.is_alive())
        .map(|_| {
            let count = BANNER_TEXT.chars().count();
            self.tab_bar_pipeline
                .ensure_instance_capacity(&self.device, 1);
            count
        });

    // Build per-pass instance lists OUTSIDE the render-pass scope so we can
    // call `&mut self` methods on the engine and pipelines without conflicting
    // with the render-pass borrow.
    let cell_instances = if let Some(term) = term {
        crate::render::quad::build_cell_instances(
            term,
            &mut self.text_engine,
            &self.cursor,
            now,
            cell_w,
            cell_h,
            layout.bar_height_px,
        )
    } else {
        Vec::new()
    };
    let tab_rects = self.tab_bar.build_rects(app, &layout);
    let tab_glyphs = self.tab_bar.build_glyphs(
        app,
        &layout,
        &mut self.text_engine,
    );
    let banner_quads = banner_glyph_count.map(|count| {
        crate::render::quad::build_banner_instances(
            BANNER_TEXT,
            count,
            &mut self.text_engine,
            cell_w,
            cell_h,
            &layout,
            surface_size.0,
        )
    });
    let bell_alpha = self.bell.tint_alpha(now);

    // Now grow the GPU buffers (still outside the render-pass scope).
    let atlas_size = self.text_engine.atlas_size();
    if self.text_engine.texture_dirty() {
        self.quad_pipeline.rebind_atlas(
            &self.device,
            &self.text_engine.view,
            &self.text_engine.sampler,
        );
    }
    let total_quads =
        cell_instances.len() + tab_glyphs.len() + banner_quads.as_ref().map_or(0, |v| v.len());
    self.quad_pipeline
        .ensure_instance_capacity(&self.device, total_quads as u64);
    self.tab_bar_pipeline
        .ensure_instance_capacity(&self.device, (tab_rects.len() + 2) as u64); // +2 = banner rect + bell

    let frame = self.surface.get_current_texture()?;
    let view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = self
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("vibeflow-frame-encoder"),
        });

    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("vibeflow-frame-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(CLEAR_COLOR),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        // ---- Cell grid pass ----
        if !cell_instances.is_empty() {
            self.quad_pipeline
                .draw(&mut pass, &self.queue, &cell_instances, surface_size, atlas_size);
        }

        // ---- Tab bar rects pass ----
        if !tab_rects.is_empty() {
            self.tab_bar_pipeline
                .draw(&mut pass, &self.queue, &tab_rects, surface_size);
        }

        // ---- Tab bar text pass ----
        if !tab_glyphs.is_empty() {
            self.quad_pipeline
                .draw(&mut pass, &self.queue, &tab_glyphs, surface_size, atlas_size);
        }

        // ---- Dead-tab banner ----
        if let Some(quads) = banner_quads {
            // Background rect first.
            let banner_h = (cell_h as f32) * 2.0;
            let banner_y = layout.bar_height_px as f32 + 16.0;
            let banner_w = surface_size.0 as f32;
            let banner_rect = crate::render::tabs::RectInstance::new(
                0.0,
                banner_y,
                banner_w,
                banner_h,
                [0.0, 0.0, 0.0, 0.85],
            );
            self.tab_bar_pipeline.draw(
                &mut pass,
                &self.queue,
                std::slice::from_ref(&banner_rect),
                surface_size,
            );
            self.quad_pipeline
                .draw(&mut pass, &self.queue, &quads, surface_size, atlas_size);
        }

        // ---- Bell flash overlay ----
        if bell_alpha > 0.0 {
            let bell_rect = crate::render::tabs::RectInstance::new(
                0.0,
                0.0,
                surface_size.0 as f32,
                surface_size.1 as f32,
                [1.0, 1.0, 1.0, bell_alpha],
            );
            self.tab_bar_pipeline.draw(
                &mut pass,
                &self.queue,
                std::slice::from_ref(&bell_rect),
                surface_size,
            );
        }
    }

    self.queue.submit(std::iter::once(encoder.finish()));
    frame.present();
    Ok(())
}
```

The new `build_cell_instances` and `build_banner_instances` helpers live in `quad.rs` (Step 3 below). The Stage 6 free function `build_cell_instances` (in `mod.rs`) is replaced by the `quad.rs` version that uses `text_engine`.

Add a public method on `Renderer` for the bell event (used by `window.rs`):

```rust
/// Note that the active tab's session rang the bell. Triggers a 200 ms
/// white-tint fade.
pub fn note_bell(&mut self) {
    self.bell.note(std::time::Instant::now());
}
```

- [ ] **Step 3: Add `build_cell_instances` + `build_banner_instances` to `quad.rs`**

Append to `crates/vibeflow/src/render/quad.rs`:

```rust
use crate::render::cursor::CursorBlink;
use crate::render::text_engine::{GlyphRef, TextEngine};

/// Walk the active grid and emit one [`QuadInstance`] per visible cell.
/// Skips cells whose glyph is unrenderable (`text_engine.glyph_for` returned
/// `None`) — those become invisible cells (background still drawn via the
/// shared bg pass). Toggles the cursor cell based on `CursorBlink::visible`.
pub fn build_cell_instances(
    term: &alacritty_terminal::term::Term<alacritty_terminal::event::VoidListener>,
    text_engine: &mut TextEngine,
    cursor: &CursorBlink,
    now: std::time::Instant,
    cell_w: u32,
    cell_h: u32,
    y_offset_px: u32,
) -> Vec<QuadInstance> {
    use alacritty_terminal::index::Point;
    use crate::render::colors::resolve_color;

    let cursor_visible = cursor.visible(now);
    let content = term.renderable_content();
    let cursor_pos = Point::new(content.cursor.point.line, content.cursor.point.column);
    let colors = content.colors;
    let fg_default = [0.9_f32, 0.9, 0.9, 1.0];
    let bg_default = [0.05_f32, 0.05, 0.07, 1.0];

    let mut out = Vec::new();
    for cell in content.display_iter {
        let line = cell.point.line.0 as i32;
        let col = cell.point.column.0 as u32;
        if line < 0 {
            continue;
        }
        let row = line as u32;

        let mut fg = resolve_color(cell.fg, &colors, fg_default, bg_default);
        let mut bg = resolve_color(cell.bg, &colors, bg_default, fg_default);
        let is_cursor = cell.point == cursor_pos;
        if is_cursor && cursor_visible {
            std::mem::swap(&mut fg, &mut bg);
        }

        let screen_x = (col * cell_w) as f32;
        let screen_y = (row * cell_h + y_offset_px) as f32;

        let glyph = text_engine.glyph_for(cell.c).unwrap_or(GlyphRef {
            atlas_x: 0,
            atlas_y: 0,
            atlas_w: 0,
            atlas_h: 0,
            bearing_x: 0,
            bearing_y: 0,
        });

        // Always emit a background rect by drawing a quad with bg=bg, fg=bg
        // and a zero-size atlas rect (so the alpha is 0 and the result is
        // pure bg). Then emit a second quad for the glyph itself.
        out.push(QuadInstance::new(
            screen_x,
            screen_y,
            cell_w as f32,
            cell_h as f32,
            0.0,
            0.0,
            0.0,
            0.0,
            bg,
            bg,
        ));
        if glyph.atlas_w > 0 && glyph.atlas_h > 0 {
            out.push(QuadInstance::new(
                screen_x + glyph.bearing_x as f32,
                screen_y + (cell_h as f32 - glyph.bearing_y as f32),
                glyph.atlas_w as f32,
                glyph.atlas_h as f32,
                glyph.atlas_x as f32,
                glyph.atlas_y as f32,
                glyph.atlas_w as f32,
                glyph.atlas_h as f32,
                fg,
                bg,
            ));
        }
    }
    out
}

/// Build the dead-tab banner's centered text quads.
pub fn build_banner_instances(
    text: &str,
    glyph_count: usize,
    text_engine: &mut TextEngine,
    cell_w: u32,
    cell_h: u32,
    layout: &crate::render::tabs::TabBarLayout,
    surface_w: u32,
) -> Vec<QuadInstance> {
    let banner_h = (cell_h as f32) * 2.0;
    let banner_y = layout.bar_height_px as f32 + 16.0;
    let banner_w = surface_w as f32;
    let text_w = (glyph_count as f32) * (cell_w as f32);
    let text_x = (banner_w - text_w) / 2.0;
    let text_y = banner_y + (banner_h - cell_h as f32) / 2.0;

    let amber: [f32; 4] = [
        0xff as f32 / 255.0,
        0xbd as f32 / 255.0,
        0x2e as f32 / 255.0,
        1.0,
    ];
    let black: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

    let mut out = Vec::with_capacity(glyph_count);
    let mut x = text_x;
    for c in text.chars() {
        if let Some(glyph) = text_engine.glyph_for(c) {
            if glyph.atlas_w > 0 && glyph.atlas_h > 0 {
                out.push(QuadInstance::new(
                    x + glyph.bearing_x as f32,
                    text_y + (cell_h as f32 - glyph.bearing_y as f32),
                    glyph.atlas_w as f32,
                    glyph.atlas_h as f32,
                    glyph.atlas_x as f32,
                    glyph.atlas_y as f32,
                    glyph.atlas_w as f32,
                    glyph.atlas_h as f32,
                    amber,
                    black,
                ));
            }
        }
        x += cell_w as f32;
    }
    out
}
```

The cursor logic (swap fg/bg on cursor cell when visible) is folded into the cell builder. `CursorBlink` is purely state — see Task 7.

The two-quads-per-cell trick (background quad + glyph quad) keeps the shader simple. For cells with no glyph (e.g. spaces), only the background quad is emitted.

- [ ] **Step 4: Delete `grid.rs`, `grid.wgsl`, `atlas.rs`**

```bash
cd /home/bhengen/dev/vibeflow
git rm crates/vibeflow/src/render/grid.rs
git rm crates/vibeflow/src/render/grid.wgsl
git rm crates/vibeflow/src/render/atlas.rs
```

In `crates/vibeflow/src/render/mod.rs`, remove the now-stale module declarations:

```rust
pub mod atlas; // delete this line
pub mod grid;  // delete this line
```

Also remove any free `build_cell_instances` function in `mod.rs` — its replacement lives in `quad.rs`.

If anything still references `crate::render::atlas::*` or `crate::render::grid::*`, the compile error will pinpoint it. The only known references after Stage 6 were:
- `crate::render::atlas::{glyph_index, GlyphAtlas}` in `tabs.rs` — Task 5 swaps these to `text_engine.glyph_for`
- `crate::render::atlas::ATLAS_LAYOUT` in `mod.rs` — gone (no atlas grid in Stage 7)
- `crate::render::grid::CellInstance` — gone

If the implementer hits a reference outside Tasks 4–5's scope, STOP and report.

- [ ] **Step 5: Remove `fontdue`**

In `crates/vibeflow/Cargo.toml`, delete the `fontdue = "0.9.3"` line. `Cargo.lock` will update on next build.

- [ ] **Step 6: Verify build (cell rendering should now work end-to-end via `QuadPipeline`)**

```bash
cd /home/bhengen/dev/vibeflow
cargo build --bin vibeflow
cargo test -p vibeflow
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: clean. Test count drops by however many tests `atlas.rs` and `grid.rs` had (Stage 5 added 0 tests in atlas.rs, 0 in grid.rs — both modules are pure GPU code without unit tests). Total lib stays at 133 (post-Task-2).

If `cargo test` fails because some Stage 5/6 test referenced `atlas::glyph_index` or `grid::CellInstance`, STOP and report — the cited test should have already been migrated. Stage 6 didn't add any such tests; the only callers were inside `tabs.rs` and `mod.rs`.

The `cell_pitch_with_real_jbm_metrics` test in `window.rs` (Task 1 of Stage 6) is unaffected — it tests `pixels_to_grid` which is pitch-agnostic.

- [ ] **Step 7: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add -A
git commit -m "feat(render): migrate cell grid to QuadPipeline + delete fontdue/atlas/grid"
```

---

## Task 5: Migrate `tabs.rs` to `text_engine.glyph_for` + drop deprecated aliases

**Files:**
- Modify: `crates/vibeflow/src/render/tabs.rs`
- Modify: `crates/vibeflow/src/render/quad.rs` (remove deprecation aliases)
- Modify: `crates/vibeflow/src/render/mod.rs` (drop `_text_pipeline` field if any holdover)

After Task 4, `Renderer` has one `quad_pipeline` and zero `text_pipeline`. Tab text + close-button glyphs + the `+` glyph all need migrating from the Stage 6 ASCII atlas (`glyph_index(c)`) to `text_engine.glyph_for(c)`.

- [ ] **Step 1: Update `TabBarRenderer::build_glyphs` signature**

Find `pub fn build_glyphs(&self, app: &App, layout: &TabBarLayout, atlas: &GlyphAtlas) -> Vec<GlyphInstance>`. Change to:

```rust
pub fn build_glyphs(
    &self,
    app: &App,
    layout: &TabBarLayout,
    text_engine: &mut crate::render::text_engine::TextEngine,
) -> Vec<crate::render::quad::QuadInstance>
```

- [ ] **Step 2: Migrate the body**

The Stage 6 body computed pixel positions per character via `cell_w_f` × char index, then pushed `GlyphInstance::new(x, y, glyph_idx, fg, bg)` with `glyph_idx = glyph_index(c).unwrap_or(0)`.

Replace with `text_engine.glyph_for(c)` returning a `GlyphRef`, then push `QuadInstance::new(screen_x + bearing_x, screen_y + (cell_h - bearing_y), atlas_w, atlas_h, atlas_x, atlas_y, atlas_w, atlas_h, fg, bg)`.

The helper `push_text_glyphs` from Stage 6 changes signature too:

```rust
fn push_text_glyphs(
    out: &mut Vec<crate::render::quad::QuadInstance>,
    text_engine: &mut crate::render::text_engine::TextEngine,
    s: &str,
    pos: (f32, f32),
    cell_w: f32,
    cell_h: f32,
    fg: [f32; 4],
    bg: [f32; 4],
    max_x_px: u32,
) {
    let (x_start, y) = pos;
    let mut x = x_start;
    for c in s.chars() {
        if x + cell_w > max_x_px as f32 {
            break;
        }
        if let Some(g) = text_engine.glyph_for(c) {
            if g.atlas_w > 0 && g.atlas_h > 0 {
                out.push(crate::render::quad::QuadInstance::new(
                    x + g.bearing_x as f32,
                    y + (cell_h - g.bearing_y as f32),
                    g.atlas_w as f32,
                    g.atlas_h as f32,
                    g.atlas_x as f32,
                    g.atlas_y as f32,
                    g.atlas_w as f32,
                    g.atlas_h as f32,
                    fg,
                    bg,
                ));
            }
        }
        x += cell_w;
    }
}
```

The `cell_h` parameter is new — needed for baseline positioning.

The `+` and `×` button glyphs change too: the Stage 6 code called `glyph_index('+')` and `glyph_index('×')`. Replace with `text_engine.glyph_for('+')` and `text_engine.glyph_for('×')`.

The full migrated `build_glyphs` body is ~80 LOC; expand by analogy from the Stage 6 source.

- [ ] **Step 3: Drop the deprecation aliases from `quad.rs`**

Remove the trailing block from `crates/vibeflow/src/render/quad.rs`:

```rust
#[deprecated(note = "use QuadPipeline directly")]
pub type TextPipeline = QuadPipeline;
#[deprecated(note = "use QuadInstance directly")]
pub type GlyphInstance = QuadInstance;
```

After this, `cargo clippy -- -D warnings` will fail if anything still references the aliases.

- [ ] **Step 4: Update `mod.rs` field name**

If `Renderer` still has `text_pipeline: TextPipeline` (the deprecated alias type) from Task 4's first pass, rename to `quad_pipeline: QuadPipeline`. Search-and-replace `text_pipeline` → `quad_pipeline`.

- [ ] **Step 5: Verify**

```bash
cd /home/bhengen/dev/vibeflow
cargo build --bin vibeflow
cargo test -p vibeflow
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: build clean, all tests pass (count unchanged from Task 4), clippy clean (deprecation warnings now fatal because the aliases are gone, but no callers left).

Smoke run (manual gate — vibeflow must launch without panic, with text legible):

```bash
RUST_LOG=vibeflow=info ./target/debug/vibeflow
```

The window should show the shell prompt, a blinking cursor (Task 7 wires the actual blink — for now it's just visible), and tab text. Type `echo hello`. Each character renders correctly via cosmic-text.

If glyphs render at wrong positions (e.g. clipped at the top of cells), the bearing math is off. The conventional formula is: `screen_y_for_glyph = cell_origin_y + (line_height - bearing_y)`. The plan uses `screen_y + (cell_h as f32 - g.bearing_y as f32)`. If glyphs sit too high or too low, adjust by ±cell_h/4.

- [ ] **Step 6: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/render/tabs.rs crates/vibeflow/src/render/quad.rs crates/vibeflow/src/render/mod.rs
git commit -m "feat(render): migrate tabs.rs to text_engine.glyph_for + drop deprecation aliases"
```

---

## Task 6: Subtitle tracker-state tint

**Files:**
- Modify: `crates/vibeflow/src/render/tabs.rs`

`TabBarRenderer::build_glyphs` currently uses one fg color per tab (`FG_ACTIVE` or `FG_INACTIVE`) for both title and subtitle. Stage 7 splits them: title keeps the existing color; subtitle uses `indicator_color(state)` (full alpha, even for `Waiting` — the pulse only modulates the stripe).

For the `Active` state, fall back to the title's color so the subtitle isn't black-on-black. This is the only state where `indicator_color` returns transparent; everything else has an opaque color.

- [ ] **Step 1: Add a subtitle-color helper**

In `crates/vibeflow/src/render/tabs.rs`, add near `indicator_color`:

```rust
/// Subtitle text color: tracker-state-tinted for non-`Active` states,
/// falls back to the title fg for `Active`.
fn subtitle_color(state: TabState, fallback_fg: [f32; 4]) -> [f32; 4] {
    let mut c = indicator_color(state);
    if c[3] == 0.0 {
        return fallback_fg;
    }
    c[3] = 1.0; // ensure opaque even though indicator may pulse
    c
}
```

- [ ] **Step 2: Use it in `build_glyphs`**

Find the subtitle push block (in `TabBarRenderer::build_glyphs`):

```rust
push_text_glyphs(
    &mut glyphs,
    text_engine,
    &label.subtitle,
    (subtitle_x_start, subtitle_y),
    cell_w_f,
    cell_h_f,
    fg,                            // <-- old: same as title
    bg,
    tab.body.x + tab.body.w - tab.close_button.w - 4,
);
```

Replace the `fg` arg with `subtitle_color(session.state(), fg)`:

```rust
push_text_glyphs(
    &mut glyphs,
    text_engine,
    &label.subtitle,
    (subtitle_x_start, subtitle_y),
    cell_w_f,
    cell_h_f,
    subtitle_color(session.state(), fg),
    bg,
    tab.body.x + tab.body.w - tab.close_button.w - 4,
);
```

The title push above stays unchanged (uses `fg`).

- [ ] **Step 3: Add tests**

Append to the existing `mod tests` in `tabs.rs`:

```rust
    #[test]
    fn subtitle_color_returns_amber_for_waiting() {
        let fallback = [0.0, 0.0, 0.0, 1.0];
        let c = subtitle_color(TabState::Waiting, fallback);
        // Same as indicator_color(Waiting) but alpha forced to 1.0.
        assert!((c[0] - 1.0).abs() < 0.01);
        assert!((c[1] - 0.74).abs() < 0.05);
        assert!((c[2] - 0.18).abs() < 0.05);
        assert_eq!(c[3], 1.0);
    }

    #[test]
    fn subtitle_color_falls_back_to_title_for_active() {
        let fallback = [0.5, 0.6, 0.7, 1.0];
        let c = subtitle_color(TabState::Active, fallback);
        assert_eq!(c, fallback);
    }
```

- [ ] **Step 4: Verify**

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow --lib render::tabs
cargo test -p vibeflow
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: 13 (Stage 6) + 2 (Task 6) = 15 render::tabs tests; total lib = 135.

Smoke: open vibeflow, emit `printf '\033]1338;state=working\007'`. The subtitle on the active tab changes to `working` (Stage 6 already does this) AND the subtitle text renders in blue (new in Stage 7).

- [ ] **Step 5: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/render/tabs.rs
git commit -m "feat(render): subtitle text tinted by tracker state (per-state color)"
```

---

## Task 7: Cursor blink (TDD)

**Files:**
- Modify: `crates/vibeflow/src/render/cursor.rs`
- Modify: `crates/vibeflow/src/window.rs` (about-to-wait redraw cadence)

`CursorBlink` is a tiny stateful oracle. The renderer asks `visible(now) -> bool`; the answer flips every 500 ms.

The about-to-wait loop must request a redraw when the answer would have changed since the last frame. Otherwise the cursor looks stuck.

- [ ] **Step 1: Implement `CursorBlink`**

Replace the contents of `crates/vibeflow/src/render/cursor.rs`:

```rust
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
```

- [ ] **Step 2: Wire about-to-wait redraw cadence**

In `crates/vibeflow/src/window.rs`, find `about_to_wait`. Stage 6's body schedules at 16 ms when `any_waiting`, 100 ms otherwise. Stage 7 also needs to fire a redraw when the cursor's blink state changes between ticks.

Replace the body with:

```rust
fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
    use crate::session::tracker::TabState;

    let now = Instant::now();

    for (idx, ev) in self.app.poll_all(now) {
        self.handle_session_event(idx, ev);
    }
    for (idx, ev) in self.app.tick_all(now) {
        self.handle_session_event(idx, ev);
    }

    let any_waiting = self
        .app
        .tabs()
        .iter()
        .any(|tab| tab.state() == TabState::Waiting);

    let next_deadline = if any_waiting {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
        now + Duration::from_millis(16)
    } else {
        // Cursor blinks at 1 Hz (500 ms toggle). Schedule a redraw at the
        // next blink boundary, capped at 100 ms so tracker timeouts still
        // tick at their usual cadence.
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
        now + Duration::from_millis(100)
    };

    event_loop.set_control_flow(ControlFlow::WaitUntil(next_deadline));
}
```

The simplification: redraw EVERY tick when not Waiting (10 Hz). Cursor blink at 500 ms is a strict subset — every fifth redraw the visibility flips. 10 redraws/sec on an idle terminal is fine and avoids the bookkeeping of "what was the last blink state I drew?".

If profiling later shows 10 Hz idle redraws are too costly (battery), Stage 9's config layer can disable cursor blink entirely or extend the period.

- [ ] **Step 3: Verify**

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow --lib render::cursor
cargo test -p vibeflow
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: 3 new tests pass; total lib = 138.

Smoke: open vibeflow, look at the cursor. It blinks (visible / invisible) once per second. Type a character — the cursor jumps to the new position, still blinking.

- [ ] **Step 4: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/render/cursor.rs crates/vibeflow/src/window.rs
git commit -m "feat(render): 1 Hz cursor blink + tick redraws to keep blink visible"
```

---

## Task 8: Bell visual flash (TDD)

**Files:**
- Modify: `crates/vibeflow/src/render/bell.rs`
- Modify: `crates/vibeflow/src/session/session.rs`
- Modify: `crates/vibeflow/src/app.rs`
- Modify: `crates/vibeflow/src/window.rs`

The chain: shell writes `0x07` → `vte::ansi::Processor`'s `Handler::bell` action fires → `PtySession` records `bell_pending = true` → `poll(...)` drains it into `SessionEvent::Bell` → `App::poll_all` propagates it → `WindowApp::handle_session_event` calls `renderer.note_bell()` → `Renderer::render` reads `bell.tint_alpha(now)` and overlays a white rect.

- [ ] **Step 1: Implement `BellFlash`**

Replace the contents of `crates/vibeflow/src/render/bell.rs`:

```rust
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
```

- [ ] **Step 2: Add `bell_pending` + `SessionEvent::Bell` in `session.rs`**

In `crates/vibeflow/src/session/session.rs`, add to the `SessionEvent` enum:

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum SessionEvent {
    StateChanged(TabState),
    TermUpdated,
    Died,
    /// Shell rang the bell (BEL, 0x07).
    Bell,
}
```

Add `bell_pending: bool` to `PtySession`. Initialise to `false` in `spawn`.

vte's `Processor::advance` doesn't directly expose BEL — it's part of the C0 control set. The simplest hook is to scan the byte stream BEFORE handing it to vte: in `poll`, when iterating `DispatchEvent::PassThrough { byte }`, check if `byte == 0x07` and set `bell_pending = true`.

In `poll`, find the existing pass-through arm:

```rust
DispatchEvent::PassThrough { byte } => {
    self.parser.advance(&mut self.term, byte);
}
```

Replace with:

```rust
DispatchEvent::PassThrough { byte } => {
    if byte == 0x07 {
        self.bell_pending = true;
    }
    self.parser.advance(&mut self.term, byte);
}
```

After the inner loop in `poll`, drain the flag:

```rust
if self.bell_pending {
    self.bell_pending = false;
    events.push(SessionEvent::Bell);
}
```

(Place this after the existing `events.push(SessionEvent::TermUpdated)` block.)

- [ ] **Step 3: Add a test for the bell pipeline**

Append to `crates/vibeflow/src/session/session.rs`'s `mod tests` (DO NOT MODIFY ANY EXISTING TEST):

```rust
    #[test]
    fn poll_emits_bell_event_when_07_byte_received() {
        let mut s = PtySession::spawn(
            &["/bin/sh", "-c", "printf '\\007hi'; sleep 0.5"],
            TrackerConfig::default(),
        )
        .unwrap();
        // Wait for child output.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut got_bell = false;
        while std::time::Instant::now() < deadline && !got_bell {
            for ev in s.poll(std::time::Instant::now()) {
                if matches!(ev, SessionEvent::Bell) {
                    got_bell = true;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(got_bell, "no Bell event seen within 2s");
    }
```

- [ ] **Step 4: Wire through `App` and `Window`**

In `crates/vibeflow/src/app.rs`, `App::poll_all` already returns `Vec<(usize, SessionEvent)>` — no change needed; the new `Bell` variant flows through naturally.

In `crates/vibeflow/src/window.rs`, find `handle_session_event`. Add an arm:

```rust
SessionEvent::Bell => {
    if let Some(renderer) = self.renderer.as_mut() {
        renderer.note_bell();
    }
    if let Some(window) = self.window.as_ref() {
        window.request_redraw();
    }
}
```

(The exact match-arm style depends on whether `handle_session_event` uses `match ev { ... }` or chained `if let`. Match the existing pattern.)

- [ ] **Step 5: Verify**

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: 4 new bell.rs tests + 1 new session test = 5 new; total lib = 143. The session test is integration-flavored (spawns a real shell); if it's flaky on CI, mark `#[ignore]` and document.

Smoke: open vibeflow, run `printf '\007'`. The window briefly tints white. Run again — same flash.

- [ ] **Step 6: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/render/bell.rs \
        crates/vibeflow/src/render/mod.rs \
        crates/vibeflow/src/session/session.rs \
        crates/vibeflow/src/window.rs
git commit -m "feat(render): bell visual flash on BEL (0x07) — 200ms white tint fade"
```

---

## Task 9: Final verification + tag

- [ ] **Step 1: Append Stage 7 section to `docs/TESTING.md`**

```markdown

## Stage 7 — cosmic-text font shaping + subtitle tint + cursor blink + bell flash

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo build --bin vibeflow
RUST_LOG=vibeflow=info ./target/debug/vibeflow
```

- [ ] Window opens within ~700 ms (slightly longer than Stage 6 because
  cosmic-text scans system fonts at startup). Tab bar at top, prompt below.
- [ ] Cursor visibly blinks at 1 Hz on the active tab.
- [ ] Type `echo "héllo wörld 中文 🎉"`. Each character renders:
  - ASCII via JetBrainsMono.
  - Latin extended (é, ö) via JBM (it has full Latin-1 coverage).
  - CJK (中文) via the user's installed CJK font (Noto Sans CJK on most
    Linux distros). If no CJK font installed: tofu boxes — that's fine.
  - Emoji (🎉) renders as monochrome outline or tofu — Stage 7.5 adds
    color emoji.
- [ ] Run `printf '\007'`. Window briefly tints white (~200 ms).
- [ ] Run `printf '\033]1338;state=waiting\007'`. Subtitle changes to
  `waiting` AND renders in **amber** (Stage 6 only changed the text;
  Stage 7 tints it).
- [ ] Run `printf '\033]1338;state=working\007'`. Subtitle in **blue**.
- [ ] Run `printf '\033]1338;state=active\007'` (or wait for it to default).
  Subtitle tint disappears (back to the default tab fg).
- [ ] Open ~10 tabs. Atlas shouldn't visibly stutter as new glyphs are
  cached. (Internal: glyph_for cache hits on repeat ASCII; misses only on
  first sighting of each codepoint.)
- [ ] Resize the window to a tiny size (~10 px). No crash; no GPU errors.
- [ ] Press Ctrl+D in the active tab. Session dies; dead-tab banner appears
  in amber. Cursor stops blinking on the dead tab.
- [ ] Re-run with `WINIT_UNIX_BACKEND=x11`. All checks above still pass.

**Known Stage 7 limitations (deferred to later stages):**

- Color emoji renders as monochrome outline or tofu — Stage 7.5 adds the
  RGBA atlas path.
- No programming ligatures (`==>` renders as three glyphs) — Stage 8 polish.
- No bidi or complex shaping — Stage 8+.
- Font family hardcoded to JBM + system fallback — Stage 9 (TOML config).
- Cursor blink period not configurable — Stage 9.
```

Append to `docs/TESTING.md` after the Stage 6 section.

- [ ] **Step 2: Full local CI dry-run**

```bash
cd /home/bhengen/dev/vibeflow
cargo fmt --all -- --check && \
  cargo clippy --workspace --all-targets -- -D warnings && \
  cargo build --workspace --all-targets && \
  cargo test --workspace --all-targets && \
  RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps && \
  ( cd bindings/npm && npm run build && npm test ) && \
  echo "ALL GREEN"
```

Expected test count: every Stage-6 test still passes (126 lib + 3 + 4 + 27 = 160) plus Stage 7's additions:
- Task 1: 4 tests (text_engine basics)
- Task 2: 3 tests (atlas growth — may be `#[ignore]` if wgpu null adapter is unavailable)
- Task 6: 2 tests (subtitle_color)
- Task 7: 3 tests (cursor blink)
- Task 8: 4 + 1 = 5 tests (bell)

Net: 17 new lib tests → ~143 lib + 3 + 4 + 27 = 177 Rust tests + 1 proptest + 15 npm.

- [ ] **Step 3: 60-second fuzz on the protocol parser**

```bash
cd /home/bhengen/dev/vibeflow/crates/vibeflow-protocol
cargo +nightly fuzz run parse -- -max_total_time=60
```

- [ ] **Step 4: Walk the smoke checklist**

Re-walk `docs/TESTING.md`'s Stage 7 section.

- [ ] **Step 5: Commit + tag**

```bash
cd /home/bhengen/dev/vibeflow
git add docs/TESTING.md
git commit -m "docs: Stage 7 manual smoke checklist"
git tag -a stage7-text-shaping-complete \
  -m "cosmic-text font shaping + subtitle tint + cursor blink + bell flash complete (Stage 7 of v0.1)"
git tag --list
```

- [ ] **Step 6: Surface to user**

Report:
- Number of new commits on this stage (~10).
- Local CI dry-run result.
- New tag name.
- Whether the user wants Stage 7.5 (color emoji) or Stage 8 (keyboard shortcuts + clipboard) as the next plan.

---

## Spec coverage check

Mapping Stage 7 spec requirements → tasks:

| Spec section | Covered by |
|---|---|
| Visual design — subtitle reflects state | Task 6 (`subtitle_color`) |
| Differentiator — Notice indicator amber pulse | Stage 6 (kept) |
| Components — `render/font.rs` (~150 LOC) | Task 1 + 2 (`text_engine.rs`, ~400 LOC) — bigger because cosmic-text + dynamic atlas, but same role |
| Visual design — block cursor blinks | Task 7 |
| Visual design — bell visual feedback | Task 8 |
| Tech stack — cosmic-text replaces fontdue | Tasks 0–4 |
| Out of scope — color emoji | Deferred to Stage 7.5 |

**Out of scope for Stage 7 (deferred):**
- Color emoji (RGBA atlas) — Stage 7.5
- Subtitle italics — Stage 8 polish
- Programming ligatures — Stage 8+
- Bidi/complex shaping — Stage 8+
- Configurable font family / cursor / bell — Stage 9

## Self-review

- **Spec coverage:** every Stage 7-relevant spec requirement maps to a task.
- **Placeholder scan:** no `TBD`/`TODO`/`implement later` patterns. Each step has actual code or commands.
- **Type consistency check:**
  - `TextEngine` defined in Task 1, extended in Task 2, used in Tasks 4–6.
  - `GlyphRef { atlas_x, atlas_y, atlas_w, atlas_h, bearing_x, bearing_y }` — defined in Task 2, consumed in Tasks 4 and 5.
  - `QuadInstance` (64 bytes) — defined in Task 3, used in Tasks 4–5.
  - `QuadPipeline::new(device, surface_format, atlas_view, atlas_sampler)` — Task 3 signature, called in Task 4's `Renderer::new`.
  - `QuadPipeline::rebind_atlas(device, view, sampler)` — Task 3, called in Task 4's `Renderer::render` when `text_engine.texture_dirty()` returns `true`.
  - `CursorBlink::visible(now)` — Task 7, called in `quad::build_cell_instances` (Task 4 Step 3).
  - `BellFlash::tint_alpha(now)` and `note(now)` — Task 8, called in `Renderer::render` and `Renderer::note_bell`.
  - `SessionEvent::Bell` — Task 8, handled in `WindowApp::handle_session_event`.
- **Clippy / fmt discipline:** every code-changing task ends with verify-fmt+clippy.
- **Threading-model discipline:** unchanged. `TextEngine`, `QuadPipeline`, `CursorBlink`, `BellFlash` all on the main thread.
- **Test count tracking:** Stage 6 ended at 126 lib tests. Stage 7 ends at ~143 lib tests (counts may vary ±3 if any GPU-touching tests are `#[ignore]`d on CI).

## Notable plan risks

1. **cosmic-text's API isn't perfectly stable across 0.x releases.** Pinning to `cosmic-text = "0.12"` should be safe; if the tree resolves to a later 0.x with an incompatible `Buffer::shape_until_scroll` signature or `physical(...)` method, Task 1's verify will fail on the first cargo build. The fix: read the new API docs and adapt one or two call sites, then re-dispatch.
2. **Glyph baseline math is the most likely source of "looks wrong" bugs.** The plan uses `screen_y + (cell_h - bearing_y)` to position glyphs. If glyphs render at the wrong vertical position, the implementer should compare with cosmic-text's own example renderer to see the correct formula. Don't invent — copy.
3. **Atlas growth path is GPU-side.** If `grow_atlas` produces a black atlas, the bind-group rebind isn't being triggered. Task 3 Step 2 adds `texture_dirty` for exactly this; verify it's called every frame in Task 4's `Renderer::render`.
4. **System fonts may be unavailable on minimal Docker test images.** If the CI runner has no CJK fonts, `rasterize_cjk_uses_system_fallback` returns None — that's fine; the test asserts only "doesn't panic". If it ever does panic, FontSystem's fontdb scanner had a bug and we should report upstream.
5. **Bell test is integration-flavored.** It spawns a real `/bin/sh` and waits up to 2 s for output. On a heavily-loaded CI runner this could time out. If the test is flaky, mark `#[ignore]` and rely on smoke testing.
6. **`pub texture: wgpu::Texture` and `pub view/sampler` on `TextEngine` violate encapsulation slightly.** Renderer needs the view + sampler to build the bind group. Stage 9 may move bind-group construction into `TextEngine` itself; for Stage 7, the `pub` fields are fine.
7. **Two-quads-per-cell doubles the cell-grid instance count.** 80×24 cells → 1920 cells → up to 3840 quads. At 64 bytes/quad that's 245 KB of vertex data per frame. Negligible for any modern GPU. Profile if it matters; for now, ship.
8. **Cursor blink at 10 Hz idle redraw cadence is wasteful.** A real terminal might drop to <1 Hz when idle. For Stage 7 this is acceptable; Stage 9's config layer can add `[cursor] blink = false` to disable it for battery-conscious users.

These risks are addressed by the senior pre-execution review pass and the Stage 7 manual smoke walkthrough before merge.
