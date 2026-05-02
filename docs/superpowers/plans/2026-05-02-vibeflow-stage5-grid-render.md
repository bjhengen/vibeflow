# vibeflow Stage 5 Implementation Plan: alacritty_terminal grid + cell renderer

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Connect `alacritty_terminal::Term` to `PtySession` so the parsed cell grid is the source of truth for what the user sees, then render that grid through the wgpu pipeline introduced in Stage 4. After this plan, vibeflow is a usable terminal — typing into a tab shows characters, the shell prompt and command output are visible, the block cursor is drawn, and ANSI 16 / 256-indexed / truecolor foreground and background colors render correctly.

**Architecture:** Three new render submodules, a `Term` field on `PtySession`, and a refactor of one Stage 3 test plus the `SessionEvent` enum:

- `session/session.rs` (modify) — `PtySession` gains `term: Term<VoidListener>` and `parser: vte::ansi::Processor` fields. `poll` feeds `DispatchEvent::PassThrough` bytes into `parser.advance(&mut self.term, byte)` so the grid stays current. The byte payload is no longer surfaced upward; `SessionEvent::PassThrough(Vec<u8>)` is renamed to `SessionEvent::TermUpdated` (no payload — it's just a signal that the grid changed).
- `app.rs` (modify) — `App::active_term(&self) -> Option<&Term<VoidListener>>` exposes the focused tab's grid for the renderer.
- `render/colors.rs` (new) — pure logic, TDD'd: a default 256-entry ANSI palette plus `resolve_color(alacritty_color, &Colors, fg_default, bg_default) -> Rgb` that maps the three `Color` variants (`Named`, `Indexed`, `Spec`) to concrete RGB triples. Required because alacritty's `Colors` slot table is sparse (only filled when the running app sends OSC 4 palette overrides).
- `render/atlas.rs` (new) — `GlyphAtlas` rasterizes printable ASCII (`0x20..=0x7e`) at the configured pixel size using `fontdue::Font::rasterize`, packs the glyphs into a single 2D `wgpu::Texture` with a fixed glyph-cell pitch, and exposes `glyph_index(c) -> Option<u32>`. ~150 LOC.
- `render/grid.wgsl` (new) — vertex + fragment shaders for the cell render pass. Vertex shader expands per-instance cell data (col, row, glyph_index, fg, bg) into 6 vertices forming a textured quad in screen space. Fragment shader samples the atlas's grayscale alpha and `mix`es bg → fg.
- `render/grid.rs` (new) — wgpu pipeline state + per-frame instance-buffer building. ~250 LOC.
- `render/mod.rs` (modify) — `Renderer` gains the new pipeline state and `render` now takes `Option<&Term<VoidListener>>`; if `None`, falls back to the Stage-4 clear-color path.
- `window.rs` (modify) — `RedrawRequested` calls `renderer.render(self.app.active_term())`; `handle_session_event` requests a redraw on `TermUpdated`.
- `assets/JetBrainsMono-Regular.ttf` (new, downloaded) — embedded via `include_bytes!`. JetBrains Mono is OFL/Apache-licensed; the spec already specifies it as the default font.

**Threading model:** unchanged. `Term` lives on the main thread inside `PtySession`. `Processor` lives alongside it. The mpsc channel still carries raw bytes from the reader thread; `Term` mutation happens only inside `PtySession::poll`. `Renderer` stays on the main thread per the wgpu+winit constraint.

**Tech Stack:** Adds:

```toml
alacritty_terminal = "0.24"
fontdue = "0.9"
bytemuck = { version = "1", features = ["derive"] }
```

`bytemuck` is needed to safely cast `[CellInstance]` slices to `[u8]` for `wgpu::Queue::write_buffer`. `fontdue` is the rasterizer; the spec lists `cosmic-text` as the v0.1 final font dep, but Stage 7 — when shaping is needed — is the natural place to swap. fontdue is much simpler (zero deps, ~5-line rasterization API) and a better learning vehicle for now.

**Stage scope:** Stage 5 ends with a window that **renders the terminal grid in color**. The user can run a shell command, see the output, see the cursor, see ANSI/truecolor styling. **Out of scope (deferred):** selection rendering (mouse drag highlights), scrollback rendering on mouse wheel, the tab bar with the Notice indicator, cursor blink animation, bell, hyperlinks, image protocols. Stages 6+ add those.

**Lessons carried forward from Stages 1–4:**
- Pre-fmt the verbatim Rust code (rustfmt prefers wider line breaking than the human-readable plan style).
- Forward-declared items get `#[allow(dead_code)]` until the first lib-level caller arrives, with cleanup in the introducing-caller task.
- Per-task `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings` verify step before commit.
- Intra-doc links must use `[`Self::method`]` not `[`method`]` to satisfy `RUSTDOCFLAGS="-D warnings" cargo doc`.
- For tests that depend on subprocess byte emission, prefer `python3 -c "...sys.stdout.buffer.write(bytes([...]))"` over `/bin/sh -c "printf '\xNN…'"`. Ubuntu's `/bin/sh` is dash, whose `printf` does not interpret `\xNN` hex escapes (only octal `\NNN`).
- winit + wgpu cannot be unit-tested headlessly. This stage's tests cover only pure logic (color resolution, atlas-layout math, alacritty Term grid contents). The visible render is validated by the manual smoke checklist.
- Plan-verbatim API code must be verified against the actual locked-in dependency versions. The Stage 4 plan was reviewed against winit 0.30.13 and wgpu 0.20.1 source before dispatch; this plan was reviewed against `alacritty_terminal 0.24.2` and `fontdue 0.9.3`.
- Senior-tier final review (Sonnet+) catches whole-stage issues that per-task Haiku reviewers miss. Stage 5 ends with one before merging.

---

## File Structure

| Path | Responsibility |
|---|---|
| `crates/vibeflow/Cargo.toml` (modify) | Add `alacritty_terminal = "0.24"`, `fontdue = "0.9"`, `bytemuck = { version = "1", features = ["derive"] }`. |
| `crates/vibeflow/assets/JetBrainsMono-Regular.ttf` (new, download) | Embedded font via `include_bytes!`. ~250 KB. |
| `crates/vibeflow/assets/LICENSE.JetBrainsMono.txt` (new, download) | OFL license file for the bundled font (attribution requirement). |
| `crates/vibeflow/src/render/mod.rs` (modify) | Add new submodules; `Renderer` gains `atlas: GlyphAtlas` and `grid_pipeline: GridPipeline`; `render` takes `Option<&Term<VoidListener>>`. |
| `crates/vibeflow/src/render/colors.rs` (new) | Default ANSI palette + `resolve_color`. ~120 LOC. |
| `crates/vibeflow/src/render/atlas.rs` (new) | `GlyphAtlas`: fontdue rasterization, single-texture packing, UV math. ~180 LOC. |
| `crates/vibeflow/src/render/grid.wgsl` (new) | Vertex + fragment shaders. ~70 LOC of WGSL. |
| `crates/vibeflow/src/render/grid.rs` (new) | `GridPipeline`: pipeline-state object, uniform layout, instance-buffer management, `draw` method. ~250 LOC. |
| `crates/vibeflow/src/session/session.rs` (modify) | Add `term`, `parser` fields; feed bytes; rename `SessionEvent::PassThrough` → `TermUpdated`; expose `term()`. |
| `crates/vibeflow/src/app.rs` (modify) | Add `active_term()` accessor; refactor Stage-3 `send_input_writes_to_active_tab` test to assert via Term. |
| `crates/vibeflow/src/window.rs` (modify) | `RedrawRequested` passes `app.active_term()` to renderer; `TermUpdated` triggers `request_redraw`. |
| `docs/TESTING.md` (extend) | Append Stage 5 manual smoke checklist. |

---

## Task 0: Add deps + asset directory + module declarations + stubs

**Files:**
- Modify: `crates/vibeflow/Cargo.toml`
- Modify: `crates/vibeflow/src/render/mod.rs`
- Create: `crates/vibeflow/assets/JetBrainsMono-Regular.ttf` (downloaded)
- Create: `crates/vibeflow/assets/LICENSE.JetBrainsMono.txt` (downloaded)
- Create: stub files for `crates/vibeflow/src/render/colors.rs`, `crates/vibeflow/src/render/atlas.rs`, `crates/vibeflow/src/render/grid.rs`, `crates/vibeflow/src/render/grid.wgsl`

- [ ] **Step 1: Download JetBrains Mono Regular**

JetBrains Mono v2.304 is the version used here. Download the Regular weight and the LICENSE.txt:

```bash
cd /home/bhengen/dev/vibeflow
mkdir -p crates/vibeflow/assets
curl -fsSL -o crates/vibeflow/assets/JetBrainsMono-Regular.ttf \
    https://github.com/JetBrains/JetBrainsMono/raw/v2.304/fonts/ttf/JetBrainsMono-Regular.ttf
curl -fsSL -o crates/vibeflow/assets/LICENSE.JetBrainsMono.txt \
    https://github.com/JetBrains/JetBrainsMono/raw/v2.304/OFL.txt
ls -la crates/vibeflow/assets/
```

Expected:
- `JetBrainsMono-Regular.ttf` ~270 KB
- `LICENSE.JetBrainsMono.txt` ~4 KB (SIL Open Font License 1.1)

If either download fails (network issues, repo move): fall back to any other monospace TTF that's OFL- or Apache-licensed (e.g. JuliaMono, IBM Plex Mono). Update `LICENSE.JetBrainsMono.txt` accordingly. Note the substitution in your report.

- [ ] **Step 2: Add the new dependencies to `crates/vibeflow/Cargo.toml`**

Replace the existing `[dependencies]` section. The current Stage-4 contents are:

```toml
[dependencies]
vibeflow-protocol = { path = "../vibeflow-protocol", version = "0.1" }
portable-pty = "0.8"
winit = "0.30"
wgpu = "0.20"
pollster = "0.3"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

Replace with:

```toml
[dependencies]
vibeflow-protocol = { path = "../vibeflow-protocol", version = "0.1" }
portable-pty = "0.8"
winit = "0.30"
wgpu = "0.20"
pollster = "0.3"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
alacritty_terminal = "0.24"
fontdue = "0.9"
bytemuck = { version = "1", features = ["derive"] }
```

Leave `[package]`, `[lints]`, `[lib]`, `[[bin]]`, `[dev-dependencies]` unchanged.

**Why these versions:** `alacritty_terminal = "0.24"` resolves to 0.24.2 — the API was reviewed against that exact patch. `fontdue = "0.9"` resolves to 0.9.3. `bytemuck` with `derive` lets us `#[derive(Pod, Zeroable)]` on `CellInstance` so the slice-to-bytes cast is safe and zero-cost.

- [ ] **Step 3: Declare the new submodules in `render/mod.rs`**

The current Stage-4 `crates/vibeflow/src/render/mod.rs` starts with a doc comment + `Renderer` struct. Add module declarations at the top, between the doc comment and the existing `use std::sync::Arc;`:

```rust
//! GPU rendering primitives. Stage 4 ships a minimal [`Renderer`] that opens a
//! wgpu surface on a [`winit::window::Window`] and clears it to a solid color.
//! Stage 5 layers the cell grid on top; Stage 6 adds the tab bar.

pub mod atlas;
pub mod colors;
pub mod grid;

use std::sync::Arc;
// ... rest of Stage 4 contents unchanged for now ...
```

(Tasks 4 and 6 modify `Renderer` itself; this step only adds the module declarations.)

- [ ] **Step 4: Stub the new files**

Create `crates/vibeflow/src/render/colors.rs`:

```rust
//! ANSI / 256-indexed / truecolor → RGB resolution. Pure logic. Stage 5 Task 1
//! fills in the default palette table and the [`resolve_color`] function.
```

Create `crates/vibeflow/src/render/atlas.rs`:

```rust
//! Glyph atlas. Pre-rasterises printable ASCII (0x20..=0x7e) via fontdue at the
//! configured pixel size, packs the glyphs into a single wgpu texture, and
//! exposes UV / metric lookups by character. Stage 7 will replace fontdue with
//! cosmic-text shaping for full Unicode + ligatures + emoji.
```

Create `crates/vibeflow/src/render/grid.rs`:

```rust
//! Cell-grid render pipeline. Owns the wgpu pipeline-state object, the bind
//! group for the atlas texture + sampler, the per-frame uniform buffer, and
//! the dynamically-grown instance buffer.
```

Create `crates/vibeflow/src/render/grid.wgsl`:

```wgsl
// vibeflow Stage 5 cell-grid shader. Filled in by Task 5.
```

- [ ] **Step 5: Verify the workspace builds**

```bash
cd /home/bhengen/dev/vibeflow
cargo build -p vibeflow
cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: clean build (the new modules are stubs but valid). Clippy silent. `Cargo.lock` will have grown — `alacritty_terminal` pulls in `vte`, `bitflags`, etc., and `fontdue` pulls in `hashbrown` and a couple of font-parser crates.

If the build fails because alacritty_terminal needs a system dep (uncommon — it's pure Rust), report BLOCKED with the exact error.

- [ ] **Step 6: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/Cargo.toml \
    crates/vibeflow/assets/JetBrainsMono-Regular.ttf \
    crates/vibeflow/assets/LICENSE.JetBrainsMono.txt \
    crates/vibeflow/src/render/mod.rs \
    crates/vibeflow/src/render/colors.rs \
    crates/vibeflow/src/render/atlas.rs \
    crates/vibeflow/src/render/grid.rs \
    crates/vibeflow/src/render/grid.wgsl \
    Cargo.lock
git commit -m "chore(vibeflow): add alacritty_terminal/fontdue/bytemuck deps + Stage 5 stubs + JetBrains Mono"
```

---

## Task 1: Default ANSI palette + Color resolver (TDD)

**Files:**
- Modify: `crates/vibeflow/src/render/colors.rs`

`alacritty_terminal::vte::ansi::Color` has three variants — `Named(NamedColor)`, `Indexed(u8)`, `Spec(Rgb)`. Stage 5 needs to translate any cell's `fg` and `bg` into a concrete `Rgb` for the GPU. The complication is that alacritty's `Colors` slot table (`[Option<Rgb>; 269]`) is sparse: only filled when the running app emits OSC 4 palette overrides. For the common case where no override is set, we need a built-in default ANSI palette.

This task implements the default palette as a static `[Rgb; 256]` (16 ANSI named colors + 6×6×6 color cube + 24 grayscale ramp) plus a `resolve_color` function. Pure logic, fully TDD'd.

- [ ] **Step 1: Write the failing tests**

Replace the contents of `crates/vibeflow/src/render/colors.rs` with the test scaffold (still missing the implementation):

```rust
//! ANSI / 256-indexed / truecolor → RGB resolution. Pure logic.
//!
//! `alacritty_terminal::vte::ansi::Color` carries three variants. To produce
//! GPU-ready RGB we need a default ANSI palette (the Term struct doesn't fill
//! one — it only stores OSC-4 overrides). [`resolve_color`] handles all three
//! variants against the default palette + any overrides.

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_colors() -> Colors {
        // Default `Colors::default()` has every slot None.
        Colors::default()
    }

    #[test]
    fn default_palette_first_eight_match_xterm_basics() {
        // The classic ANSI 0..=7 palette, xterm defaults.
        assert_eq!(default_palette()[0], Rgb { r: 0x00, g: 0x00, b: 0x00 }); // black
        assert_eq!(default_palette()[1], Rgb { r: 0xcd, g: 0x00, b: 0x00 }); // red
        assert_eq!(default_palette()[2], Rgb { r: 0x00, g: 0xcd, b: 0x00 }); // green
        assert_eq!(default_palette()[3], Rgb { r: 0xcd, g: 0xcd, b: 0x00 }); // yellow
        assert_eq!(default_palette()[4], Rgb { r: 0x00, g: 0x00, b: 0xee }); // blue
        assert_eq!(default_palette()[5], Rgb { r: 0xcd, g: 0x00, b: 0xcd }); // magenta
        assert_eq!(default_palette()[6], Rgb { r: 0x00, g: 0xcd, b: 0xcd }); // cyan
        assert_eq!(default_palette()[7], Rgb { r: 0xe5, g: 0xe5, b: 0xe5 }); // white
    }

    #[test]
    fn default_palette_bright_colors_are_brighter() {
        // Bright variants are at indices 8..=15.
        for i in 0..8 {
            let normal = default_palette()[i];
            let bright = default_palette()[i + 8];
            // Sum of channels is monotonically nondecreasing for the bright variant.
            let normal_sum = normal.r as u32 + normal.g as u32 + normal.b as u32;
            let bright_sum = bright.r as u32 + bright.g as u32 + bright.b as u32;
            assert!(
                bright_sum >= normal_sum,
                "bright[{}] ({bright_sum}) should be ≥ normal[{}] ({normal_sum})",
                i + 8,
                i
            );
        }
    }

    #[test]
    fn default_palette_color_cube_at_index_16_is_pure_black() {
        // The 6×6×6 color cube starts at index 16. (16, 0, 0, 0).
        assert_eq!(
            default_palette()[16],
            Rgb { r: 0, g: 0, b: 0 }
        );
    }

    #[test]
    fn default_palette_grayscale_ramp_starts_dark_ends_light() {
        // The grayscale ramp occupies 232..=255.
        let dark = default_palette()[232];
        let light = default_palette()[255];
        assert!(dark.r < 20, "expected near-black at 232, got {dark:?}");
        assert!(light.r > 220, "expected near-white at 255, got {light:?}");
        // Ramp is monotonically nondecreasing.
        for i in 232..255 {
            let lo = default_palette()[i].r;
            let hi = default_palette()[i + 1].r;
            assert!(hi >= lo, "ramp not monotonic at {i}: {lo} > {hi}");
        }
    }

    #[test]
    fn resolve_color_spec_passes_rgb_unchanged() {
        let rgb = Rgb { r: 0x12, g: 0x34, b: 0x56 };
        let resolved = resolve_color(
            Color::Spec(rgb),
            &empty_colors(),
            Rgb { r: 0xff, g: 0xff, b: 0xff }, // fg fallback
            Rgb { r: 0x00, g: 0x00, b: 0x00 }, // bg fallback
        );
        assert_eq!(resolved, rgb);
    }

    #[test]
    fn resolve_color_indexed_uses_default_palette_when_overrides_empty() {
        // Index 1 is red in the default palette.
        let resolved = resolve_color(
            Color::Indexed(1),
            &empty_colors(),
            Rgb { r: 0xff, g: 0xff, b: 0xff },
            Rgb { r: 0x00, g: 0x00, b: 0x00 },
        );
        assert_eq!(resolved, Rgb { r: 0xcd, g: 0x00, b: 0x00 });
    }

    #[test]
    fn resolve_color_indexed_prefers_override_when_set() {
        let mut colors = Colors::default();
        colors[1usize] = Some(Rgb { r: 0xab, g: 0xcd, b: 0xef });
        let resolved = resolve_color(
            Color::Indexed(1),
            &colors,
            Rgb { r: 0xff, g: 0xff, b: 0xff },
            Rgb { r: 0x00, g: 0x00, b: 0x00 },
        );
        assert_eq!(resolved, Rgb { r: 0xab, g: 0xcd, b: 0xef });
    }

    #[test]
    fn resolve_color_named_foreground_uses_fg_fallback_when_unset() {
        let fg = Rgb { r: 0xee, g: 0xee, b: 0xee };
        let resolved = resolve_color(
            Color::Named(NamedColor::Foreground),
            &empty_colors(),
            fg,
            Rgb { r: 0x00, g: 0x00, b: 0x00 },
        );
        assert_eq!(resolved, fg);
    }

    #[test]
    fn resolve_color_named_background_uses_bg_fallback_when_unset() {
        let bg = Rgb { r: 0x0e, g: 0x0e, b: 0x12 };
        let resolved = resolve_color(
            Color::Named(NamedColor::Background),
            &empty_colors(),
            Rgb { r: 0xff, g: 0xff, b: 0xff },
            bg,
        );
        assert_eq!(resolved, bg);
    }

    #[test]
    fn resolve_color_named_red_uses_default_palette_when_unset() {
        // NamedColor::Red is index 1 in the ANSI palette.
        let resolved = resolve_color(
            Color::Named(NamedColor::Red),
            &empty_colors(),
            Rgb { r: 0xff, g: 0xff, b: 0xff },
            Rgb { r: 0x00, g: 0x00, b: 0x00 },
        );
        assert_eq!(resolved, Rgb { r: 0xcd, g: 0x00, b: 0x00 });
    }
}
```

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow --lib render::colors
```

Expected: compile errors — `default_palette` and `resolve_color` not defined.

- [ ] **Step 2: Implement `default_palette` and `resolve_color`**

Append the implementation above the `#[cfg(test)] mod tests` block in `crates/vibeflow/src/render/colors.rs`:

```rust
/// 256-entry default ANSI / 256-color palette. Slots 0..=15 are the classic
/// ANSI 16 (xterm defaults), 16..=231 are the 6×6×6 color cube, 232..=255 are
/// the 24-step grayscale ramp.
///
/// This is the fallback used when [`alacritty_terminal::term::color::Colors`]
/// has no override set for a slot. Most apps don't emit OSC 4 palette
/// overrides, so this fallback covers the common case.
#[must_use]
pub fn default_palette() -> [Rgb; 256] {
    let mut palette = [Rgb { r: 0, g: 0, b: 0 }; 256];

    // ANSI 0..=7 (xterm normal-intensity defaults).
    palette[0] = Rgb { r: 0x00, g: 0x00, b: 0x00 };
    palette[1] = Rgb { r: 0xcd, g: 0x00, b: 0x00 };
    palette[2] = Rgb { r: 0x00, g: 0xcd, b: 0x00 };
    palette[3] = Rgb { r: 0xcd, g: 0xcd, b: 0x00 };
    palette[4] = Rgb { r: 0x00, g: 0x00, b: 0xee };
    palette[5] = Rgb { r: 0xcd, g: 0x00, b: 0xcd };
    palette[6] = Rgb { r: 0x00, g: 0xcd, b: 0xcd };
    palette[7] = Rgb { r: 0xe5, g: 0xe5, b: 0xe5 };
    // ANSI 8..=15 (xterm bright defaults).
    palette[8] = Rgb { r: 0x7f, g: 0x7f, b: 0x7f };
    palette[9] = Rgb { r: 0xff, g: 0x00, b: 0x00 };
    palette[10] = Rgb { r: 0x00, g: 0xff, b: 0x00 };
    palette[11] = Rgb { r: 0xff, g: 0xff, b: 0x00 };
    palette[12] = Rgb { r: 0x5c, g: 0x5c, b: 0xff };
    palette[13] = Rgb { r: 0xff, g: 0x00, b: 0xff };
    palette[14] = Rgb { r: 0x00, g: 0xff, b: 0xff };
    palette[15] = Rgb { r: 0xff, g: 0xff, b: 0xff };

    // 6×6×6 color cube (indices 16..=231). xterm uses {0, 95, 135, 175, 215, 255}
    // for each of the six steps per channel.
    const CUBE_STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let mut idx = 16usize;
    for r in 0..6 {
        for g in 0..6 {
            for b in 0..6 {
                palette[idx] = Rgb {
                    r: CUBE_STEPS[r],
                    g: CUBE_STEPS[g],
                    b: CUBE_STEPS[b],
                };
                idx += 1;
            }
        }
    }
    debug_assert_eq!(idx, 232);

    // Grayscale ramp (indices 232..=255). xterm uses 8 + 10*i for the i-th step.
    for i in 0..24 {
        let v = 8 + 10 * i;
        palette[232 + i as usize] = Rgb { r: v, g: v, b: v };
    }

    palette
}

/// Resolve an alacritty `Color` to a concrete RGB triple, using the override
/// table if it has the requested slot set, falling back to the built-in
/// [`default_palette`] for `Indexed` / `Named` color values, and to the
/// caller-supplied `fg_default` / `bg_default` for the special
/// `NamedColor::Foreground` / `NamedColor::Background` semantic slots.
///
/// `Spec(rgb)` is passed through unchanged.
#[must_use]
pub fn resolve_color(color: Color, colors: &Colors, fg_default: Rgb, bg_default: Rgb) -> Rgb {
    match color {
        Color::Spec(rgb) => rgb,
        Color::Indexed(idx) => colors[idx as usize].unwrap_or(default_palette()[idx as usize]),
        Color::Named(named) => named_color_to_rgb(named, colors, fg_default, bg_default),
    }
}

fn named_color_to_rgb(
    named: NamedColor,
    colors: &Colors,
    fg_default: Rgb,
    bg_default: Rgb,
) -> Rgb {
    if let Some(rgb) = colors[named] {
        return rgb;
    }
    // The `Foreground` / `Background` slots are semantic, not part of the
    // 256-color palette. They use the caller's defaults when unset.
    match named {
        NamedColor::Foreground | NamedColor::DimForeground | NamedColor::BrightForeground => {
            fg_default
        }
        NamedColor::Background => bg_default,
        // Cursor + selection-bg/fg also live in the special slot range; treat
        // them as transparent fallbacks via the fg/bg defaults for now. Stage 6
        // adds proper handling.
        NamedColor::Cursor => fg_default,
        // Everything else is in the 0..=15 ANSI range. NamedColor's repr is the
        // index for those, so we can index the default palette directly.
        other => default_palette()[other as usize],
    }
}
```

Note: `Colors` indexing accepts `usize` for the 256-color slots and `NamedColor` for the semantic ones — both are in scope via `alacritty_terminal::term::color::Colors`'s public `Index` impls.

- [ ] **Step 3: Run the tests**

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow --lib render::colors
```

Expected: 9 tests pass.

- [ ] **Step 4: Verify fmt + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

If clippy fires `dead_code` on `default_palette` / `resolve_color` (both `pub`, but no caller yet outside tests), that would be unusual since they're public — but if it does, narrow the suppression. Otherwise no allow needed.

- [ ] **Step 5: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/render/colors.rs
git commit -m "feat(render): default ANSI palette and resolve_color"
```

---

## Task 2: PtySession owns Term + Processor; rename SessionEvent::PassThrough → TermUpdated; refactor callers (TDD)

**Files:**
- Modify: `crates/vibeflow/src/session/session.rs`
- Modify: `crates/vibeflow/src/app.rs`
- Modify: `crates/vibeflow/src/window.rs`

This is the architectural pivot of Stage 5. Three concrete changes:

1. `PtySession` gains `term: Term<VoidListener>` and `parser: vte::ansi::Processor` fields. The `Term` is constructed at default 80×24 in `spawn`; subsequent `resize` calls re-size it alongside the PTY.
2. `PtySession::poll` feeds `DispatchEvent::PassThrough(bytes)` into the processor, which mutates the term's grid. The bytes are consumed in place — they no longer need to bubble up.
3. `SessionEvent::PassThrough(Vec<u8>)` is renamed to `SessionEvent::TermUpdated` (no payload — it's a "the grid changed, re-render" signal). All call sites update accordingly.

Plus accessors: `PtySession::term(&self) -> &Term<VoidListener>` and `App::active_term(&self) -> Option<&Term<VoidListener>>`.

Plus refactors: the Stage 3 `app::tests::send_input_writes_to_active_tab` test currently asserts `b"hi"` arrives via `SessionEvent::PassThrough`; it now asserts via the Term's grid contents.

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` block in `crates/vibeflow/src/session/session.rs`:

```rust
    #[test]
    fn term_consumes_bytes_during_poll() {
        // Spawn a child that writes a known string. After poll, Term's grid
        // should contain the characters.
        let mut s = PtySession::spawn(
            &[
                "python3",
                "-c",
                "import sys, time; sys.stdout.buffer.write(b'hello\\n'); sys.stdout.flush(); time.sleep(2)",
            ],
            TrackerConfig::default(),
        )
        .unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if Instant::now() >= deadline {
                panic!("never observed Term contents");
            }
            let _events = s.poll(Instant::now());
            // Read the first row of Term and look for "hello".
            let row_text: String = s
                .term()
                .renderable_content()
                .display_iter
                .filter(|i| i.point.line.0 == 0)
                .map(|i| i.cell.c)
                .collect();
            if row_text.contains("hello") {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn poll_emits_term_updated_when_bytes_arrive() {
        let mut s = PtySession::spawn(
            &[
                "python3",
                "-c",
                "import sys, time; sys.stdout.buffer.write(b'hi'); sys.stdout.flush(); time.sleep(2)",
            ],
            TrackerConfig::default(),
        )
        .unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saw_term_updated = false;
        while Instant::now() < deadline && !saw_term_updated {
            for ev in s.poll(Instant::now()) {
                if matches!(ev, SessionEvent::TermUpdated) {
                    saw_term_updated = true;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(saw_term_updated, "expected at least one TermUpdated event");
    }
```

Note that the Stage 3 `poll_routes_osc_1338_through_dispatcher_and_tracker` test currently uses `SessionEvent::PassThrough` only via the OSC-1338 path — which doesn't return `PassThrough` events anyway (the dispatcher consumes them as `AiState`). That test should still compile against `TermUpdated` without changes since it never references the `PassThrough` variant by name.

The Stage 3 `app::tests::send_input_writes_to_active_tab` DOES reference `SessionEvent::PassThrough(bytes)` and needs refactoring (Step 5 below).

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow --lib session::session
```

Expected: compile errors — `SessionEvent::TermUpdated` not defined; `s.term()` not defined.

- [ ] **Step 2: Modify `SessionEvent`, add `term`/`parser` fields, modify `spawn` and `poll`, add `term()` accessor**

Replace the contents of `crates/vibeflow/src/session/session.rs` with:

```rust
//! `PtySession` — one tab's PTY child, reader thread, OSC dispatcher, AI-state
//! tracker, and `alacritty_terminal::Term`. All driven from the main thread
//! via a single-producer single-consumer channel.

use std::io::Write;
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config as TermConfig, Term};
use alacritty_terminal::vte::ansi::Processor;
use portable_pty::Child;

use crate::session::osc::{DispatchEvent, OscDispatcher};
use crate::session::pty::{spawn_pty, PtyHandles};
use crate::session::tracker::{AiStateTracker, TabState, TrackerConfig, TrackerInput};

/// Default tab size when the session is first spawned. The window size in
/// `WindowApp::resumed` calls `App::resize_all` shortly after, which calls
/// `PtySession::resize` and updates both the PTY and `Term`.
const DEFAULT_ROWS: u16 = 24;
const DEFAULT_COLS: u16 = 80;

/// Public event type the `App` observes from a session, beyond just the
/// underlying [`DispatchEvent`]. `Died` lets the App detect when the child
/// exits and the reader thread has finished. `TermUpdated` is the redraw
/// trigger — bytes were consumed by the per-session [`Term`], so the grid
/// changed and the renderer should refresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    /// State of the per-session tracker just changed to this value.
    StateChanged(TabState),
    /// Bytes were consumed by [`Term`]; the grid changed. The renderer reads
    /// the current grid via [`PtySession::term`] / [`crate::app::App::active_term`].
    TermUpdated,
    /// The child exited or the reader thread terminated. After this event,
    /// `is_alive()` returns false and further `poll()` calls produce nothing.
    Died,
}

/// One terminal tab's per-session machinery.
pub struct PtySession {
    /// Drains here when the reader thread sends bytes from the PTY master.
    rx: Receiver<Vec<u8>>,
    /// Used by [`Self::send_input`] to write keystrokes to the PTY master.
    writer: Box<dyn Write + Send>,
    /// The PTY master. Kept alive on the main thread; the reader thread holds a
    /// cloned `Box<dyn Read + Send>` whose lifetime is independent of this
    /// field. `MasterPty::resize` is called through this handle.
    master: Box<dyn portable_pty::MasterPty + Send>,
    /// Child process handle — used for liveness checks and explicit kill.
    child: Box<dyn Child + Send + Sync>,
    /// Reader thread handle. Owned by the session; joined when `Drop` runs.
    reader_thread: Option<JoinHandle<()>>,
    /// Per-session OSC parser.
    dispatcher: OscDispatcher,
    /// Per-session VT/ANSI parser. Drives `term` when fed via `Processor::advance`.
    parser: Processor,
    /// Per-session terminal grid (alacritty_terminal). Source of truth for
    /// what the cell renderer draws.
    term: Term<VoidListener>,
    /// Per-session state tracker.
    tracker: AiStateTracker,
    /// True until either the child exits or the reader-thread errors out.
    alive: bool,
}

impl PtySession {
    /// Spawn a child via the given `argv` on a fresh pseudoterminal and start
    /// the reader thread.
    ///
    /// # Errors
    /// Propagates PTY-spawn or thread-creation failures.
    pub fn spawn(argv: &[&str], config: TrackerConfig) -> std::io::Result<Self> {
        let PtyHandles {
            reader,
            writer,
            child,
            master,
        } = spawn_pty(argv)?;
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let mut reader = reader;
        let reader_thread = thread::Builder::new()
            .name("vibeflow-pty-reader".into())
            .spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if tx.send(buf[..n].to_vec()).is_err() {
                                break;
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
            })?;

        let term_size = TermSize::new(DEFAULT_COLS as usize, DEFAULT_ROWS as usize);
        let term = Term::new(TermConfig::default(), &term_size, VoidListener);

        Ok(Self {
            rx,
            writer,
            master,
            child,
            reader_thread: Some(reader_thread),
            dispatcher: OscDispatcher::new(),
            parser: Processor::new(),
            term,
            tracker: AiStateTracker::new(config),
            alive: true,
        })
    }

    /// Current visual state of this session's tab.
    #[must_use]
    pub fn state(&self) -> TabState {
        self.tracker.state()
    }

    /// Drain every pending byte chunk off the reader channel, run each through
    /// the dispatcher, route resulting events into the tracker AND the per-session
    /// `Term`, and return the public-facing [`SessionEvent`]s for the App.
    /// Non-blocking — returns immediately if the channel is empty.
    pub fn poll(&mut self, now: Instant) -> Vec<SessionEvent> {
        let mut events = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(chunk) => {
                    for ev in self.dispatcher.feed(&chunk) {
                        match ev {
                            DispatchEvent::AiState(frame) => {
                                if self.tracker.on_input(TrackerInput::AiFrame(frame), now) {
                                    events.push(SessionEvent::StateChanged(self.tracker.state()));
                                }
                            }
                            DispatchEvent::Prompt(marker) => {
                                if self.tracker.on_input(TrackerInput::Prompt(marker), now) {
                                    events.push(SessionEvent::StateChanged(self.tracker.state()));
                                }
                            }
                            DispatchEvent::PassThrough(bytes) => {
                                self.tracker.on_input(TrackerInput::OutputObserved, now);
                                // Feed bytes through the VT parser into Term. This is
                                // where the grid actually updates.
                                for &byte in &bytes {
                                    self.parser.advance(&mut self.term, byte);
                                }
                                events.push(SessionEvent::TermUpdated);
                            }
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if self.alive {
                        self.alive = false;
                        events.push(SessionEvent::Died);
                    }
                    break;
                }
            }
        }
        events
    }

    /// Write keystroke bytes to the PTY master.
    ///
    /// # Errors
    /// Propagates any underlying `io::Error` from the writer.
    pub fn send_input(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    /// Run the tracker's timeout checks at `now`. Returns a [`SessionEvent`]
    /// per timeout-driven state change (currently zero or one event).
    pub fn tick(&mut self, now: Instant) -> Vec<SessionEvent> {
        if self.tracker.tick(now) {
            vec![SessionEvent::StateChanged(self.tracker.state())]
        } else {
            Vec::new()
        }
    }

    /// Toggle the Tier 3 heuristic-silence inference. The App calls this when
    /// the foreground process matches the configured AI-tool list.
    pub fn set_heuristic_active(&mut self, active: bool) {
        self.tracker.set_heuristic_active(active);
    }

    /// Resize the PTY to `rows` rows × `cols` cols, AND resize the per-session
    /// `Term` so the grid layout matches.
    ///
    /// # Errors
    /// Wraps `portable_pty`'s typed error via `io::Error::other`.
    pub fn resize(&mut self, rows: u16, cols: u16) -> std::io::Result<()> {
        self.master
            .resize(portable_pty::PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(std::io::Error::other)?;
        self.term
            .resize(TermSize::new(cols as usize, rows as usize));
        Ok(())
    }

    /// Whether the child is still running and the reader thread alive.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.alive
    }

    /// Read-only access to the per-session `Term` for rendering.
    #[must_use]
    pub fn term(&self) -> &Term<VoidListener> {
        &self.term
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        if let Some(handle) = self.reader_thread.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    // ... existing Stage 2/3/4 tests unchanged below, except as noted ...
}
```

The body of `mod tests` keeps all prior tests verbatim. `PtySession::resize` was previously `&self` (Stage 4 Task 1) — note that this task changes it to `&mut self` because `term.resize` requires `&mut`. The cascade: `App::resize_all` was `&self` (Stage 4 Task 1) — that needs to change to `&mut self` too. See Step 4.

Also note: the existing test `tick_fires_stale_state_timeout` and `set_heuristic_active_toggles_tier_3` access `s.dispatcher.feed(...)` and `s.tracker.on_input(...)` directly. Those still work (private-field-in-mod-tests idiom). No test code outside the new tests added in Step 1 needs to change in this file.

- [ ] **Step 3: Add `App::active_term` and update `App::resize_all` to take `&mut self`**

In `crates/vibeflow/src/app.rs`, locate the `resize_all` method and change its signature to `&mut self`. Then add `active_term` after it.

Replace this block:

```rust
    pub fn resize_all(&self, rows: u16, cols: u16) -> std::io::Result<()> {
        let mut first_error: Option<std::io::Error> = None;
        for tab in &self.tabs {
            if let Err(e) = tab.resize(rows, cols) {
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
        match first_error {
            None => Ok(()),
            Some(e) => Err(e),
        }
    }
```

with:

```rust
    pub fn resize_all(&mut self, rows: u16, cols: u16) -> std::io::Result<()> {
        let mut first_error: Option<std::io::Error> = None;
        for tab in &mut self.tabs {
            if let Err(e) = tab.resize(rows, cols) {
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
        match first_error {
            None => Ok(()),
            Some(e) => Err(e),
        }
    }

    /// Read-only access to the active tab's [`alacritty_terminal::term::Term`]
    /// for rendering. Returns `None` if there are no tabs.
    #[must_use]
    pub fn active_term(
        &self,
    ) -> Option<&alacritty_terminal::term::Term<alacritty_terminal::event::VoidListener>> {
        self.tabs.get(self.active).map(|t| t.term())
    }
```

The `&mut self` change ripples: callers of `App::resize_all` (currently just `WindowApp::window_event`'s `Resized` arm and `WindowApp::resumed`) need a mutable borrow of `self.app`. Both already have one.

- [ ] **Step 4: Update `WindowApp::handle_session_event` for the `TermUpdated` variant**

In `crates/vibeflow/src/window.rs`, replace the `PassThrough` arm in `handle_session_event`:

Old:

```rust
            SessionEvent::PassThrough(bytes) => {
                // Stage 5 sends these into alacritty_terminal::Term::input.
                // Stage 4 just records the byte count at trace level so we can
                // sanity-check throughput from the log without spamming.
                tracing::trace!(tab = idx, bytes = bytes.len(), "passthrough");
            }
```

New:

```rust
            SessionEvent::TermUpdated => {
                // Bytes were fed into the per-session Term in PtySession::poll.
                // Request a redraw so the renderer reads the new grid contents.
                tracing::trace!(tab = idx, "term updated");
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
```

- [ ] **Step 5: Refactor the Stage 3 `app::tests::send_input_writes_to_active_tab` test**

In `crates/vibeflow/src/app.rs`, locate the `send_input_writes_to_active_tab` test (in `mod tests`). The current body asserts `b"hi"` arrives via `SessionEvent::PassThrough`. Replace it with a body that asserts via the Term grid:

Old:

```rust
    #[test]
    fn send_input_writes_to_active_tab() {
        let mut app = App::new();
        app.new_tab(&["/bin/cat"]).unwrap();
        app.send_input(b"hi\n").unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut got = false;
        while Instant::now() < deadline && !got {
            for (_, ev) in app.poll_all(Instant::now()) {
                if let SessionEvent::PassThrough(bytes) = ev {
                    if bytes.windows(2).any(|w| w == b"hi") {
                        got = true;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(got, "expected `hi` to round-trip through cat");
        let _ = app.send_input(&[0x04]);
    }
```

New:

```rust
    #[test]
    fn send_input_writes_to_active_tab() {
        let mut app = App::new();
        app.new_tab(&["/bin/cat"]).unwrap();
        app.send_input(b"hi\n").unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saw_hi = false;
        while Instant::now() < deadline && !saw_hi {
            // Drain any TermUpdated/StateChanged events; their side effect
            // is updating the per-session Term, which we read below.
            let _events = app.poll_all(Instant::now());
            if let Some(term) = app.active_term() {
                let row0: String = term
                    .renderable_content()
                    .display_iter
                    .filter(|i| i.point.line.0 == 0)
                    .map(|i| i.cell.c)
                    .collect();
                if row0.contains("hi") {
                    saw_hi = true;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(saw_hi, "expected `hi` in active tab's grid");
        let _ = app.send_input(&[0x04]);
    }
```

- [ ] **Step 6: Verify all tests + fmt + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: every Stage-4 test still passes, plus the 2 new tests in `session::session` and the refactored Stage-3 test in `app`. Total lib-test count rises from 78 to 80.

If any prior test fails because it pattern-matches on `SessionEvent::PassThrough(...)` — find it and refactor to use `TermUpdated` (no payload) or to read from `term()`/`active_term()`. The most likely culprits: the `_unused_session_event_silences_dead_code`-era tests in `app/tests/` (Stage 3 deleted those, so this should be clean) and the `poll_all_collects_state_changes_from_each_session` test (which only matches on `StateChanged`, so it's fine).

- [ ] **Step 7: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/session/session.rs crates/vibeflow/src/app.rs crates/vibeflow/src/window.rs
git commit -m "feat(session,app,window): integrate alacritty_terminal::Term + rename PassThrough → TermUpdated"
```

---

## Task 3: Glyph atlas — fontdue rasterization + GPU texture upload

**Files:**
- Modify: `crates/vibeflow/src/render/atlas.rs`

The atlas pre-rasterises printable ASCII (`0x20` space through `0x7e` tilde — 95 glyphs) at a fixed pixel size using `fontdue`, packs them into a single 2D texture with a uniform glyph-cell pitch, and exposes per-glyph index + UV math to the shader. Stage 7 (cosmic-text) replaces this with a dynamic atlas that handles full Unicode + ligatures + emoji.

The pure parts (UV-coordinate math, `glyph_index(char) -> u32` lookup) are TDD'd. The GPU texture upload is verified via `cargo build` and the manual smoke run.

For Stage 5, the atlas size is hardcoded: `CELL_PX = 16` × cell width = the per-glyph cell pitch in the atlas; we lay out the glyphs in a 16-column × 6-row grid (16 × 6 = 96 ≥ 95). The atlas texture is `(16 * cell_w) × (6 * cell_h)` pixels.

Note that `cell_w` is the font's actual advance width (computed from the font), not the design `CELL_PX` from `window.rs` (which was 8 — that placeholder was always wrong and Stage 7 replaces it with proper font metrics). This task introduces the *real* per-cell pixel size, which Stage 6's window resize math should use.

- [ ] **Step 1: Write the failing tests for the pure logic**

Replace the contents of `crates/vibeflow/src/render/atlas.rs` with:

```rust
//! Glyph atlas. Pre-rasterises printable ASCII (0x20..=0x7e) via fontdue at the
//! configured pixel size, packs the glyphs into a single wgpu texture, and
//! exposes UV / metric lookups by character. Stage 7 will replace fontdue with
//! cosmic-text shaping for full Unicode + ligatures + emoji.

use fontdue::{Font, FontSettings};

/// Range of code points pre-rendered into the Stage 5 atlas.
const ATLAS_FIRST: u32 = 0x20; // space
const ATLAS_LAST: u32 = 0x7e; // tilde
/// Number of glyphs in the atlas.
const ATLAS_GLYPHS: u32 = ATLAS_LAST - ATLAS_FIRST + 1;
/// Layout: 16 glyphs per row, 6 rows.
const ATLAS_COLS: u32 = 16;
const ATLAS_ROWS: u32 = 6; // 16 * 6 = 96 >= 95 glyphs

/// Map a `char` to its glyph index in the atlas. Returns `None` for any
/// character outside the pre-rendered range. Stage 7 swaps this for a dynamic
/// lookup that handles arbitrary Unicode.
#[must_use]
pub fn glyph_index(c: char) -> Option<u32> {
    let code = c as u32;
    if (ATLAS_FIRST..=ATLAS_LAST).contains(&code) {
        Some(code - ATLAS_FIRST)
    } else {
        None
    }
}

/// Compute the pixel-space rectangle of a glyph in the atlas, given its index
/// and the per-cell pitch in pixels.
#[must_use]
pub fn glyph_pixel_rect(index: u32, cell_w: u32, cell_h: u32) -> (u32, u32, u32, u32) {
    let col = index % ATLAS_COLS;
    let row = index / ATLAS_COLS;
    (col * cell_w, row * cell_h, cell_w, cell_h)
}

/// Total atlas dimensions in pixels.
#[must_use]
pub fn atlas_pixel_size(cell_w: u32, cell_h: u32) -> (u32, u32) {
    (ATLAS_COLS * cell_w, ATLAS_ROWS * cell_h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_index_of_space_is_zero() {
        assert_eq!(glyph_index(' '), Some(0));
    }

    #[test]
    fn glyph_index_of_capital_a() {
        assert_eq!(glyph_index('A'), Some(0x41 - 0x20));
    }

    #[test]
    fn glyph_index_of_tilde_is_last() {
        assert_eq!(glyph_index('~'), Some(0x7e - 0x20));
        assert_eq!(glyph_index('~').unwrap(), ATLAS_GLYPHS - 1);
    }

    #[test]
    fn glyph_index_of_unicode_returns_none() {
        assert_eq!(glyph_index('é'), None);
        assert_eq!(glyph_index('🦀'), None);
    }

    #[test]
    fn glyph_index_of_control_char_returns_none() {
        assert_eq!(glyph_index('\n'), None);
        assert_eq!(glyph_index('\t'), None);
        assert_eq!(glyph_index('\x1b'), None);
    }

    #[test]
    fn glyph_pixel_rect_for_index_0_is_top_left() {
        assert_eq!(glyph_pixel_rect(0, 8, 16), (0, 0, 8, 16));
    }

    #[test]
    fn glyph_pixel_rect_wraps_to_next_row_after_atlas_cols_glyphs() {
        // Index 16 → row 1, col 0. (8, 16) cell → x=0, y=16.
        assert_eq!(glyph_pixel_rect(ATLAS_COLS, 8, 16), (0, 16, 8, 16));
    }

    #[test]
    fn atlas_pixel_size_multiplies_cells_by_layout() {
        assert_eq!(atlas_pixel_size(8, 16), (16 * 8, 6 * 16));
    }
}
```

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo test -p vibeflow --lib render::atlas
```

Expected: 8 tests pass.

- [ ] **Step 2: Add the `GlyphAtlas` GPU type (verified by `cargo build`, not unit tests)**

Append to `crates/vibeflow/src/render/atlas.rs` (above the `#[cfg(test)] mod tests` block):

```rust
/// Embedded JetBrains Mono Regular at compile time. ~270 KB.
const FONT_BYTES: &[u8] = include_bytes!("../../assets/JetBrainsMono-Regular.ttf");

/// Pixel size at which the Stage 5 atlas is rasterised. Stage 6+ will read this
/// from `[window.font_size]` in the TOML config.
pub const FONT_PX: f32 = 16.0;

/// GPU-side glyph atlas. Owns the pre-rendered texture and reports the
/// per-cell pitch the renderer uses for layout.
pub struct GlyphAtlas {
    /// Texture holding the rasterised glyphs in a row-major grid.
    pub texture: wgpu::Texture,
    /// View used by the bind group.
    pub view: wgpu::TextureView,
    /// Linear sampler for atlas reads.
    pub sampler: wgpu::Sampler,
    /// Per-cell pitch in physical pixels — the integer width and height that
    /// each glyph occupies in the atlas (and, by extension, on the screen).
    pub cell_w_px: u32,
    pub cell_h_px: u32,
}

impl GlyphAtlas {
    /// Rasterise the printable-ASCII range and upload it to a wgpu texture.
    ///
    /// # Errors
    /// Returns `Err` if the embedded font can't be parsed or fontdue can't
    /// produce metrics for the configured pixel size.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> anyhow::Result<Self> {
        let font = Font::from_bytes(FONT_BYTES, FontSettings::default())
            .map_err(|e| anyhow::anyhow!("parse font: {e}"))?;

        // Compute the per-cell pitch from the font's line metrics.
        let line_metrics = font
            .horizontal_line_metrics(FONT_PX)
            .ok_or_else(|| anyhow::anyhow!("no horizontal line metrics for font"))?;
        let cell_h_px = (line_metrics.new_line_size.ceil() as u32).max(1);
        // Use 'M' as the proxy for column width — JetBrains Mono is monospace,
        // so every glyph reports the same advance, but 'M' is a safe choice.
        let (m_metrics, _) = font.rasterize('M', FONT_PX);
        let cell_w_px = (m_metrics.advance_width.ceil() as u32).max(1);

        let (atlas_w_px, atlas_h_px) = atlas_pixel_size(cell_w_px, cell_h_px);

        // Build the atlas pixel buffer (single-channel grayscale alpha, R8Unorm).
        let mut atlas_pixels = vec![0u8; (atlas_w_px * atlas_h_px) as usize];
        let ascent = line_metrics.ascent;
        for code in ATLAS_FIRST..=ATLAS_LAST {
            let c = char::from_u32(code).expect("ASCII range");
            let idx = glyph_index(c).expect("in range");
            let (gx, gy, _, _) = glyph_pixel_rect(idx, cell_w_px, cell_h_px);
            let (metrics, bitmap) = font.rasterize(c, FONT_PX);
            if metrics.width == 0 || metrics.height == 0 {
                continue; // space and other empty glyphs
            }
            // fontdue places the glyph relative to the baseline; we offset so
            // the glyph sits inside the cell with its baseline at `ascent` rows
            // below the cell top.
            let x_offset = metrics.xmin.max(0) as u32;
            let y_offset = (ascent.ceil() as i32 - (metrics.ymin + metrics.height as i32))
                .max(0) as u32;
            for y in 0..metrics.height as u32 {
                for x in 0..metrics.width as u32 {
                    let dst_x = gx + x_offset + x;
                    let dst_y = gy + y_offset + y;
                    if dst_x >= gx + cell_w_px || dst_y >= gy + cell_h_px {
                        continue; // clip out-of-cell
                    }
                    let src = bitmap[(y * metrics.width as u32 + x) as usize];
                    atlas_pixels[(dst_y * atlas_w_px + dst_x) as usize] = src;
                }
            }
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vibeflow-glyph-atlas"),
            size: wgpu::Extent3d {
                width: atlas_w_px,
                height: atlas_h_px,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &atlas_pixels,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(atlas_w_px),
                rows_per_image: Some(atlas_h_px),
            },
            wgpu::Extent3d {
                width: atlas_w_px,
                height: atlas_h_px,
                depth_or_array_layers: 1,
            },
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("vibeflow-glyph-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Ok(Self {
            texture,
            view,
            sampler,
            cell_w_px,
            cell_h_px,
        })
    }

    /// Atlas width × height in pixels.
    #[must_use]
    pub fn pixel_size(&self) -> (u32, u32) {
        atlas_pixel_size(self.cell_w_px, self.cell_h_px)
    }

    /// Width of one glyph cell in the atlas (and, by extension, the per-cell
    /// pitch the grid renderer uses on screen).
    #[must_use]
    pub fn cell_pitch(&self) -> (u32, u32) {
        (self.cell_w_px, self.cell_h_px)
    }
}
```

- [ ] **Step 3: Verify build + fmt + clippy + tests**

```bash
cd /home/bhengen/dev/vibeflow
cargo build -p vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
cargo test -p vibeflow --lib render::atlas
```

Expected: clean build, 8 atlas-module tests pass.

If clippy complains about `dead_code` on `GlyphAtlas` (it's pub, so this would be unusual — but it has no caller yet outside Stage 5 Tasks 4–5), narrow the suppression with a comment "first user is `Renderer` in Stage 5 Task 4".

- [ ] **Step 4: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/render/atlas.rs
git commit -m "feat(render): GlyphAtlas pre-renders ASCII printable into a wgpu texture"
```

---

## Task 4: WGSL shader + `GridPipeline` state

**Files:**
- Modify: `crates/vibeflow/src/render/grid.wgsl`
- Modify: `crates/vibeflow/src/render/grid.rs`

This task creates the wgpu pipeline-state object that the cell render pass will use. Stage 5 ships a single pipeline that draws every cell of the active grid as one instanced draw call. The vertex shader expands per-instance data into 6 vertices (two triangles); the fragment shader samples the atlas and mixes background and foreground colors based on the atlas alpha.

No tests in this task — wgpu pipeline construction has no behavior worth unit-testing in isolation. Verification is `cargo build` + the manual smoke run in Task 8.

- [ ] **Step 1: Write the WGSL shader**

Replace the contents of `crates/vibeflow/src/render/grid.wgsl` with:

```wgsl
// vibeflow Stage 5 cell-grid shader.
//
// One draw call. Per-frame uniform supplies grid + surface dimensions in
// pixels. Per-instance buffer supplies one CellInstance per visible cell:
// (col, row, glyph_index, fg, bg). Vertex shader expands the instance into
// 6 vertices (two triangles) covering the cell rectangle. Fragment shader
// samples the atlas's grayscale alpha and mixes bg → fg.

struct GridUniform {
    surface_size_px: vec2<f32>,   // viewport size in physical pixels
    cell_size_px:    vec2<f32>,   // per-cell pitch in physical pixels
    atlas_size_px:   vec2<f32>,   // atlas texture size in pixels
    atlas_cells:     vec2<u32>,   // atlas layout (cols, rows of glyphs)
    _pad:            vec2<u32>,
};

@group(0) @binding(0) var<uniform> u: GridUniform;
@group(0) @binding(1) var atlas_texture: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct VsIn {
    @builtin(vertex_index)   vertex_id:    u32,
    @builtin(instance_index) instance_id:  u32,
    @location(0) cell:        vec4<u32>, // .x=col .y=row .z=glyph_index .w=_pad
    @location(1) fg:          vec4<f32>,
    @location(2) bg:          vec4<f32>,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv:             vec2<f32>,
    @location(1) fg:             vec4<f32>,
    @location(2) bg:             vec4<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    // The 6 vertices of the cell quad, mapping vertex_id → (corner, uv).
    // Triangle 1: (0,0) (1,0) (0,1). Triangle 2: (1,0) (1,1) (0,1).
    var quad_offsets = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    let corner = quad_offsets[in.vertex_id];

    let col      = f32(in.cell.x);
    let row      = f32(in.cell.y);
    let glyph    = in.cell.z;

    // Cell-pixel position: top-left of the cell.
    let cell_top_left_px = vec2<f32>(col, row) * u.cell_size_px;
    let pos_px           = cell_top_left_px + corner * u.cell_size_px;

    // Convert to clip space. Viewport [0..W,0..H] → NDC [-1..1, 1..-1] (Y flipped).
    let ndc = (pos_px / u.surface_size_px) * 2.0 - vec2<f32>(1.0, 1.0);
    let clip_pos = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);

    // Atlas UV: locate the glyph's cell, then sample within it.
    let atlas_col = f32(glyph % u.atlas_cells.x);
    let atlas_row = f32(glyph / u.atlas_cells.x);
    let glyph_top_left_px = vec2<f32>(atlas_col, atlas_row) * u.cell_size_px;
    let glyph_pos_px      = glyph_top_left_px + corner * u.cell_size_px;
    let uv                = glyph_pos_px / u.atlas_size_px;

    var out: VsOut;
    out.clip_pos = clip_pos;
    out.uv       = uv;
    out.fg       = in.fg;
    out.bg       = in.bg;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Atlas is R8Unorm → only .r is meaningful; treat as alpha.
    let alpha = textureSample(atlas_texture, atlas_sampler, in.uv).r;
    let rgb   = mix(in.bg.rgb, in.fg.rgb, alpha);
    return vec4<f32>(rgb, 1.0);
}
```

- [ ] **Step 2: Implement `GridPipeline`**

Replace the contents of `crates/vibeflow/src/render/grid.rs` with:

```rust
//! Cell-grid render pipeline. Owns the wgpu pipeline-state object, the bind
//! group for the atlas texture + sampler, the per-frame uniform buffer, and
//! the dynamically-grown instance buffer.

use anyhow::Result;
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::render::atlas::GlyphAtlas;

/// Per-instance data for the cell render pass. Layout matches `VsIn` in
/// `grid.wgsl`. The packed `cell` u32×4 carries column, row, glyph index, and
/// a padding word so the next field aligns to 16 bytes.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct CellInstance {
    pub cell: [u32; 4],   // .x=col, .y=row, .z=glyph_index, .w=_pad
    pub fg:   [f32; 4],
    pub bg:   [f32; 4],
}

impl CellInstance {
    pub fn new(col: u32, row: u32, glyph: u32, fg: [f32; 4], bg: [f32; 4]) -> Self {
        Self {
            cell: [col, row, glyph, 0],
            fg,
            bg,
        }
    }
}

/// Per-frame uniform. Layout matches `GridUniform` in `grid.wgsl`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct GridUniform {
    surface_size_px: [f32; 2],
    cell_size_px:    [f32; 2],
    atlas_size_px:   [f32; 2],
    atlas_cells:     [u32; 2],
    _pad:            [u32; 2],
}

/// Cell-grid render pipeline. One per [`crate::render::Renderer`].
pub struct GridPipeline {
    pipeline:        wgpu::RenderPipeline,
    bind_group:      wgpu::BindGroup,
    uniform_buffer:  wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    instance_capacity: u64,  // current allocated capacity in instances
}

const INITIAL_INSTANCE_CAPACITY: u64 = 80 * 24; // matches default Term size
const INSTANCE_STRIDE: u64 = std::mem::size_of::<CellInstance>() as u64;

impl GridPipeline {
    /// Build the pipeline. Borrows the device + queue from the parent
    /// `Renderer`; references the atlas texture/view/sampler.
    ///
    /// # Errors
    /// Currently infallible after the atlas is built; returns `Result` for
    /// future-proofing (Stage 6+ may add fallible config loading).
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        atlas: &GlyphAtlas,
    ) -> Result<Self> {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vibeflow-grid-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("grid.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vibeflow-grid-bind-group-layout"),
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
            label: Some("vibeflow-grid-uniform"),
            size: std::mem::size_of::<GridUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vibeflow-grid-bind-group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&atlas.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&atlas.sampler),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vibeflow-grid-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vibeflow-grid-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: INSTANCE_STRIDE,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Uint32x4,
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
                    ],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
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
            cache: None,
        });

        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vibeflow-grid-instances"),
            size: INSTANCE_STRIDE * INITIAL_INSTANCE_CAPACITY,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            pipeline,
            bind_group,
            uniform_buffer,
            instance_buffer,
            instance_capacity: INITIAL_INSTANCE_CAPACITY,
        })
    }

    /// Resize the instance buffer if the requested capacity exceeds the
    /// current allocation. Doubles the capacity each time it grows.
    pub fn ensure_instance_capacity(&mut self, device: &wgpu::Device, needed: u64) {
        if needed <= self.instance_capacity {
            return;
        }
        let mut new_capacity = self.instance_capacity;
        while new_capacity < needed {
            new_capacity *= 2;
        }
        self.instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vibeflow-grid-instances"),
            size: INSTANCE_STRIDE * new_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.instance_capacity = new_capacity;
    }

    /// Upload uniforms + instance data and submit one instanced draw call into
    /// the supplied `RenderPass`. Caller must have already begun the render
    /// pass on the surface texture.
    pub fn draw<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        queue: &wgpu::Queue,
        instances: &[CellInstance],
        surface_size_px: (u32, u32),
        atlas_size_px: (u32, u32),
        cell_size_px: (u32, u32),
        atlas_cells: (u32, u32),
    ) {
        if instances.is_empty() {
            return;
        }
        let uniform = GridUniform {
            surface_size_px: [surface_size_px.0 as f32, surface_size_px.1 as f32],
            cell_size_px:    [cell_size_px.0 as f32, cell_size_px.1 as f32],
            atlas_size_px:   [atlas_size_px.0 as f32, atlas_size_px.1 as f32],
            atlas_cells:     [atlas_cells.0, atlas_cells.1],
            _pad:            [0, 0],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
        queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(instances));

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
        pass.draw(0..6, 0..(instances.len() as u32));
    }
}

