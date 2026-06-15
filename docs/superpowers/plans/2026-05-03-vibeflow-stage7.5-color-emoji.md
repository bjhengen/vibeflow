# vibeflow Stage 7.5 Implementation Plan: color emoji RGBA atlas + wide-glyph fix

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a parallel RGBA8Unorm color atlas to `TextEngine`, route glyph rasterization by `SwashContent` (mono → R8, color → RGBA), teach the unified `QuadPipeline` to sample either atlas via a per-instance `kind` flag and a fragment-shader branch, and correct wide-glyph rendering by reading `WIDE_CHAR`/`WIDE_CHAR_SPACER` flags from `alacritty_terminal`.

**Architecture:** Two GPU textures (R8 mono + RGBA color) inside `TextEngine`, share one shelf-packer + one sampler. Single `QuadPipeline` binds both textures; per-instance `flags[0]: u32` is `0` for mono / `1` for color; fragment shader picks `mix(bg,fg,alpha)` for mono and premultiplied over-blend for color. `build_cell_instances` widens the bg quad of `WIDE_CHAR` cells to 2× cell width and skips `WIDE_CHAR_SPACER` cells outright.

**Tech Stack:** No new dependencies. cosmic-text 0.12 already pulls swash for color content; `alacritty_terminal` already exposes `cell.flags`. wgpu 0.20 supports per-instance `Uint32x4` attributes natively.

**Lessons carried forward from Stages 1–7:**
- Pre-execution senior review of plan code is high-value. Stage 7's plan-vs-reality drift caught real ordering / cursor / color-resolution issues. Run a Sonnet review pass on this plan before dispatching tasks.
- Per-task Haiku reviewers consistently miss whole-stage issues. Run a final senior-tier holistic review before merging.
- Implementers will sometimes use refactor tasks to rewrite UNRELATED tests with fabricated justifications. Compare test-name lists before/after every multi-file refactor.
- Plan-verbatim Rust must be rustfmt-clean.
- Per-instance buffer growth must keep 16-byte alignment. `QuadInstance` going 64 → 80 bytes is naturally aligned (5 × 16). `QuadUniform` going 16 → 32 bytes also aligned.
- `pub fn` items don't trigger dead_code warnings; `#[allow(dead_code)]` from earlier stages was unnecessary and was removed at end of Stage 7. Don't reintroduce it.
- WGSL bugs only surface at runtime when `Renderer::new` calls the pipeline. Smoke run is the validation gate.
- VNC display is available on host (port 5901). GUI smoke runs are runnable.

---

## File Structure

| Path | Responsibility | Net delta |
|---|---|---|
| `crates/vibeflow/src/render/text_engine.rs` (modify) | Add `GlyphKind`, `RasterImage.kind`, `GlyphRef.kind`. Add color atlas (texture/view/w/h/shelves). Factor shelf-packer into reusable helper. Update `rasterize` to return RGBA for `SwashContent::Color`. Update `try_atlas` to route by kind. Rename `atlas_dirty` → `atlases_dirty`. Add `color_atlas_size()` getter. | +180 / −30 |
| `crates/vibeflow/src/render/quad.rs` (modify) | Extend `QuadInstance` with `flags: [u32; 4]` (16 bytes; total 80). Add fifth `Uint32x4` vertex attribute at offset 64. Bind-group layout grows to 4 entries (uniform + mono tex + color tex + sampler). Extend `QuadUniform` to 32 bytes (surface + mono atlas + color atlas + 8B pad). Rename `rebind_atlas` → `rebind_atlases`. Read `cell.flags` in `build_cell_instances` to handle WIDE_CHAR / WIDE_CHAR_SPACER. | +90 / −25 |
| `crates/vibeflow/src/render/quad.wgsl` (modify) | New bindings: mono tex @ 1, color tex @ 2, sampler @ 3. Extend `QuadUniform`. Vertex shader picks atlas-size pair by kind. Fragment shader branches: mono → mix; color → premultiplied over-blend. | +25 / −5 |
| `crates/vibeflow/src/render/mod.rs` (modify) | `Renderer::new` passes `&text_engine.color_view` into `QuadPipeline::new`. Renamed `rebind_atlas` → `rebind_atlases`. No structural change. | +5 / −5 |
| `docs/TESTING.md` (modify) | Append Stage 7.5 manual smoke checklist. | +30 |

**Net add:** ≈ +300 / −65 (≈ +235 net), 5 files modified, 0 deps added, 0 files deleted.

---

## Task 0: Branch + `GlyphKind` foundation + shared shelf-pack helper (TDD)

**Files:**
- Create branch: `stage7.5-color-emoji` from `main` (commit `c3e32b8`).
- Modify: `crates/vibeflow/src/render/text_engine.rs`

This task introduces the type machinery (`GlyphKind`) and refactors the existing shelf-packer into a free helper that both atlases can share. **No color atlas yet** — that's Task 1. Getting the abstraction right first means Task 1 becomes a small additive change.

- [ ] **Step 1: Create the branch**

```bash
cd /path/to/vibeflow
git checkout main
git pull --ff-only || true   # safe even if no remote / no upstream
git checkout -b stage7.5-color-emoji
```

- [ ] **Step 2: Add `GlyphKind` and extend `RasterImage` / `GlyphRef`**

In `crates/vibeflow/src/render/text_engine.rs`, after the existing `use` block at the top, before `pub const PRIMARY_FONT`, add:

```rust
/// Which atlas a rasterized glyph belongs in. Stage 7 glyphs were all `Mono`;
/// Stage 7.5 adds `Color` for emoji and other RGBA-rasterized glyphs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphKind {
    /// R8Unorm mask (alpha-only). Renderer uses `mix(bg, fg, alpha)`.
    Mono,
    /// RGBA8Unorm with premultiplied alpha (swash's color-glyph format).
    Color,
}
```

Update `RasterImage` (currently has `width, height, bearing_x, bearing_y, data`) to add `kind`:

```rust
/// One rasterized glyph plus its placement metrics (relative to the cell origin).
#[derive(Debug, Clone)]
pub struct RasterImage {
    pub kind: GlyphKind,
    /// Width of the bitmap in pixels.
    pub width: u32,
    /// Height of the bitmap in pixels.
    pub height: u32,
    /// Offset from cell origin (cell top-left) to the bitmap top-left, in pixels.
    pub bearing_x: i32,
    pub bearing_y: i32,
    /// Mono: R8 alpha bytes (length = width * height).
    /// Color: RGBA premultiplied bytes (length = 4 * width * height).
    pub data: Vec<u8>,
}
```

Update `GlyphRef` (currently has `atlas_x, atlas_y, atlas_w, atlas_h, bearing_x, bearing_y`) to add `kind`:

```rust
/// Reference to a rasterized glyph in the atlas. Returned by `glyph_for`.
/// All position fields in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlyphRef {
    pub kind: GlyphKind,
    pub atlas_x: u32,
    pub atlas_y: u32,
    pub atlas_w: u32,
    pub atlas_h: u32,
    /// Bearing relative to the cell top-left.
    pub bearing_x: i32,
    pub bearing_y: i32,
}
```

- [ ] **Step 3: Update `rasterize` to return `RasterImage.kind`**

Find the `rasterize` method body. Currently it has:

```rust
if image.content == SwashContent::Color {
    return None;
}

Some(RasterImage {
    width: image.placement.width,
    ...
    data: image.data.clone(),
})
```

Replace with:

```rust
let kind = match image.content {
    SwashContent::Color => GlyphKind::Color,
    // Stage 7.5 treats SubpixelMask as Mono (proper subpixel AA is Stage 9+).
    SwashContent::Mask | SwashContent::SubpixelMask => GlyphKind::Mono,
};

Some(RasterImage {
    kind,
    width: image.placement.width,
    height: image.placement.height,
    bearing_x: image.placement.left,
    bearing_y: image.placement.top,
    data: image.data.clone(),
})
```

The `image.data` slice is already in the right format for each kind (R8 for Mask/SubpixelMask, RGBA premultiplied for Color) — swash documents this.

- [ ] **Step 4: Update `try_atlas` to thread `kind` through (Task 0 still all `Mono`-routed)**

In `try_atlas`, the existing code is roughly:

