# vibeflow Stage 7.5 Design: Color Emoji + Wide-Glyph Cells

**Date:** 2026-05-03
**Status:** approved (brainstorm); pending implementation plan
**Predecessor:** Stage 7 (`stage7-text-shaping-complete`, merged in `c3e32b8`).
**Successor:** Stage 8 (clipboard / keyboard shortcuts).

## Summary

Stage 7 shipped cosmic-text-driven Unicode shaping with a single R8 monochrome
glyph atlas. Color-emoji codepoints (🎉, 😀, 🚀, …) currently render as tofu /
monochrome outlines because the rasterizer rejects swash's `SwashContent::Color`
output. Stage 7.5 adds a parallel RGBA8Unorm atlas, routes glyphs to whichever
atlas matches their content, and teaches the unified `QuadPipeline` to sample
either atlas per-instance via a fragment-shader branch on a new `kind` field.

The same change makes wide-glyph rendering correct: the `WIDE_CHAR` /
`WIDE_CHAR_SPACER` flags from `alacritty_terminal` are now consulted, so a wide
glyph (CJK or color emoji) gets a 2-cell-wide background and its spacer cell
draws nothing.

## Goals

- Color emoji renders in its native rasterized colors (not tofu, not mono outline).
- Wide-glyph cells (CJK, color emoji) render with a 2-cell-wide background and
  no double-rendering at the spacer cell.
- Stage 7's monochrome behavior (ASCII, Latin, CJK without color, tab text,
  banner, cursor inversion) keeps working unchanged.
- Mono-only callers (banner, tab text) need no source changes.

## Non-Goals (deferred)