// Suppress until Renderer wires GridPipeline in Task 5/6.
#[allow(dead_code)]
const _UNUSED_BUT_REFERENCED_IN_NEXT_TASK: () = ();
```

Note: the `#[allow(dead_code)]` on the trailing const is a placeholder so clippy doesn't fire on the otherwise-unreferenced `GridPipeline` struct between this task and Task 5. Remove it in Task 5 once `Renderer` constructs a `GridPipeline`.

If clippy complains about unused imports of `DeviceExt` (we don't actually use `create_buffer_init` here — just `create_buffer`), drop that line. Verify by running clippy.

- [ ] **Step 3: Verify build + fmt + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo build -p vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: clean build. The shader code is compiled at runtime by wgpu, not at `cargo build` time — syntax errors in `grid.wgsl` only show up when `Renderer::new` runs (Task 5). To catch them earlier, you can use the `naga-cli` if installed: `naga grid.wgsl --validate` — but it's optional.

- [ ] **Step 4: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/render/grid.rs crates/vibeflow/src/render/grid.wgsl
git commit -m "feat(render): grid WGSL shader and GridPipeline state"
```

---

## Task 5: `Renderer::render` consumes `Term` and draws cells

**Files:**
- Modify: `crates/vibeflow/src/render/mod.rs`

This task replaces the Stage-4 clear-color body of `Renderer::render` with the grid-rendering implementation. `Renderer` gains `atlas: GlyphAtlas` and `grid_pipeline: GridPipeline` fields, and the `render` signature becomes `render(&mut self, term: Option<&Term<VoidListener>>) -> Result<(), wgpu::SurfaceError>`. When `term` is `None`, the renderer falls back to clear-color (matches Stage 4 behavior — useful when the active tab is dead and `Term` rendering would be wrong).

Cursor rendering is added in Task 6.

- [ ] **Step 1: Modify `Renderer` to construct + own the new state**

In `crates/vibeflow/src/render/mod.rs`, find the `Renderer` struct definition and the `Renderer::new` body. Update them to construct a `GlyphAtlas` and a `GridPipeline`, and store them.

Replace the existing struct definition:

```rust
pub struct Renderer {
    /// Kept so the surface's borrow stays valid for the renderer's lifetime.
    _window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,
}
```

with:

```rust
pub struct Renderer {
    /// Kept so the surface's borrow stays valid for the renderer's lifetime.
    _window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,
    /// Pre-rendered ASCII glyph atlas. Stage 7 replaces with cosmic-text.
    atlas: crate::render::atlas::GlyphAtlas,
    /// Cell-grid render pipeline. One instanced draw per frame.
    grid_pipeline: crate::render::grid::GridPipeline,
}
```

Then update `Renderer::new` to construct the atlas + pipeline at the end. Find the existing return statement:

```rust
        Ok(Self {
            _window: window,
            surface,
            device,
            queue,
            surface_config,
        })
```

Replace with:

```rust
        let atlas = crate::render::atlas::GlyphAtlas::new(&device, &queue)?;
        let grid_pipeline = crate::render::grid::GridPipeline::new(&device, format, &atlas)?;

        Ok(Self {
            _window: window,
            surface,
            device,
            queue,
            surface_config,
            atlas,
            grid_pipeline,
        })
```

Drop the `#[allow(dead_code)] const _UNUSED_BUT_REFERENCED_IN_NEXT_TASK` line from `grid.rs` since `GridPipeline` is now constructed by `Renderer`.

- [ ] **Step 2: Replace `render`'s body**

Find the existing `Renderer::render` definition. The Stage-4 body just clears to `CLEAR_COLOR`. Replace with:

```rust
    /// Submit a single-frame render. If `term` is `Some`, draws every visible
    /// cell of the grid; if `None`, just clears to the dark theme color.
    pub fn render(
        &mut self,
        term: Option<&alacritty_terminal::term::Term<alacritty_terminal::event::VoidListener>>,
    ) -> std::result::Result<(), wgpu::SurfaceError> {
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

            if let Some(term) = term {
                let instances = build_cell_instances(term, &self.atlas);
                if !instances.is_empty() {
                    self.grid_pipeline.ensure_instance_capacity(
                        &self.device,
                        instances.len() as u64,
                    );
                    let (atlas_w, atlas_h) = self.atlas.pixel_size();
                    let (cell_w, cell_h) = self.atlas.cell_pitch();
                    let surface_size = (
                        self.surface_config.width,
                        self.surface_config.height,
                    );
                    self.grid_pipeline.draw(
                        &mut pass,
                        &self.queue,
                        &instances,
                        surface_size,
                        (atlas_w, atlas_h),
                        (cell_w, cell_h),
                        crate::render::atlas::ATLAS_LAYOUT,
                    );
                }
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
        Ok(())
    }
```

- [ ] **Step 3: Add `build_cell_instances` helper + `ATLAS_LAYOUT` const**

Append at the end of `crates/vibeflow/src/render/mod.rs` (after the `Renderer` impl, outside any other impl block):

```rust
/// Walk the active grid and emit one [`crate::render::grid::CellInstance`]
/// per visible cell. Cells whose character is outside the atlas range
/// (non-printable / non-ASCII for Stage 5) are emitted with the space-glyph
/// index — visually they show only the background color, which matches
/// well-behaved control characters.
fn build_cell_instances(
    term: &alacritty_terminal::term::Term<alacritty_terminal::event::VoidListener>,
    atlas: &crate::render::atlas::GlyphAtlas,
) -> Vec<crate::render::grid::CellInstance> {
    use alacritty_terminal::vte::ansi::Rgb;

    let content = term.renderable_content();
    let colors = content.colors;
    let fg_default = Rgb {
        r: 0xe5,
        g: 0xe5,
        b: 0xe5,
    };
    let bg_default = Rgb {
        r: 0x0e,
        g: 0x0e,
        b: 0x12,
    };

    let mut instances: Vec<crate::render::grid::CellInstance> = Vec::new();
    for indexed in content.display_iter {
        let row = indexed.point.line.0;
        if row < 0 {
            continue; // skip scrollback above the viewport — Stage 6+
        }
        let col = indexed.point.column.0 as u32;
        let cell = indexed.cell;
        let glyph = crate::render::atlas::glyph_index(cell.c).unwrap_or(0); // space
        let fg_rgb = crate::render::colors::resolve_color(cell.fg, colors, fg_default, bg_default);
        let bg_rgb = crate::render::colors::resolve_color(cell.bg, colors, fg_default, bg_default);
        let fg = rgb_to_f32(fg_rgb);
        let bg = rgb_to_f32(bg_rgb);
        instances.push(crate::render::grid::CellInstance::new(
            col, row as u32, glyph, fg, bg,
        ));
    }
    instances
}

fn rgb_to_f32(rgb: alacritty_terminal::vte::ansi::Rgb) -> [f32; 4] {
    [
        rgb.r as f32 / 255.0,
        rgb.g as f32 / 255.0,
        rgb.b as f32 / 255.0,
        1.0,
    ]
}
```

In `crates/vibeflow/src/render/atlas.rs`, expose the layout constants used by the renderer. Above the existing `glyph_index` function, change the `ATLAS_COLS` and `ATLAS_ROWS` consts from private to `pub`, and add a re-export tuple:

Replace:

```rust
/// Layout: 16 glyphs per row, 6 rows.
const ATLAS_COLS: u32 = 16;
const ATLAS_ROWS: u32 = 6; // 16 * 6 = 96 >= 95 glyphs
```

with:

```rust
/// Layout: 16 glyphs per row, 6 rows.
pub const ATLAS_COLS: u32 = 16;
pub const ATLAS_ROWS: u32 = 6; // 16 * 6 = 96 >= 95 glyphs

/// Atlas grid layout (cols, rows). Used by the grid shader's UV math.
pub const ATLAS_LAYOUT: (u32, u32) = (ATLAS_COLS, ATLAS_ROWS);
```

- [ ] **Step 4: Verify build + fmt + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo build -p vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

If the build fails because Renderer's existing tests can't construct one (none should — the Stage 4 plan didn't add tests for Renderer), inspect and report. Most likely cause if it does fail: shader compilation errors in `grid.wgsl` are runtime errors, not compile errors. So if `cargo build` passes, the WGSL is syntactically valid; semantic shader bugs surface in the smoke run.

- [ ] **Step 5: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/render/mod.rs crates/vibeflow/src/render/atlas.rs crates/vibeflow/src/render/grid.rs
git commit -m "feat(render): Renderer renders cell grid via GridPipeline + GlyphAtlas"
```

---

## Task 6: Block cursor rendering

**Files:**
- Modify: `crates/vibeflow/src/render/mod.rs`

The cursor is rendered as one extra cell at the cursor position with foreground and background swapped (block cursor, no blink). When `RenderableCursor::shape == CursorShape::Hidden`, no cursor instance is appended.

- [ ] **Step 1: Update `build_cell_instances` to append the cursor**

In `crates/vibeflow/src/render/mod.rs`, modify `build_cell_instances` to append a cursor instance after the regular cell loop. Replace the function with:

```rust
fn build_cell_instances(
    term: &alacritty_terminal::term::Term<alacritty_terminal::event::VoidListener>,
    atlas: &crate::render::atlas::GlyphAtlas,
) -> Vec<crate::render::grid::CellInstance> {
    use alacritty_terminal::vte::ansi::{CursorShape, Rgb};

    let content = term.renderable_content();
    let colors = content.colors;
    let fg_default = Rgb {
        r: 0xe5,
        g: 0xe5,
        b: 0xe5,
    };
    let bg_default = Rgb {
        r: 0x0e,
        g: 0x0e,
        b: 0x12,
    };

    let mut instances: Vec<crate::render::grid::CellInstance> = Vec::new();
    let cursor_pos = content.cursor;
    let cursor_visible = cursor_pos.shape != CursorShape::Hidden;
    let cursor_row_col = if cursor_visible {
        Some((cursor_pos.point.line.0, cursor_pos.point.column.0 as u32))
    } else {
        None
    };

    for indexed in content.display_iter {
        let row = indexed.point.line.0;
        if row < 0 {
            continue;
        }
        let col = indexed.point.column.0 as u32;
        let cell = indexed.cell;
        let glyph = crate::render::atlas::glyph_index(cell.c).unwrap_or(0);
        let fg_rgb = crate::render::colors::resolve_color(cell.fg, colors, fg_default, bg_default);
        let bg_rgb = crate::render::colors::resolve_color(cell.bg, colors, fg_default, bg_default);

        // If this is the cursor cell, swap fg and bg to draw a block cursor.
        let on_cursor = cursor_row_col == Some((row, col));
        let (fg, bg) = if on_cursor {
            (rgb_to_f32(bg_rgb), rgb_to_f32(fg_rgb))
        } else {
            (rgb_to_f32(fg_rgb), rgb_to_f32(bg_rgb))
        };

        instances.push(crate::render::grid::CellInstance::new(
            col, row as u32, glyph, fg, bg,
        ));
        let _ = atlas; // touched for future use; suppresses unused-param lint
    }
    instances
}
```

(The `_ = atlas;` line is a no-op kept so the function signature is unchanged from Task 5; clippy may flag it, in which case remove it.)

- [ ] **Step 2: Verify build + fmt + clippy**

```bash
cd /home/bhengen/dev/vibeflow
cargo build -p vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

- [ ] **Step 3: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/render/mod.rs
git commit -m "feat(render): block cursor rendered by inverting fg/bg on the cursor cell"
```

---

## Task 7: Wire `WindowApp` to pass active Term to `Renderer`

**Files:**
- Modify: `crates/vibeflow/src/window.rs`

The Stage-4 `RedrawRequested` arm calls `renderer.render()` with no argument. Stage 5's `Renderer::render` takes `Option<&Term>`. This task threads the call.

- [ ] **Step 1: Update the `RedrawRequested` arm**

In `crates/vibeflow/src/window.rs`, locate the existing `RedrawRequested` arm (Stage 4 Task 8 introduced the surface-error-aware version). Replace it:

Old:

```rust
            WindowEvent::RedrawRequested => {
                let Some(renderer) = self.renderer.as_mut() else {
                    return;
                };
                match renderer.render() {
                    Ok(()) => {}
                    Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                        renderer.reconfigure();
                    }
                    Err(wgpu::SurfaceError::OutOfMemory) => {
                        tracing::error!("GPU out of memory; exiting");
                        event_loop.exit();
                    }
                    Err(wgpu::SurfaceError::Timeout) => {
                        if let Some(window) = self.window.as_ref() {
                            window.request_redraw();
                        }
                    }
                }
            }
```

New (note the split borrow of `self.app` and `self.renderer`):

```rust
            WindowEvent::RedrawRequested => {
                let term = self.app.active_term();
                let Some(renderer) = self.renderer.as_mut() else {
                    return;
                };
                match renderer.render(term) {
                    Ok(()) => {}
                    Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                        renderer.reconfigure();
                    }
                    Err(wgpu::SurfaceError::OutOfMemory) => {
                        tracing::error!("GPU out of memory; exiting");
                        event_loop.exit();
                    }
                    Err(wgpu::SurfaceError::Timeout) => {
                        if let Some(window) = self.window.as_ref() {
                            window.request_redraw();
                        }
                    }
                }
            }