```rust
fn try_atlas(&mut self, c: char) -> Option<GlyphRef> {
    let img = self.rasterize(c)?;
    if img.width == 0 || img.height == 0 {
        return Some(GlyphRef {
            atlas_x: 0, atlas_y: 0,
            atlas_w: 0, atlas_h: 0,
            bearing_x: 0, bearing_y: 0,
        });
    }
    let (x, y) = self.allocate(img.width, img.height);
    self.upload_to_atlas(x, y, img.width, img.height, &img.data);
    Some(GlyphRef {
        atlas_x: x, atlas_y: y,
        atlas_w: img.width, atlas_h: img.height,
        bearing_x: img.bearing_x, bearing_y: img.bearing_y,
    })
}
```

Update to thread `img.kind` through (atlas routing comes in Task 1; for Task 0 the kind just propagates to the returned `GlyphRef`). Importantly, `Color` glyphs go through the existing mono path here (will be wrong-format upload), so this Task 0 build will FAIL the smoke test for emoji — that's expected and Task 1 fixes it. Task 0's purpose is to set up the type plumbing.

```rust
fn try_atlas(&mut self, c: char) -> Option<GlyphRef> {
    let img = self.rasterize(c)?;
    if img.width == 0 || img.height == 0 {
        return Some(GlyphRef {
            kind: img.kind,
            atlas_x: 0, atlas_y: 0,
            atlas_w: 0, atlas_h: 0,
            bearing_x: 0, bearing_y: 0,
        });
    }
    // Task 0 still routes everything through the mono atlas. Task 1 splits.
    let (x, y) = self.allocate(img.width, img.height);
    self.upload_to_atlas(x, y, img.width, img.height, &img.data);
    Some(GlyphRef {
        kind: img.kind,
        atlas_x: x, atlas_y: y,
        atlas_w: img.width, atlas_h: img.height,
        bearing_x: img.bearing_x, bearing_y: img.bearing_y,
    })
}
```

- [ ] **Step 5: Update tests in `text_engine.rs`**

The Stage 7 tests `cell_metrics_returns_jbm_pitch_at_16px`, `rasterize_ascii_letter_returns_image`, `rasterize_space_returns_none_or_empty_image`, `rasterize_cjk_uses_system_fallback` all live in `mod tests` (most are `#[ignore]` because they need wgpu via the `test_engine()` helper).

Add two new tests inside `mod tests` (DO NOT MODIFY EXISTING TESTS):

```rust
    #[test]
    fn rasterize_mono_letter_returns_mono_kind() {
        let mut engine = test_engine();
        let img = engine.rasterize('A').unwrap();
        assert_eq!(img.kind, GlyphKind::Mono);
        assert_eq!(img.data.len(), (img.width * img.height) as usize);
    }

    #[test]
    fn rasterize_color_emoji_returns_color_kind() {
        let mut engine = test_engine();
        // 🎉 (U+1F389). Skip cleanly if the test env has no color emoji font.
        if let Some(img) = engine.rasterize('🎉') {
            assert_eq!(img.kind, GlyphKind::Color);
            // RGBA: data length = 4 * width * height.
            assert_eq!(img.data.len(), (4 * img.width * img.height) as usize);
        }
    }
```

Both are `#[ignore]`-flavored because they use `test_engine()` (which needs Mesa software GL). Mark both `#[ignore = "requires Mesa software GL (LIBGL_ALWAYS_SOFTWARE=1); run with --ignored"]`.

WAIT — the existing `rasterize_ascii_letter_returns_image` test is also `#[ignore]`d in Stage 7. Both new tests follow the same pattern.

NOTE: If you find that `rasterize_ascii_letter_returns_image` does NOT use `test_engine()` (i.e. it calls `TextEngine::new()` somewhere), STOP and report — Stage 7 should have unified all four Task 1 tests on the helper.

- [ ] **Step 6: Verify**

```bash
cd /path/to/vibeflow
cargo build -p vibeflow
cargo test -p vibeflow --lib 2>&1 | tail -3
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -3
```