- Programming ligatures (`==>`, `!=`) — Stage 8+. (Switch from `Shaping::Basic`
  to `Shaping::Advanced` requires cluster-width handling that's its own feature.)
- Configurable color emoji font family — Stage 9 (TOML config layer).
- Subpixel-mask AA — Stage 9+. For Stage 7.5, `SwashContent::SubpixelMask` is
  treated as `Mono`.
- Bidi / complex shaping — Stage 8+.
- Per-codepoint color overrides (theming emoji) — out of scope for v0.1.
- Cursor-over-emoji color inversion — color glyphs ignore the cell's fg/bg, so
  the cursor cell just shows the emoji on a swapped background. Acceptable for
  v0.1; revisit in Stage 9 if needed.

## Architecture

**Pattern: dual atlas, single pipeline, branching shader.**

Two GPU textures live inside `TextEngine`:

| Atlas | Format | Purpose |
|---|---|---|
| `mono_texture` (existing) | `R8Unorm`, 256×N | Mask-rendered glyphs (most ASCII / Latin / CJK). |
| `color_texture` (new) | `RGBA8Unorm`, 256×N | Pre-multiplied color glyphs (emoji, color fonts). Initial 256×256. |

Both atlases share the same shelf-packer code (factored into a generic helper)
and the same Nearest-filter / ClampToEdge sampler.

`glyph_for(c)` returns a `GlyphRef` carrying a `kind: GlyphKind { Mono | Color }`
field; the cell-instance builder copies that kind into the per-instance buffer
as `flags[0]: u32`. The fragment shader branches:

- `kind == 0` (Mono): sample mono atlas's `.r` as alpha, `mix(bg, fg, alpha)`.
  Identical to Stage 7.
- `kind == 1` (Color): sample color atlas's full RGBA, treat as premultiplied,
  composite over `bg`: `out = vec4(s.rgb + bg.rgb * (1.0 - s.a), 1.0)`.

A single draw call covers both kinds — instances are interleaved in one buffer.

### Wide-glyph fix

`alacritty_terminal::term::cell::Flags` exposes `WIDE_CHAR` and
`WIDE_CHAR_SPACER`. Stage 7 ignores both. Stage 7.5 reads them in
`build_cell_instances`:

- `WIDE_CHAR_SPACER` cell → emit nothing (no bg, no glyph).
- `WIDE_CHAR` cell → emit one background quad of width `2 * cell_w` (covering
  the wide cell + its spacer). Glyph quad is sized by the rasterized image
  (unchanged). Cursor-cell fg/bg swap and `CursorShape::Hidden` gate continue
  to apply to the wide cell.
- Non-wide cells → unchanged.

This works for monochrome wide chars (CJK) too, so CJK rendering becomes
correct as a side effect.

## Components & Files

| File | Change |
|---|---|
| `crates/vibeflow/src/render/text_engine.rs` | Add `GlyphKind` enum (`Mono \| Color`). `RasterImage` and `GlyphRef` gain `kind`. `TextEngine` gains parallel `color_texture/view/atlas_w/h/shelves`. `rasterize` now returns the RGBA payload for `SwashContent::Color`. `try_atlas` routes by kind. The shelf-pack inner loop is extracted into `shelf_pack(&mut Vec<Shelf>, atlas_w, atlas_h, w, h, &mut grow_cb) -> (u32, u32)` used by both atlases. `atlas_dirty` becomes `atlases_dirty` (single bool). New `color_atlas_size() -> (u32, u32)` getter. |
| `crates/vibeflow/src/render/quad.rs` | `QuadInstance` gains `flags: [u32; 4]` (16 bytes; total 80). Vertex attributes: add `Uint32x4` at offset 64. Bind-group layout grows to 4 entries (uniform + mono tex + color tex + sampler). `QuadUniform` grows to 32 bytes (mono + color atlas size pairs + 8 B pad). `make_bind_group` / `rebind_atlas` → `rebind_atlases(device, mono_view, color_view, sampler)`. `build_cell_instances` reads `cell.flags`, sets `flags[0] = glyph.kind as u32`, doubles bg width on `WIDE_CHAR`, skips `WIDE_CHAR_SPACER`. `build_banner_instances` keeps emitting `kind = Mono`. |
| `crates/vibeflow/src/render/quad.wgsl` | Bindings: mono @ 1, color @ 2, sampler @ 3. `QuadUniform` grows. Vertex shader picks atlas-size pair by `kind` for UV math. Fragment shader branches by `kind` (mono mix vs. premultiplied over-blend). |
| `crates/vibeflow/src/render/mod.rs` | `Renderer::new` passes `&text_engine.color_view` into `QuadPipeline::new`. The `texture_dirty` poll calls renamed `rebind_atlases`. No structural change. |
| `crates/vibeflow/Cargo.toml` | No dep change. cosmic-text 0.12 already pulls swash for color content. |
| `docs/TESTING.md` | Append Stage 7.5 manual smoke checklist. |

**Net:** ~+300 / −65 LOC. 4 files modified. 0 deps added. 0 files deleted.

## Data Flow

### Per-glyph rasterization (cached)

```
glyph_for(c)
  └ cache hit → return GlyphRef
  └ cache miss → rasterize(c)
      └ swash output:
          - SwashContent::Mask → RasterImage { kind: Mono, R8 bytes }
          - SwashContent::Color → RasterImage { kind: Color, RGBA premultiplied }
          - SwashContent::SubpixelMask → treated as Mask (Stage 9+ for proper handling)
      └ try_atlas(kind, image) → shelf-pack into the right atlas → upload via queue.write_texture
      └ if any atlas grew, atlases_dirty = true
      └ cache.insert(c, Some(GlyphRef { kind, ... }))
```

### Per-frame render

1. `build_cell_instances` walks the term's `display_iter`. For each cell:
   - Skip `WIDE_CHAR_SPACER` cells.
   - For `WIDE_CHAR` cells, emit a 2-cell-wide bg quad.
   - Call `text_engine.glyph_for(cell.c)`. The returned `GlyphRef.kind` is
     copied into `QuadInstance.flags[0]`.
2. After all builders, `Renderer::render` checks `text_engine.atlases_dirty()`.
   If true, `quad_pipeline.rebind_atlases(...)`.
3. Single `quad_pipeline.draw(...)` call dispatches both kinds. The shader
   branches per-instance.

Tab text and banner code paths stay unchanged — they only ever produce mono
glyphs, so `flags[0]` stays 0 and they go through the existing mono path.

## Tests

Stage 6 and Stage 7's TDD pattern continues. New tests gate behavior; all 128
default-running + 7 `#[ignore]`d Stage 7 tests must keep passing.

### Unit tests

| Test | File | Default-run? |
|---|---|---|
| `rasterize_color_emoji_returns_color_kind` | `text_engine.rs` | Yes (no wgpu) |
| `rasterize_mono_letter_still_returns_mono_kind` | `text_engine.rs` | Yes |
| `glyph_for_emoji_routes_to_color_atlas` | `text_engine.rs` | `#[ignore]` (wgpu) |
| `color_atlas_grows_when_full` | `text_engine.rs` | `#[ignore]` (wgpu) |
| `build_cell_instances_skips_wide_char_spacer` | `quad.rs` (or helper fn) | Yes |

The first emoji test guards against missing emoji fonts: if `rasterize('🎉')`
returns `None`, the test is a no-op and notes the env dep in a comment.

### Final test count (after Stage 7.5)

130 default + 9 ignored. (Stage 7's 128 + 7 + 2 default + 2 ignored.)

### Smoke checklist (appended to `docs/TESTING.md`)

```
- [ ] Run `printf '🎉 🚀 😀\n'`. Emoji renders in full color.
- [ ] Run `printf '中文 vs 中文\n'`. CJK renders identically each side; no
      overflow into adjacent cells.
- [ ] Run `printf '🎉🎉🎉\n'`. Backgrounds tile cleanly under back-to-back
      wide chars; no clipping.
- [ ] Type at the prompt with cursor over a wide char. Cursor block covers
      the full 2-cell width.
- [ ] Run `for i in $(seq 1 100); do printf '%s' $(printf '\\U%x' $((0x1f600 + i % 40))); done`.
      Atlas grows; no visible stutter.
- [ ] Resize the window to ~10 px. No GPU errors; emoji still correct.
- [ ] On a system with NO color emoji font (uninstall Noto Color Emoji):
      emoji renders as tofu/outline (Stage 7 behavior). No crash.
- [ ] Re-run with `WINIT_UNIX_BACKEND=x11`. All checks above still pass.
```

## Risks

1. **No color emoji font installed.** `glyph_for('🎉')` returns `None`;
   tofu/outline is the fallback. Smoke checklist guards. host (Ubuntu 24.04)
   ships Noto Color Emoji by default — non-issue in dev.
2. **swash returning straight (non-premultiplied) alpha.** Documentation says
   premultiplied. If a host font ships otherwise, emoji washes out. Mitigation:
   visual smoke check; if washed-out emoji is observed, switch the shader's
   color-branch from `s.rgb + bg.rgb * (1.0 - s.a)` to `mix(bg.rgb, s.rgb, s.a)`.
3. **Cursor over color emoji.** Cell builder swaps fg/bg, but the color shader
   path ignores fg/bg. Result: emoji on inverted bg. Acceptable for v0.1.
4. **Atlas growth during a single frame.** Both atlases may grow on the same
   frame from different `glyph_for` calls. `atlases_dirty` is a single bool that
   fires regardless; `rebind_atlases` rebuilds with both views. Safe by
   construction.
5. **WIDE_CHAR detection in tests.** Constructing an `alacritty_terminal::Term`
   with a wide char at a known position is non-trivial. Fall back to a
   pure-logic helper test if needed (`should_skip_cell(flags) -> bool`).

## Out of Scope (re-statement)

- Programming ligatures.
- Bidi / complex shaping.
- Configurable color emoji font family.
- Subpixel mask AA.
- Per-codepoint emoji theming.
- Cursor-over-emoji color inversion.
- Color glyph caching by RGB value (we cache by `char`).

## Open Questions

None. The architectural decisions above were made during brainstorming; the
plan can proceed.

## Estimated Effort

3–5 days, paced for Rust learning. Smaller than Stage 7 (no new deps, no plan
ordering surprises, well-bounded). Senior pre-execution review of the
implementation plan recommended (Stage 6 + Stage 7 lessons).