```

- [ ] **Step 2: Verify build + fmt + clippy + tests**

```bash
cd /home/bhengen/dev/vibeflow
cargo build -p vibeflow
cargo test -p vibeflow
cargo fmt --all -- --check && cargo clippy -p vibeflow --all-targets -- -D warnings
```

Expected: clean build; all tests pass.

- [ ] **Step 3: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add crates/vibeflow/src/window.rs
git commit -m "feat(window): pass active Term to Renderer on every RedrawRequested"
```

---

## Task 8: Manual smoke checklist for Stage 5

**Files:**
- Modify: `docs/TESTING.md`

- [ ] **Step 1: Append a Stage 5 section to `docs/TESTING.md`**

The current file has the Stage 4 section. Append the Stage 5 section at the bottom:

```markdown

## Stage 5 — alacritty_terminal grid + cell renderer

Run:

```bash
cd /home/bhengen/dev/vibeflow
cargo build --bin vibeflow
RUST_LOG=vibeflow=info ./target/debug/vibeflow
```

- [ ] A window opens within ~500 ms. The user's shell prompt is visible inside
  it, rendered in JetBrains Mono, white-ish on the dark grey background.
- [ ] The block cursor is visible at the prompt position, with inverted colors.
- [ ] Type `echo hello world`. Each keystroke appears on screen as you type.
- [ ] Press Enter. The shell runs the command and prints `hello world` on the
  next line; the prompt re-appears below it.
- [ ] Run `ls --color`. Files appear with ANSI 16 colors (blue for directories,
  green for executables, etc.).
- [ ] Run a 256-color test: `for i in {0..255}; do printf "\\033[48;5;${i}m %3d \\033[0m" $i; done; echo`.
  All 256 background colors render distinctly.
- [ ] Run a truecolor test: `printf '\\033[38;2;255;100;0mhello\\033[0m\\n'`.
  The text renders in orange (255, 100, 0).
- [ ] Resize the window. The prompt re-flows to the new width; the shell sees
  the new size (verify with `tput cols`).
- [ ] Run `vim` or `nano`. The full-screen UI renders. Cursor moves with arrow
  keys (Stage 8 actually wires arrows; for Stage 5, hjkl in vim normal mode
  works because they're letters).
- [ ] Run `clear`. Screen clears to the dark grey background, prompt at top.
- [ ] Press Ctrl+D at an empty prompt. Stderr shows `session died`. The
  rendered grid freezes at the last known state (no banner yet — that's
  Stage 6). Click the close button to exit.
```