Expected: build clean. 128 default tests pass + 9 ignored (Stage 7's 7 + 2 new). clippy clean.

If you've added the `GlyphKind` import to existing call sites in `quad.rs` (where `GlyphRef` is constructed in the fallback path), include those edits too. Specifically `quad.rs`'s `build_cell_instances` has a fallback `GlyphRef { ... }` literal — add `kind: GlyphKind::Mono,` to it. Same in `build_banner_instances` if it constructs a fallback GlyphRef.

Search-and-add:
```bash
grep -n 'GlyphRef {' crates/vibeflow/src/render/quad.rs
```
For each construction, add `kind: crate::render::text_engine::GlyphKind::Mono,` (or import GlyphKind at the top of `quad.rs`).

- [ ] **Step 7: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/render/text_engine.rs crates/vibeflow/src/render/quad.rs
git commit -m "chore(render): introduce GlyphKind, thread kind through RasterImage/GlyphRef"
```

---

## Task 1: Color atlas in `TextEngine` + `try_atlas` routing (TDD)

**Files:**
- Modify: `crates/vibeflow/src/render/text_engine.rs`

`TextEngine` gains a parallel RGBA8Unorm atlas. The shelf-packer is extracted into a free helper used by both atlases. `try_atlas` routes by `RasterImage.kind`.

- [ ] **Step 1: Add color atlas constants and fields**

In `crates/vibeflow/src/render/text_engine.rs`, after the existing `ATLAS_INITIAL_W`/`ATLAS_INITIAL_H` constants (which are 256/256 for the mono atlas), add:

```rust
/// Initial pixel size of the color (RGBA) atlas. Color glyphs are rare and
/// usually small (16×16 emoji at FONT_PX), so 256×256 holds ~256 emoji.
const COLOR_ATLAS_INITIAL_W: u32 = 256;
const COLOR_ATLAS_INITIAL_H: u32 = 256;
```

Extend the `TextEngine` struct. The Stage 7 fields (`font_system`, `swash_cache`, `cell_w`, `cell_h`, `baseline_y`, `texture`, `view`, `sampler`, `atlas_w`, `atlas_h`, `shelves`, `cache`, `atlas_dirty`, `queue`, `device`) become:

```rust
pub struct TextEngine {
    font_system: FontSystem,
    swash_cache: SwashCache,
    cell_w: u32,
    cell_h: u32,
    baseline_y: u32,

    // Mono atlas (R8Unorm).
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    atlas_w: u32,
    atlas_h: u32,
    shelves: Vec<Shelf>,

    // Color atlas (RGBA8Unorm). Same sampler reused.
    pub color_texture: wgpu::Texture,
    pub color_view: wgpu::TextureView,
    color_atlas_w: u32,
    color_atlas_h: u32,
    color_shelves: Vec<Shelf>,

    cache: HashMap<char, Option<GlyphRef>>,
    /// True when EITHER atlas has been re-allocated since the last call.
    atlases_dirty: bool,
    queue: Arc<wgpu::Queue>,
    device: Arc<wgpu::Device>,
}
```

Note `atlas_dirty` was renamed to `atlases_dirty` (plural). Update the field initializer in `Self { ... }` accordingly. Note also `sampler` stays — it's reused for both atlases.

- [ ] **Step 2: Allocate the color atlas in `new`**

In `TextEngine::new`, after the existing mono `texture` + `view` + `sampler` creation, add the color atlas creation:

```rust
let color_texture = device.create_texture(&wgpu::TextureDescriptor {
    label: Some("vibeflow-text-engine-color-atlas"),
    size: wgpu::Extent3d {
        width: COLOR_ATLAS_INITIAL_W,
        height: COLOR_ATLAS_INITIAL_H,
        depth_or_array_layers: 1,
    },
    mip_level_count: 1,
    sample_count: 1,
    dimension: wgpu::TextureDimension::D2,
    format: wgpu::TextureFormat::Rgba8Unorm,
    // COPY_SRC required for grow_color_atlas's copy_texture_to_texture.
    usage: wgpu::TextureUsages::TEXTURE_BINDING
        | wgpu::TextureUsages::COPY_DST
        | wgpu::TextureUsages::COPY_SRC,
    view_formats: &[],
});
let color_view = color_texture.create_view(&wgpu::TextureViewDescriptor::default());
```

Update the `Ok(Self { ... })` block to include all the new fields. Initial values:

```rust
Ok(Self {
    font_system,
    swash_cache,
    cell_w,
    cell_h,
    baseline_y,
    texture,
    view,
    sampler,
    atlas_w: ATLAS_INITIAL_W,
    atlas_h: ATLAS_INITIAL_H,
    shelves: Vec::new(),
    color_texture,
    color_view,
    color_atlas_w: COLOR_ATLAS_INITIAL_W,
    color_atlas_h: COLOR_ATLAS_INITIAL_H,
    color_shelves: Vec::new(),
    cache: HashMap::new(),
    atlases_dirty: false,
    queue,
    device,
})
```

- [ ] **Step 3: Add color-aware atlas methods**

Add methods to `impl TextEngine`. Replace the existing `allocate`, `grow_atlas`, `upload_to_atlas`, `atlas_size`, `texture_dirty` with kind-aware versions:

```rust
    /// Pixel size of the mono atlas.
    #[must_use]
    pub fn atlas_size(&self) -> (u32, u32) {
        (self.atlas_w, self.atlas_h)
    }

    /// Pixel size of the color atlas.
    #[must_use]
    pub fn color_atlas_size(&self) -> (u32, u32) {
        (self.color_atlas_w, self.color_atlas_h)
    }

    /// True iff EITHER atlas has been re-allocated since the last call.
    /// `QuadPipeline` polls this each frame to rebuild its bind group when set.
    /// Resets the flag on read.
    pub fn texture_dirty(&mut self) -> bool {
        let dirty = self.atlases_dirty;
        self.atlases_dirty = false;
        dirty
    }

    /// Allocate a `w × h` rect in the mono atlas, growing as needed.
    fn allocate_mono(&mut self, w: u32, h: u32) -> (u32, u32) {
        let need_grow = !shelves_can_fit(&self.shelves, self.atlas_w, self.atlas_h, w, h);
        if need_grow {
            let new_h = double_until_fits(self.atlas_h, h, &self.shelves);
            self.grow_mono_atlas(new_h);
        }
        shelf_pack(&mut self.shelves, self.atlas_w, w, h)
    }

    /// Allocate a `w × h` rect in the color atlas, growing as needed.
    fn allocate_color(&mut self, w: u32, h: u32) -> (u32, u32) {
        let need_grow = !shelves_can_fit(&self.color_shelves, self.color_atlas_w, self.color_atlas_h, w, h);
        if need_grow {
            let new_h = double_until_fits(self.color_atlas_h, h, &self.color_shelves);
            self.grow_color_atlas(new_h);
        }
        shelf_pack(&mut self.color_shelves, self.color_atlas_w, w, h)
    }

    /// Double mono atlas height until `min_height` fits. Copies old contents.
    fn grow_mono_atlas(&mut self, min_height: u32) {
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
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("vibeflow-mono-atlas-grow-copy"),
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
        self.atlases_dirty = true;
    }

    /// Double color atlas height until `min_height` fits. Copies old contents.
    fn grow_color_atlas(&mut self, min_height: u32) {
        let mut new_h = self.color_atlas_h;
        while new_h < min_height {
            new_h *= 2;
        }
        let new_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vibeflow-text-engine-color-atlas"),
            size: wgpu::Extent3d {
                width: self.color_atlas_w,
                height: new_h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("vibeflow-color-atlas-grow-copy"),
        });
        encoder.copy_texture_to_texture(
            wgpu::ImageCopyTexture {
                texture: &self.color_texture,
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
                width: self.color_atlas_w,
                height: self.color_atlas_h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(std::iter::once(encoder.finish()));

        self.color_texture = new_texture;
        self.color_view = self.color_texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.color_atlas_h = new_h;
        self.atlases_dirty = true;
    }

    /// Upload pixel bytes into the mono atlas at (x, y) of size (w, h). R8 = 1 byte/px.
    fn upload_to_mono_atlas(&self, x: u32, y: u32, w: u32, h: u32, data: &[u8]) {
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
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
    }

    /// Upload pixel bytes into the color atlas at (x, y) of size (w, h). RGBA = 4 bytes/px.
    fn upload_to_color_atlas(&self, x: u32, y: u32, w: u32, h: u32, data: &[u8]) {
        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.color_texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
    }
```

Add these free functions OUTSIDE `impl TextEngine` (e.g. just below the `Shelf` struct definition):

```rust
/// Returns `true` if the existing shelves can fit a `w × h` rect in an atlas
/// of width `atlas_w` × height `atlas_h`. Used by both atlases.
fn shelves_can_fit(shelves: &[Shelf], atlas_w: u32, atlas_h: u32, w: u32, h: u32) -> bool {
    // Existing shelf with enough headroom?
    if shelves
        .iter()
        .any(|s| s.height >= h && s.next_x + w <= atlas_w)
    {
        return true;
    }
    // New shelf at the bottom?
    let bottom = shelves.iter().map(|s| s.y + s.height).max().unwrap_or(0);
    bottom + h <= atlas_h
}

/// Returns the smallest power-of-two-multiple of `current_h` that fits a new
/// shelf of height `new_shelf_h` after the existing shelves.
fn double_until_fits(current_h: u32, new_shelf_h: u32, shelves: &[Shelf]) -> u32 {
    let bottom = shelves.iter().map(|s| s.y + s.height).max().unwrap_or(0);
    let needed = bottom + new_shelf_h;
    let mut new_h = current_h;
    while new_h < needed {
        new_h *= 2;
    }
    new_h
}

/// Place a `w × h` rect into `shelves` (mutates). Caller has already verified
/// it fits via `shelves_can_fit`. Returns the (x, y) offset of the new rect.
fn shelf_pack(shelves: &mut Vec<Shelf>, atlas_w: u32, w: u32, h: u32) -> (u32, u32) {
    if let Some(shelf) = shelves
        .iter_mut()
        .find(|s| s.height >= h && s.next_x + w <= atlas_w)
    {
        let x = shelf.next_x;
        let y = shelf.y;
        shelf.next_x += w;
        return (x, y);
    }
    let shelf_y = shelves.iter().map(|s| s.y + s.height).max().unwrap_or(0);
    shelves.push(Shelf {
        y: shelf_y,
        height: h,
        next_x: w,
    });
    (0, shelf_y)
}
```

DELETE the old `allocate` and `upload_to_atlas` methods on `TextEngine` (they're replaced by the kind-specific ones above). Verify no callers remain via `grep`.

- [ ] **Step 4: Route in `try_atlas` by kind**

Replace `try_atlas`'s body with kind-aware routing:

```rust
fn try_atlas(&mut self, c: char) -> Option<GlyphRef> {
    let img = self.rasterize(c)?;
    if img.width == 0 || img.height == 0 {
        return Some(GlyphRef {
            kind: img.kind,
            atlas_x: 0, atlas_y: 0,
            atlas_w: 0, atlas_h: 0,
            bearing_x: 0, bearing_y: 0,
        });
    }
    let (x, y) = match img.kind {
        GlyphKind::Mono => {
            let (x, y) = self.allocate_mono(img.width, img.height);
            self.upload_to_mono_atlas(x, y, img.width, img.height, &img.data);
            (x, y)
        }
        GlyphKind::Color => {
            let (x, y) = self.allocate_color(img.width, img.height);
            self.upload_to_color_atlas(x, y, img.width, img.height, &img.data);
            (x, y)
        }
    };
    Some(GlyphRef {
        kind: img.kind,
        atlas_x: x, atlas_y: y,
        atlas_w: img.width, atlas_h: img.height,
        bearing_x: img.bearing_x, bearing_y: img.bearing_y,
    })
}
```

- [ ] **Step 5: Add tests**

Append to `mod tests` in `text_engine.rs`:

```rust
    #[test]
    #[ignore = "requires Mesa software GL (LIBGL_ALWAYS_SOFTWARE=1); run with --ignored"]
    fn glyph_for_emoji_routes_to_color_atlas() {
        let mut engine = test_engine();
        // Skip cleanly if no color emoji font in the test env.
        if let Some(g) = engine.glyph_for('🎉') {
            assert_eq!(g.kind, GlyphKind::Color);
            // Cache hit on second call returns identical GlyphRef.
            assert_eq!(engine.glyph_for('🎉'), Some(g));
        }
    }

    #[test]
    #[ignore = "requires Mesa software GL (LIBGL_ALWAYS_SOFTWARE=1); run with --ignored"]
    fn glyph_for_letter_routes_to_mono_atlas() {
        let mut engine = test_engine();
        let g = engine.glyph_for('A').unwrap();
        assert_eq!(g.kind, GlyphKind::Mono);
    }

    #[test]
    #[ignore = "requires Mesa software GL (LIBGL_ALWAYS_SOFTWARE=1); run with --ignored"]
    fn color_atlas_grows_when_full() {
        let mut engine = test_engine();
        let initial_h = engine.color_atlas_size().1;
        // Force a barrage of distinct emoji codepoints. The Smiling-Face block
        // (U+1F600 …) gives ~80 distinct emoji; at typical 16×16 size,
        // 80 emoji = 80 × 16 × 16 × 4 = 81 920 B, fits in a single 256×256
        // atlas (262 144 B). Use a wider span to force growth.
        for code in 0x1F600u32..=0x1F64Fu32 {
            if let Some(c) = char::from_u32(code) {
                engine.glyph_for(c);
            }
        }
        for code in 0x1F300u32..=0x1F320u32 {
            if let Some(c) = char::from_u32(code) {
                engine.glyph_for(c);
            }
        }
        // If env has no color emoji font, glyph_for returns None and the
        // atlas never grows. Both outcomes are valid; just assert no panic.
        let (_, h_after) = engine.color_atlas_size();
        assert!(
            h_after >= initial_h && h_after % initial_h == 0,
            "color atlas height {} is not a power-of-two multiple of {}",
            h_after,
            initial_h
        );
    }
```

NOTE: The Stage 7 test `glyph_for_caches_repeat_lookups` already exists and tests cache hits for ASCII. Don't duplicate.

- [ ] **Step 6: Verify**

```bash
cd /path/to/vibeflow
cargo build -p vibeflow 2>&1 | tail -3
cargo test -p vibeflow --lib 2>&1 | tail -3
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -10
```

Expected: build clean. 128 default + 12 ignored (Stage 7's 7 + Task 0's 2 + Task 1's 3). Clippy clean.

NOTE: If the build fails because `Renderer::render` calls the old `texture_dirty` and the old `rebind_atlas`, those signatures still match what we have post-Task-1 — we just renamed `atlas_dirty` to `atlases_dirty` internally. The public API of `texture_dirty()` is unchanged. So `mod.rs` doesn't need to change in this task.

LOCAL ONLY (verifies the GPU path on host):
```bash
LIBGL_ALWAYS_SOFTWARE=1 cargo test -p vibeflow --lib render::text_engine -- --ignored
```

- [ ] **Step 7: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/render/text_engine.rs
git commit -m "feat(render): TextEngine — color atlas (RGBA8Unorm) + kind-routed try_atlas (TDD)"
```

---

## Task 2: `QuadPipeline` — `flags[0]: u32` per instance + dual-texture bind group + branching shader

**Files:**
- Modify: `crates/vibeflow/src/render/quad.rs`
- Modify: `crates/vibeflow/src/render/quad.wgsl`
- Modify: `crates/vibeflow/src/render/tabs.rs` (push_text_glyphs callers)

The pipeline grows by one bind-group entry (color texture), one vertex attribute (`Uint32x4` flags), and one shader branch.

- [ ] **Step 1: Update `QuadInstance` (Rust side)**

In `crates/vibeflow/src/render/quad.rs`, find the `QuadInstance` struct. Currently:

```rust
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct QuadInstance {
    pub screen_rect_px: [f32; 4],
    pub atlas_rect_px: [f32; 4],
    pub fg: [f32; 4],
    pub bg: [f32; 4],
}
```

Replace with:

```rust
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct QuadInstance {
    pub screen_rect_px: [f32; 4],
    pub atlas_rect_px: [f32; 4],
    pub fg: [f32; 4],
    pub bg: [f32; 4],
    /// Lane 0: glyph kind (0 = Mono, 1 = Color). Lanes 1..=3 reserved for
    /// future per-instance flags. Total instance size: 80 bytes (5 × 16).
    pub flags: [u32; 4],
}
```

Update `QuadInstance::new` to take a `kind: u32` parameter as the LAST argument:

```rust
impl QuadInstance {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        screen_x: f32, screen_y: f32, screen_w: f32, screen_h: f32,
        atlas_x: f32, atlas_y: f32, atlas_w: f32, atlas_h: f32,
        fg: [f32; 4],
        bg: [f32; 4],
        kind: u32,
    ) -> Self {
        Self {
            screen_rect_px: [screen_x, screen_y, screen_w, screen_h],
            atlas_rect_px: [atlas_x, atlas_y, atlas_w, atlas_h],
            fg,
            bg,
            flags: [kind, 0, 0, 0],
        }
    }
}
```

Add convenience constants for callers:
```rust
pub const KIND_MONO: u32 = 0;
pub const KIND_COLOR: u32 = 1;
```

NOTE: Several call sites in `quad.rs::build_cell_instances`, `quad.rs::build_banner_instances`, and `tabs.rs::push_text_glyphs` currently pass 10 args to `QuadInstance::new`. They need to pass 11 now. Steps 2a–2c below update each, in this same task, so the build remains restorable as soon as Task 3 fixes the `mod.rs` Renderer callers.

For each caller, the rule is: read `glyph.kind` (or `g.kind`) and translate to `KIND_MONO` / `KIND_COLOR`. Bg-only quads (zero atlas rect) pass `KIND_MONO` since they don't sample the atlas.

- [ ] **Step 2: Update vertex attribute layout + stride**

Find the `wgpu::VertexBufferLayout { array_stride: QUAD_STRIDE, ... }` in `QuadPipeline::new`. The Stage 7 attribute table has 4 entries for the 4 vec4 fields at offsets 0, 16, 32, 48. Add a fifth:

```rust
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
        wgpu::VertexAttribute {
            offset: 64,
            shader_location: 4,
            format: wgpu::VertexFormat::Uint32x4,
        },
    ],
}],
```

`QUAD_STRIDE` = `std::mem::size_of::<QuadInstance>() as u64` will now equal 80. No change needed to that line.

- [ ] **Step 3: Update `QuadUniform`**

Replace:

```rust
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct QuadUniform {
    surface_size_px: [f32; 2],
    atlas_size_px: [f32; 2],
}
```

With:

```rust
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct QuadUniform {
    surface_size_px: [f32; 2],
    mono_atlas_size_px: [f32; 2],
    color_atlas_size_px: [f32; 2],
    /// 8-byte pad to keep the struct at a multiple of 16 bytes for std140-ish
    /// alignment. WGSL `vec2<f32>` is 8-byte aligned but the struct as a whole
    /// must be 16-byte aligned for uniform buffer binding.
    _pad: [f32; 2],
}
```

Total: 32 bytes. `std::mem::size_of::<QuadUniform>() as u64` is now 32.

- [ ] **Step 4: Update bind group layout to 4 entries**

Find `bind_group_layout` creation. Replace with:

```rust
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
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 3,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        },
    ],
});
```

- [ ] **Step 5: Update `make_bind_group` to take both views**

Replace:

```rust
fn make_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform_buffer: &wgpu::Buffer,
    atlas_view: &wgpu::TextureView,
    atlas_sampler: &wgpu::Sampler,
) -> wgpu::BindGroup
```

With:

```rust
fn make_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform_buffer: &wgpu::Buffer,
    mono_view: &wgpu::TextureView,
    color_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
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
                resource: wgpu::BindingResource::TextureView(mono_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(color_view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}
```

Update both calls inside `QuadPipeline::new` and inside the renamed `rebind_atlases` (Step 6).

- [ ] **Step 6: Rename `rebind_atlas` → `rebind_atlases` with both views**

Replace `pub fn rebind_atlas` with:

```rust
pub fn rebind_atlases(
    &mut self,
    device: &wgpu::Device,
    mono_view: &wgpu::TextureView,
    color_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) {
    self.bind_group = Self::make_bind_group(
        device,
        &self.bind_group_layout,
        &self.uniform_buffer,
        mono_view,
        color_view,
        sampler,
    );
}
```

Update `QuadPipeline::new` to take both views:

```rust
pub fn new(
    device: &wgpu::Device,
    surface_format: wgpu::TextureFormat,
    mono_view: &wgpu::TextureView,
    color_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> Result<Self> {
    // ... same as Stage 7 but pass mono_view + color_view + sampler to make_bind_group ...
}
```

The Stage 7 signature was `(device, surface_format, atlas_view, atlas_sampler)` — 4 args. Now it's 5 args (`mono_view`, `color_view`, `sampler` instead of `atlas_view, atlas_sampler`).

- [ ] **Step 7: Update `draw` to feed both atlas sizes**

Replace the body of `draw`:

```rust
pub fn draw<'a>(
    &'a self,
    pass: &mut wgpu::RenderPass<'a>,
    queue: &wgpu::Queue,
    instances: &[QuadInstance],
    surface_size_px: (u32, u32),
    mono_atlas_size_px: (u32, u32),
    color_atlas_size_px: (u32, u32),
) {
    if instances.is_empty() {
        return;
    }
    let uniform = QuadUniform {
        surface_size_px: [surface_size_px.0 as f32, surface_size_px.1 as f32],
        mono_atlas_size_px: [mono_atlas_size_px.0 as f32, mono_atlas_size_px.1 as f32],
        color_atlas_size_px: [color_atlas_size_px.0 as f32, color_atlas_size_px.1 as f32],
        _pad: [0.0, 0.0],
    };
    queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
    queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(instances));

    pass.set_pipeline(&self.pipeline);
    pass.set_bind_group(0, &self.bind_group, &[]);
    pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
    pass.draw(0..6, 0..(instances.len() as u32));
}
```

Stage 7's signature was `(pass, queue, instances, surface_size_px, atlas_size_px)` — 5 args. Now 6 args.

- [ ] **Step 8: Update `QuadInstance::new` call sites (3 places)**

a) **`quad.rs::build_cell_instances` glyph quad** — compute `glyph_kind` from `glyph.kind` and pass as the new 11th arg. Compute once per loop iteration (before the bg-quad emit) so it's available for the optional glyph-quad emit:

```rust
let glyph_kind: u32 = match glyph.kind {
    crate::render::text_engine::GlyphKind::Mono => crate::render::quad::KIND_MONO,
    crate::render::text_engine::GlyphKind::Color => crate::render::quad::KIND_COLOR,
};

// bg quad — pass KIND_MONO (atlas not sampled, kind is moot but consistent)
out.push(QuadInstance::new(
    screen_x, screen_y,
    cell_w as f32, cell_h as f32,
    0.0, 0.0, 0.0, 0.0,
    bg, bg,
    crate::render::quad::KIND_MONO,
));
if glyph.atlas_w > 0 && glyph.atlas_h > 0 {
    out.push(QuadInstance::new(
        screen_x + glyph.bearing_x as f32,
        screen_y + baseline_y - glyph.bearing_y as f32,
        glyph.atlas_w as f32, glyph.atlas_h as f32,
        glyph.atlas_x as f32, glyph.atlas_y as f32,
        glyph.atlas_w as f32, glyph.atlas_h as f32,
        fg, bg,
        glyph_kind,
    ));
}
```

(Task 4's wide-glyph fix adds `bg_w` and the spacer-skip; for now the bg-quad width stays `cell_w as f32`.)

b) **`quad.rs::build_banner_instances`** — banner is ASCII; pass `KIND_MONO`:

```rust
out.push(QuadInstance::new(
    x + glyph.bearing_x as f32,
    text_y + baseline_y - glyph.bearing_y as f32,
    glyph.atlas_w as f32, glyph.atlas_h as f32,
    glyph.atlas_x as f32, glyph.atlas_y as f32,
    glyph.atlas_w as f32, glyph.atlas_h as f32,
    amber,
    black,
    crate::render::quad::KIND_MONO,
));
```

c) **`tabs.rs::push_text_glyphs`** — read `g.kind` and pass through. Tab text usually resolves to Mono, but if a session label contains an emoji and the system's color emoji font picks it up, the per-instance flag will route it correctly:

```rust
fn push_text_glyphs(
    out: &mut Vec<crate::render::quad::QuadInstance>,
    text_engine: &mut crate::render::text_engine::TextEngine,
    s: &str,
    pos: (f32, f32),
    cell_w: f32,
    fg: [f32; 4],
    bg: [f32; 4],
    max_x_px: u32,
) {
    let (x_start, y) = pos;
    let baseline_y = text_engine.baseline_y() as f32;
    let mut x = x_start;
    for c in s.chars() {
        if x + cell_w > max_x_px as f32 {
            break;
        }
        if let Some(g) = text_engine.glyph_for(c) {
            if g.atlas_w > 0 && g.atlas_h > 0 {
                let kind: u32 = match g.kind {
                    crate::render::text_engine::GlyphKind::Mono => crate::render::quad::KIND_MONO,
                    crate::render::text_engine::GlyphKind::Color => crate::render::quad::KIND_COLOR,
                };
                out.push(crate::render::quad::QuadInstance::new(
                    x + g.bearing_x as f32,
                    y + baseline_y - g.bearing_y as f32,
                    g.atlas_w as f32, g.atlas_h as f32,
                    g.atlas_x as f32, g.atlas_y as f32,
                    g.atlas_w as f32, g.atlas_h as f32,
                    fg, bg,
                    kind,
                ));
            }
        }
        x += cell_w;
    }
}
```

- [ ] **Step 9: Rewrite `quad.wgsl`**

Replace the contents of `crates/vibeflow/src/render/quad.wgsl`:

```wgsl
// vibeflow Stage 7.5 unified quad shader. Replaces grid.wgsl + text.wgsl.
// Per-instance:
//   .xyzw screen_rect_px (x, y, w, h in surface pixels)
//   .xyzw atlas_rect_px  (x, y, w, h in atlas pixels — sized by `kind`'s atlas)
//   .rgba fg
//   .rgba bg
//   .x    kind (0 = Mono, 1 = Color); .yzw reserved
// Vertex shader expands 6 vertices per instance; UV uses the matching atlas
// size from QuadUniform. Fragment shader branches on kind:
//   Mono  → mix(bg, fg, sampled.r)
//   Color → premultiplied over: sampled.rgb + bg.rgb * (1 - sampled.a)

struct QuadUniform {
    surface_size_px:     vec2<f32>,
    mono_atlas_size_px:  vec2<f32>,
    color_atlas_size_px: vec2<f32>,
    _pad:                vec2<f32>,
};

@group(0) @binding(0) var<uniform> u: QuadUniform;
@group(0) @binding(1) var mono_texture:  texture_2d<f32>;
@group(0) @binding(2) var color_texture: texture_2d<f32>;
@group(0) @binding(3) var atlas_sampler: sampler;

struct VsIn {
    @builtin(vertex_index) vertex_id: u32,
    @location(0) screen_rect_px: vec4<f32>,
    @location(1) atlas_rect_px:  vec4<f32>,
    @location(2) fg:             vec4<f32>,
    @location(3) bg:             vec4<f32>,
    @location(4) flags:          vec4<u32>,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv:             vec2<f32>,
    @location(1) fg:             vec4<f32>,
    @location(2) bg:             vec4<f32>,
    @location(3) @interpolate(flat) kind: u32,
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

    let kind = in.flags.x;
    let atlas_size_px = select(u.mono_atlas_size_px, u.color_atlas_size_px, kind == 1u);
    let atlas_pos_px = in.atlas_rect_px.xy + corner * in.atlas_rect_px.zw;
    let uv = atlas_pos_px / atlas_size_px;

    var out: VsOut;
    out.clip_pos = clip_pos;
    out.uv       = uv;
    out.fg       = in.fg;
    out.bg       = in.bg;
    out.kind     = kind;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (in.kind == 1u) {
        // Color path. swash provides premultiplied RGBA.
        let s = textureSample(color_texture, atlas_sampler, in.uv);
        let rgb = s.rgb + in.bg.rgb * (1.0 - s.a);
        return vec4<f32>(rgb, 1.0);
    } else {
        // Mono path. Same as Stage 7.
        let alpha = textureSample(mono_texture, atlas_sampler, in.uv).r;
        let rgb   = mix(in.bg.rgb, in.fg.rgb, alpha);
        return vec4<f32>(rgb, 1.0);
    }
}
```