- [ ] **Step 2: Walk through the checklist**

Run each item against `./target/debug/vibeflow`. Mark items complete (`- [x]`) as they pass. If any item fails, capture the failure mode and fix before tagging Stage 5.

- [ ] **Step 3: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add docs/TESTING.md
git commit -m "docs: Stage 5 manual smoke checklist"
```

---

## Task 9: Final verification + tag

- [ ] **Step 1: Full local CI dry-run**

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

Expected test counts: every Stage 4 test still passes (78 lib + 3 prior integration + 4 PTY integration + 27 protocol); plus Stage 5 adds:
- Task 1 colors: 9 tests
- Task 2 session: 2 tests + the refactored Stage 3 test (no net change there)
- Task 3 atlas: 8 tests

Net: ~97 lib tests + 27 protocol = ~124 Rust tests + 1 proptest + 15 npm.

- [ ] **Step 2: 60-second fuzz on the protocol parser**

```bash
cd /home/bhengen/dev/vibeflow/crates/vibeflow-protocol
cargo +nightly fuzz run parse -- -max_total_time=60
```

Expected: clean.

- [ ] **Step 3: Re-walk the smoke checklist**

Re-run `docs/TESTING.md`'s Stage 5 section to confirm nothing regressed during the final fmt/clippy passes.

- [ ] **Step 4: Tag the milestone**

```bash
cd /home/bhengen/dev/vibeflow
git tag -a stage5-grid-render-complete -m "alacritty_terminal grid + cell renderer complete (Stage 5 of v0.1)"
git tag --list
```

- [ ] **Step 5: Surface to user**

Report:
- Number of new commits on this stage (~9–10).
- Local CI dry-run result.
- New tag name.
- Whether the user wants Stage 6 (tab bar with the Notice indicator + dead-tab banner) as the next plan.

---

## Spec coverage check

Mapping Stage 5 spec requirements → tasks:

| Spec section | Covered by |
|---|---|
| Components — `session/mod.rs` Term integration | Task 2 (`Term` field + bytes feed + `term()` accessor) |
| Components — `render/grid.rs` (~400 LOC) | Tasks 4 (pipeline + shader) + 5 (per-frame draw + cell instances) + 6 (cursor) |
| Components — font atlas (Stage 7 covers full shaping; Stage 5 ships the simpler version) | Task 3 (`GlyphAtlas` with fontdue) |
| Data flow A — Bytes flow PTY → OscDispatcher → alacritty grid + AiStateTracker | Task 2 (`PtySession::poll` feeds Term + tracker) |
| Data flow B — Cell render at every redraw | Tasks 5–7 |
| Visual design — Default theme dark `#0e0e12` background | Stage 4 `CLEAR_COLOR` const, reused as fragment-shader bg fallback (Task 1, Task 5) |
| Color semantics — ANSI 16 + 256 + truecolor | Task 1 (`default_palette` + `resolve_color`), Task 5 (cell instances pull resolved RGB) |
| Cursor rendering | Task 6 |
| Error handling — Malformed VT sequences | `vte::ansi::Processor` handles them silently (no app-level handling needed) |

**Out of scope for Stage 5 (deferred):**
- Selection rendering — Stage 6.
- Scrollback rendering on mouse wheel — Stage 6.
- Tab bar + Notice indicator — Stage 6.
- Dead-tab banner — Stage 6.
- Cursor blink animation — Stage 6.
- Bell / visual flash — Stage 6+.
- Hyperlinks — Stage 8+.
- Image protocols (sixel / kitty) — out of v0.1.
- Cosmic-text font shaping (ligatures, emoji, fallback) — Stage 7.
- Foreground-process detection driving `set_heuristic_active(true)` — Stage 9.

## Self-review

- **Spec coverage:** every Stage 5-relevant spec requirement maps to a task. Stage 6+ items are explicitly listed as out of scope.
- **Placeholder scan:** no `TBD`/`TODO`/`implement later`/`similar to` patterns. Each step has actual code or actual commands.
- **Type consistency check:**
  - `SessionEvent::TermUpdated` (no payload) used identically in `PtySession::poll` (Task 2), `WindowApp::handle_session_event` (Task 2), and the new `poll_emits_term_updated_when_bytes_arrive` test (Task 2).
  - `App::active_term() -> Option<&Term<VoidListener>>` matches the type accepted by `Renderer::render(&mut self, term: Option<&Term<VoidListener>>)` (Task 5).
  - `CellInstance { cell: [u32; 4], fg: [f32; 4], bg: [f32; 4] }` (32 bytes) matches the WGSL `VsIn` declaration (Task 4) and the vertex-buffer attribute offsets (0, 16, 32 bytes).
  - `GridUniform { surface_size_px, cell_size_px, atlas_size_px, atlas_cells, _pad }` matches the WGSL `GridUniform` struct field-by-field (Task 4).
  - `resolve_color(Color, &Colors, fg_default, bg_default) -> Rgb` (Task 1) used identically in `build_cell_instances` (Task 5).
  - `glyph_index(c) -> Option<u32>` and `atlas.cell_pitch() -> (u32, u32)` (Task 3) used identically in `Renderer::render` (Task 5).