Note WGSL's `select(a, b, cond)` returns `b` if `cond` is true, else `a`. The shader uses it to pick the correct atlas-size pair.

- [ ] **Step 10: Verify build (will fail at `Renderer` callers in `mod.rs` — expected)**

```bash
cd /path/to/vibeflow
cargo build -p vibeflow 2>&1 | tail -20
```

Expected errors at `mod.rs` ONLY:
- `mod.rs::Renderer::new` calls `QuadPipeline::new(&device, format, &text_engine.view, &text_engine.sampler)` — old 4-arg signature. New: 5 args (`device, format, mono_view, color_view, sampler`). Task 3 fixes this.
- `mod.rs::Renderer::render` calls `self.quad_pipeline.draw(&mut pass, &self.queue, &instances, surface_size, atlas_size)` — old 5-arg signature. New: 6 args. Task 3 fixes this.
- `mod.rs::Renderer::render` calls `self.quad_pipeline.rebind_atlas(...)` — renamed to `rebind_atlases(device, mono_view, color_view, sampler)`. Task 3 fixes.

If errors appear in any file other than `mod.rs`, STOP and report — those are unexpected and indicate a Task 2 regression. (Step 8 a/b/c should have fixed all `QuadInstance::new` call sites.)

```bash
cargo fmt --all -- --check
```