- **Clippy / fmt discipline:** every code-changing task ends with verify-fmt+clippy.
- **Threading-model discipline:** `Term` and `Processor` live on the main thread inside `PtySession`. The reader thread continues to send raw bytes via mpsc; the main thread consumes them in `poll`. `Renderer`, `GlyphAtlas`, and `GridPipeline` all live on the main thread per the wgpu+winit constraint. No new threads, no new locks.
- **Forward-declared item handling:** `GridPipeline` introduced in Task 4 has a temporary `#[allow(dead_code)]`-suppression hack (the `_UNUSED_BUT_REFERENCED_IN_NEXT_TASK` const) that Task 5 removes. `GlyphAtlas` (Task 3) and `default_palette`/`resolve_color` (Task 1) are `pub` so they don't need suppression even between tasks.
- **Pedagogical clarity (user is learning Rust):** the plan explains non-obvious choices inline:
  - Why `&mut self` on `App::resize_all` after Stage 4 used `&self` (Task 2 — `Term::resize` takes `&mut`)
  - The `vte::ansi::Processor` byte-by-byte advance pattern vs. the naive `Term::advance_bytes` API that doesn't exist (Task 2)
  - Why fontdue and not cosmic-text in Stage 5 (preamble; deferred to Stage 7)
  - The 6-vertex-per-instanced-quad expansion in WGSL (Task 4)
  - The fragment shader's `mix(bg, fg, alpha)` pattern using R8Unorm grayscale alpha (Task 4)
  - `bytemuck::Pod`/`Zeroable` for safe slice-to-bytes casting (Task 4)
  - The block-cursor "swap fg and bg" technique vs. a separate cursor pipeline (Task 6)
  - Why the `Line` is `i32` and we filter `< 0` to skip scrollback (Tasks 5–6)