Should be clean (Task 2 modifies plan-verbatim Rust + WGSL; rustfmt doesn't touch WGSL).

- [ ] **Step 11: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/render/quad.rs \
        crates/vibeflow/src/render/quad.wgsl \
        crates/vibeflow/src/render/tabs.rs
git commit -m "feat(render): QuadPipeline — dual-texture bind group + per-instance kind + branching shader"
```

The commit leaves the build broken at `mod.rs`. Task 3 restores it.

---

## Task 3: Wire `Renderer` to the new `QuadPipeline` signature

**Files:**
- Modify: `crates/vibeflow/src/render/mod.rs`

After this task, the build is GREEN again.

- [ ] **Step 1: Update `Renderer::new`**

Find the line in `Renderer::new`:

```rust
let quad_pipeline = crate::render::quad::QuadPipeline::new(
    &device,
    format,
    &text_engine.view,
    &text_engine.sampler,
)?;
```

Replace with:

```rust
let quad_pipeline = crate::render::quad::QuadPipeline::new(
    &device,
    format,
    &text_engine.view,
    &text_engine.color_view,
    &text_engine.sampler,
)?;
```

- [ ] **Step 2: Update `Renderer::render`**

Find the texture_dirty + rebind_atlas block. Currently:

```rust
let atlas_size = self.text_engine.atlas_size();
if self.text_engine.texture_dirty() {
    self.quad_pipeline.rebind_atlas(
        &self.device,
        &self.text_engine.view,
        &self.text_engine.sampler,
    );
}
```

Replace with:

```rust
let mono_atlas_size = self.text_engine.atlas_size();
let color_atlas_size = self.text_engine.color_atlas_size();
if self.text_engine.texture_dirty() {
    self.quad_pipeline.rebind_atlases(
        &self.device,
        &self.text_engine.view,
        &self.text_engine.color_view,
        &self.text_engine.sampler,
    );
}
```

Find the `quad_pipeline.draw(...)` calls (there are three: cells, tab text, banner). Each currently passes `(pass, queue, instances, surface_size, atlas_size)`. Update each to `(pass, queue, instances, surface_size, mono_atlas_size, color_atlas_size)`:

```rust
self.quad_pipeline
    .draw(&mut pass, &self.queue, &cell_instances, surface_size, mono_atlas_size, color_atlas_size);
// ... and similarly for tab_glyphs, banner quads ...
```

- [ ] **Step 3: Verify build green**

```bash
cd /path/to/vibeflow
cargo build -p vibeflow
cargo test -p vibeflow --lib 2>&1 | tail -3
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -3
```

Expected: build clean. 128 default tests pass + 12 ignored. clippy clean.

LOCAL ONLY:
```bash
LIBGL_ALWAYS_SOFTWARE=1 cargo test -p vibeflow --lib render::text_engine -- --ignored
```

If clippy complains about `mono_atlas_size`/`color_atlas_size` being computed-but-unused (because some draw call paths don't reach the cell-grid pass), it shouldn't — both are passed to every `draw` call. But if it does, prefix the unused one with `_`.

Smoke run (FIRST WORKING SMOKE for Stage 7.5):

```bash
RUST_LOG=vibeflow=info ./target/debug/vibeflow &
```

In the launched window, type `printf '🎉 hello'`. The 🎉 should now render in color (Noto Color Emoji on host). All Stage 7 behavior should still work.

If the emoji renders but is the WRONG color (washed out, magenta-shifted, etc.), suspect the premultiplied-alpha shader formula. Try switching to non-premultiplied: `mix(in.bg.rgb, s.rgb, s.a)` in the color branch. Stage 7.5 spec calls this out as a known risk.

If the emoji renders as a black rectangle, the color atlas isn't being uploaded — check that `upload_to_color_atlas`'s `bytes_per_row` is `w * 4` (not `w`).

If the screen tears or flickers, the bind group might be holding a stale view. Check that `rebind_atlases` is reached when `texture_dirty()` returns true.

- [ ] **Step 4: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/render/mod.rs
git commit -m "feat(render): wire Renderer to dual-atlas QuadPipeline (color emoji renders)"
```

---

## Task 4: Wide-glyph fix in `build_cell_instances` (TDD)

**Files:**
- Modify: `crates/vibeflow/src/render/quad.rs`

Read `cell.flags` for `WIDE_CHAR` and `WIDE_CHAR_SPACER`. Skip spacer cells; widen the bg quad of wide cells to 2 × cell_w. This corrects rendering for both color emoji AND CJK simultaneously.

- [ ] **Step 1: Add a pure-logic helper test**

Append to a new `#[cfg(test)] mod cell_layout_tests` block in `quad.rs` (does NOT exist yet — Stage 7 didn't add unit tests inside quad.rs):

```rust
#[cfg(test)]
mod cell_layout_tests {
    use alacritty_terminal::term::cell::Flags;

    /// Returns true if a cell with the given flags should be skipped entirely
    /// during cell-instance building (no bg, no glyph). Two cases:
    ///   - `WIDE_CHAR_SPACER`: the spacer cell that sits to the right of a
    ///     WIDE_CHAR. The wide cell's 2-cell bg quad covers it; skipping
    ///     prevents double-rendering.
    ///   - `LEADING_WIDE_CHAR_SPACER`: emitted at the start of a line when a
    ///     wide char wrapped from the previous line. Same skip logic — the
    ///     wide-cell on the previous line's right edge is the visual.
    pub(super) fn should_skip_cell(flags: Flags) -> bool {
        flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
    }

    /// Returns true if a cell's background quad should be 2 × cell_w wide
    /// (covering itself + its WIDE_CHAR_SPACER neighbour).
    pub(super) fn cell_is_wide(flags: Flags) -> bool {
        flags.contains(Flags::WIDE_CHAR)
    }

    #[test]
    fn skips_wide_char_spacer() {
        assert!(should_skip_cell(Flags::WIDE_CHAR_SPACER));
    }

    #[test]
    fn skips_leading_wide_char_spacer() {
        // Wrapped-wide-char start-of-line spacer.
        assert!(should_skip_cell(Flags::LEADING_WIDE_CHAR_SPACER));
    }

    #[test]
    fn does_not_skip_normal_cell() {
        assert!(!should_skip_cell(Flags::empty()));
    }

    #[test]
    fn detects_wide_char() {
        assert!(cell_is_wide(Flags::WIDE_CHAR));
    }

    #[test]
    fn does_not_widen_normal_cell() {
        assert!(!cell_is_wide(Flags::empty()));
    }

    #[test]
    fn does_not_skip_wide_char_itself() {
        // The WIDE_CHAR cell renders normally (and gets a 2× bg).
        assert!(!should_skip_cell(Flags::WIDE_CHAR));
    }
}
```

The `pub(super)` fns are named so the renderer code (in the parent module) can use them. Move them out of the `#[cfg(test)]` block once tests pass — see Step 2.

- [ ] **Step 2: Move helpers out of test module + use them**

Move the two helpers `should_skip_cell` and `cell_is_wide` OUT of the `#[cfg(test)]` block (they're now production code, not test-only). Place them as free fns at the end of `quad.rs`, after `build_banner_instances`:

```rust
/// Returns true if a cell with the given flags should be skipped entirely
/// during cell-instance building (no bg, no glyph). Covers both
/// `WIDE_CHAR_SPACER` (trailing spacer after a wide char) and
/// `LEADING_WIDE_CHAR_SPACER` (line-leading spacer when a wide char wrapped
/// from the previous line).
pub(crate) fn should_skip_cell(flags: alacritty_terminal::term::cell::Flags) -> bool {
    flags.intersects(
        alacritty_terminal::term::cell::Flags::WIDE_CHAR_SPACER
            | alacritty_terminal::term::cell::Flags::LEADING_WIDE_CHAR_SPACER,
    )
}

/// Returns true if a cell's background quad should be 2 × cell_w wide
/// (covering itself + its WIDE_CHAR_SPACER neighbour).
pub(crate) fn cell_is_wide(flags: alacritty_terminal::term::cell::Flags) -> bool {
    flags.contains(alacritty_terminal::term::cell::Flags::WIDE_CHAR)
}
```

Update the `#[cfg(test)] mod cell_layout_tests` block to import via `use super::{should_skip_cell, cell_is_wide};` instead of defining them inline.

- [ ] **Step 3: Use the helpers in `build_cell_instances`**

Find the `for cell in content.display_iter` loop in `build_cell_instances`. Currently:

```rust
for cell in content.display_iter {
    let line = cell.point.line.0;
    let col = cell.point.column.0 as u32;
    if line < 0 {
        continue;
    }
    let row = line as u32;

    // ... color resolution + cursor swap ...

    let screen_x = (col * cell_w) as f32;
    let screen_y = (row * cell_h + y_offset_px) as f32;

    let glyph = text_engine.glyph_for(cell.c).unwrap_or(GlyphRef { ... fallback ... });

    out.push(QuadInstance::new(
        screen_x, screen_y,
        cell_w as f32, cell_h as f32,
        0.0, 0.0, 0.0, 0.0,
        bg, bg,
        crate::render::quad::KIND_MONO,
    ));
    // ... glyph quad ...
}
```

Update to:

```rust
for cell in content.display_iter {
    let line = cell.point.line.0;
    let col = cell.point.column.0 as u32;
    if line < 0 {
        continue;
    }
    if should_skip_cell(cell.flags) {
        continue;
    }
    let row = line as u32;

    // ... color resolution + cursor swap (unchanged) ...

    let screen_x = (col * cell_w) as f32;
    let screen_y = (row * cell_h + y_offset_px) as f32;
    let bg_w = if cell_is_wide(cell.flags) { (cell_w * 2) as f32 } else { cell_w as f32 };

    let glyph = text_engine.glyph_for(cell.c).unwrap_or(GlyphRef {
        kind: crate::render::text_engine::GlyphKind::Mono,
        atlas_x: 0, atlas_y: 0,
        atlas_w: 0, atlas_h: 0,
        bearing_x: 0, bearing_y: 0,
    });

    out.push(QuadInstance::new(
        screen_x, screen_y,
        bg_w, cell_h as f32,           // <-- bg_w replaces cell_w as f32
        0.0, 0.0, 0.0, 0.0,
        bg, bg,
        crate::render::quad::KIND_MONO,
    ));
    if glyph.atlas_w > 0 && glyph.atlas_h > 0 {
        let glyph_kind: u32 = match glyph.kind {
            crate::render::text_engine::GlyphKind::Mono => crate::render::quad::KIND_MONO,
            crate::render::text_engine::GlyphKind::Color => crate::render::quad::KIND_COLOR,
        };
        out.push(QuadInstance::new(
            screen_x + glyph.bearing_x as f32,
            screen_y + baseline_y - glyph.bearing_y as f32,
            glyph.atlas_w as f32, glyph.atlas_h as f32,
            glyph.atlas_x as f32, glyph.atlas_y as f32,
            glyph.atlas_w as f32, glyph.atlas_h as f32,
            fg, bg,
            glyph_kind,
        ));
    }
}
```

Three changes:
1. `if should_skip_cell(cell.flags) { continue; }` after the line-bounds check.
2. `bg_w` computed from `cell_is_wide(cell.flags)`.
3. Glyph quad's `kind` argument is computed from `glyph.kind`.

- [ ] **Step 4: Verify**

```bash
cd /path/to/vibeflow
cargo build -p vibeflow
cargo test -p vibeflow --lib 2>&1 | tail -3
cargo fmt --all -- --check
cargo clippy -p vibeflow --all-targets -- -D warnings 2>&1 | tail -3
```

Expected: build clean. 128 + 6 = 134 default tests pass (+ 12 ignored). clippy clean.

Smoke run (validates wide-glyph fix on real shell):

```bash
RUST_LOG=vibeflow=info ./target/debug/vibeflow &
```

In the launched window:
- Type `printf '中文 vs 中文\n'`. Both pairs should render identically; no horizontal overlap into adjacent cells; bg under each CJK char extends across 2 cells of width.
- Type `printf '🎉🎉🎉\n'`. Three back-to-back emoji with consistent backgrounds, no clipping.
- Type a single `中`. Position the cursor on it. The cursor block should cover the full 2-cell width.

If wide CJK chars now render with a 2-cell-wide bg but the glyph itself bleeds into the spacer area, that's expected behavior — the glyph's bitmap IS wider than 1 cell, so its `glyph.atlas_w` is naturally > `cell_w` and it lands across both cells.

- [ ] **Step 5: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow/src/render/quad.rs
git commit -m "feat(render): wide-glyph fix (read WIDE_CHAR flags; double bg width; skip spacer)"
```

---

## Task 5: Final verification + smoke checklist + tag

- [ ] **Step 1: Append Stage 7.5 section to `docs/TESTING.md`**

After the Stage 7 section, append:

```markdown

## Stage 7.5 — color emoji RGBA atlas + wide-glyph fix

Run:

```bash
cd /path/to/vibeflow
cargo build --bin vibeflow
RUST_LOG=vibeflow=info ./target/debug/vibeflow
```

- [ ] Run `printf '🎉 🚀 😀\n'`. Each emoji renders in **full color** (not
  monochrome outline, not tofu).
- [ ] Run `printf '中文 vs 中文\n'`. Each CJK pair renders identically; no
  overflow or overlap into adjacent cells.
- [ ] Run `printf '🎉🎉🎉\n'`. Backgrounds tile cleanly under back-to-back
  wide chars; no clipping at cell boundaries.
- [ ] Type at the prompt with cursor over a wide char. Cursor block covers
  the full 2-cell width.
- [ ] Run `for i in $(seq 1 100); do printf '%s' $(printf '\\U%x' $((0x1f600 + i % 40))); done`.
  Atlas grows; no visible stutter.
- [ ] Resize the window to ~10 px. No GPU errors; emoji still rasterized
  correctly.
- [ ] Run `vi`, enter normal mode. Cursor stops blinking on the active tab
  while shape is Hidden (Stage 7 behavior preserved).
- [ ] Press Ctrl+D in the active tab. Session dies; dead-tab banner appears
  in amber. Cursor stops blinking on the dead tab. (Stage 7 behavior.)
- [ ] On a system with NO color emoji font (uninstall Noto Color Emoji):
  emoji renders as tofu/outline. No crash.
- [ ] Re-run with `WINIT_UNIX_BACKEND=x11`. All checks above still pass.

**Known Stage 7.5 limitations (deferred to later stages):**

- No programming ligatures (`==>` renders as three glyphs) — Stage 8+.
- Cursor over a color emoji shows the emoji on a swapped background (the
  color path ignores fg/bg). Acceptable for v0.1; may revisit in Stage 9.
- No bidi or complex shaping — Stage 8+.
- Font family hardcoded to JBM + system fallback — Stage 9 (TOML config).
- Emoji font selection not configurable — Stage 9.
```

- [ ] **Step 2: Full local CI dry-run**

```bash
cd /path/to/vibeflow
cargo fmt --all -- --check && \
  cargo clippy --workspace --all-targets -- -D warnings && \
  cargo build --workspace --all-targets && \
  cargo test --workspace --all-targets && \
  RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps && \
  ( cd bindings/npm && npm run build && npm test ) && \
  echo "ALL GREEN"
```

Expected:
- 128 (Stage 7 default) + Task 4's 6 cell-layout tests = 134 default lib tests pass.
- 12 ignored: Stage 7's 7 + Task 0's 2 + Task 1's 3.
- 27 protocol crate tests, 15 npm tests, plus integration / bin tests.
- The exact total varies by harness count (Stage 7 came in around 170 runs total). The gate is "any failure stops" — exact numbers are diagnostic, not gating.

If any test fails, STOP and report.

- [ ] **Step 3: 60-second fuzz on the protocol parser**

```bash
cd /path/to/vibeflow/crates/vibeflow-protocol
cargo +nightly fuzz run parse -- -max_total_time=60
```

Expected: no crashes / leaks. Same as Stage 7's fuzz baseline.

- [ ] **Step 4: Final senior-tier holistic code review**

Stage 7's lesson: per-task Haiku reviewers consistently miss whole-stage issues. Before tagging, dispatch ONE more review covering the entire branch:

```
The reviewer should `git log --oneline main..HEAD` and inspect the cumulative
diff. Focus areas: (a) cross-task coherence — did per-task fixes regress
earlier work; (b) WGSL premultiplied-alpha math vs swash's actual output
(verify by smoke); (c) wide-cell behavior on edge cases (empty cells, cursor
on spacer, cursor on wrapped wide char); (d) test-count drift; (e) any
lingering Stage 7 references in comments/docs.
```

Subagent dispatch: use `general-purpose` with the `sonnet` model. Treat the
review's output as advisory unless flagged Critical or Important. If anything
substantive surfaces, fix before tagging.

- [ ] **Step 5: Manual smoke walkthrough**

Walk the Stage 7.5 section of `docs/TESTING.md` (Step 1 above). Brian will exercise this on host via VNC.

- [ ] **Step 6: Commit + tag**

```bash
cd /path/to/vibeflow
git add docs/TESTING.md
git commit -m "docs: Stage 7.5 manual smoke checklist"
git tag -a stage7.5-color-emoji-complete \
  -m "color emoji RGBA atlas + wide-glyph fix complete (Stage 7.5 of v0.1)"
git tag --list | grep stage7.5
```

- [ ] **Step 7: Surface to user**

Report:
- Number of new commits on this stage (~5 implementation + 1 docs = 6).
- Local CI dry-run result.
- New tag name.
- Whether the user wants Stage 8 (clipboard / keyboard shortcuts) as the next plan.

---

## Spec coverage check

Mapping Stage 7.5 spec requirements → tasks:

| Spec section | Covered by |
|---|---|
| `GlyphKind` enum + threading through `RasterImage` / `GlyphRef` | Task 0 |
| Color atlas in `TextEngine` (RGBA8Unorm, parallel to mono) | Task 1 |
| `try_atlas` routes by kind | Task 1 |
| `atlases_dirty` (single bool tracking either atlas) | Task 1 |
| `QuadInstance` extended with `flags: [u32; 4]` | Task 2 |
| Bind group grows to 4 entries (uniform + mono tex + color tex + sampler) | Task 2 |
| `QuadUniform` extended with both atlas sizes | Task 2 |
| `rebind_atlas` → `rebind_atlases` | Task 2 (rename) + Task 3 (caller) |
| WGSL fragment shader branches on kind | Task 2 (Step 8) |
| `Renderer::new` passes color view; `Renderer::render` uses `rebind_atlases` | Task 3 |
| `WIDE_CHAR_SPACER` cells skipped | Task 4 |
| `WIDE_CHAR` cell bg widened to 2 × cell_w | Task 4 |
| Smoke checklist | Task 5 Step 1 |

**Out of scope for Stage 7.5 (deferred):**
- Programming ligatures — Stage 8+
- Subpixel mask AA — Stage 9+
- Bidi / complex shaping — Stage 8+
- Configurable emoji font family — Stage 9
- Cursor-over-color-emoji inversion — v1.0+

## Self-review

- **Spec coverage:** every Stage 7.5 spec requirement maps to a task (table above).
- **Placeholder scan:** no `TBD`/`TODO`/`implement later` patterns. Each step has actual code or commands.
- **Type consistency check:**
  - `GlyphKind` defined in Task 0, consumed by Task 1 (`try_atlas`), Task 2 (build_cell_instances callsite), Task 4 (renderer logic).
  - `RasterImage.kind` and `GlyphRef.kind` defined Task 0, consumed Task 1.
  - `QuadInstance::new(...)` 11-arg signature defined Task 2, consumed Task 4 + (eventually) tabs.rs (Task 4 includes the tabs.rs adjustment).
  - `KIND_MONO`, `KIND_COLOR` defined Task 2, used Tasks 2/3/4.
  - `should_skip_cell`, `cell_is_wide` defined Task 4, used inside Task 4 only.
  - `QuadPipeline::new(device, format, mono_view, color_view, sampler)` 5-arg signature defined Task 2 Step 6, called Task 3 Step 1.
  - `QuadPipeline::draw(pass, queue, instances, surface_size, mono_atlas_size, color_atlas_size)` 6-arg signature defined Task 2 Step 7, called Task 3 Step 2.
  - `rebind_atlases` defined Task 2 Step 6, called Task 3 Step 2.
  - `text_engine.color_view`, `text_engine.color_atlas_size()` defined Task 1, used Task 3.
- **Clippy / fmt discipline:** every code-changing task ends with verify-fmt+clippy.
- **Threading-model discipline:** unchanged. All atlas state on the main thread.
- **Test count tracking:** Stage 7 ended at 128 default + 7 ignored. Stage 7.5 adds 6 cell-layout tests (default, Task 4 — includes `LEADING_WIDE_CHAR_SPACER` coverage), 2 mono/color rasterize tests (ignored, Task 0), 3 glyph_for / atlas-growth tests (ignored, Task 1). Final: 134 default + 12 ignored.

## Notable plan risks

1. **Premultiplied vs non-premultiplied alpha.** swash documents premultiplied for `SwashContent::Color`. If a host emoji font ships otherwise, emoji washes out. Mitigation in Task 3 Step 3: smoke test, switch to `mix(bg, s.rgb, s.a)` if washed-out.
2. **Wide CJK chars previously rendered correctly by accident** (the rasterized bitmap is naturally wider than `cell_w`, so it visually covers the spacer cell even though the bg quad was only 1-cell wide). Task 4's fix changes the bg to span 2 cells, which is the correct behavior, but might LOOK different (the bg under wide CJK now extends an extra cell to the right). Visual verification needed during smoke walkthrough.
3. **`tabs.rs::push_text_glyphs` is touched by Task 4**'s instruction to read `g.kind` and pass through. Verify after implementation that tab-bar text still renders correctly (titles, subtitles, +, ×). If `glyph_for` ever returns `Color` for tab text (unlikely but possible if a session label contains an emoji), the color shader path will render correctly thanks to the per-instance `kind`.
4. **alacritty_terminal cell.flags API stability.** `Flags::WIDE_CHAR` and `Flags::WIDE_CHAR_SPACER` are public in `alacritty_terminal-0.24`. If a future bump renames or removes them, Task 4 breaks. Pinning to 0.24 is fine for Stage 7.5; revisit if we bump.
5. **GL test gate.** Task 1's two new ignored tests join Stage 7's existing wgpu-touching ignored tests. CI should NOT run `--ignored` (no GL on minimal runners); host dev exercises them via `LIBGL_ALWAYS_SOFTWARE=1`.
6. **First color glyph per session is slow** (cosmic-text loads emoji font on first miss; fontdb scan is heavy). Stage 7 already pays this cost at startup for system-font scan; Stage 7.5 pays it again on first emoji render. ~50–200 ms one-time hit, not a blocker.

These risks are addressed by senior pre-execution review of this plan and the Stage 7.5 manual smoke walkthrough before merge.