---

## Notable plan risks

This plan is more complex than Stages 1–4 because it introduces three new conceptual layers (alacritty grid, font atlas, custom shader) plus full color decoding. The most likely failure modes during execution:

1. **Shader bugs.** WGSL is compile-checked by wgpu at runtime, so syntax errors only surface when `Renderer::new` is called. Logical bugs in vertex math (NDC conversion, UV computation) produce wrong-looking output rather than panics. Smoke checklist (Task 8) is the verification gate.

2. **`alacritty_terminal::vte::ansi::Color` re-exports.** The `Color`, `NamedColor`, `Rgb` types are re-exported from the inner `vte` crate. Importing from the wrong path (`vte::Color` instead of `alacritty_terminal::vte::ansi::Color`) produces type-mismatch errors. Plan-verbatim imports are correct.

3. **`Colors` slot indexing.** alacritty's `Colors` newtype has `Index<usize>` and `Index<NamedColor>` impls, but `Colors::default()` produces all-`None` slots. Task 1's tests assert against an empty `Colors` to prove the fallback path works.

4. **fontdue glyph positioning.** Task 3's atlas-pack code uses `metrics.xmin` and an ascent-based Y offset to place glyphs inside their cell. If glyphs appear too high, too low, or get clipped at the cell boundary, the math in Task 3 Step 2 needs adjustment. Smoke run (Task 8) reveals it.

5. **Test child stdout buffering.** Tasks 2's tests write `b'hello\n'` and rely on `sys.stdout.flush()` plus `time.sleep(2)` to keep the PTY open. If the tests are flaky, increase the sleep or assert against a longer-running process.

6. **`Term::resize` semantics.** Calling `Term::resize` with new dimensions resets some internal damage state. If after-resize rendering shows phantom characters, see alacritty's docs for `Term::reset_state()` or a damage-tracking pass.

These risks are addressed by the Sonnet pre-execution review pass that the user-Brian-Hengen pipeline does on every stage plan.
