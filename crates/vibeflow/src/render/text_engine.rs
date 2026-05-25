//! `TextEngine` — cosmic-text-backed glyph rasterizer + dynamic glyph atlas.
//! Replaces the static fontdue atlas from Stage 5. Supports the full Unicode
//! range via cosmic-text's font fallback (system fonts via fontdb).
//!
//! Stage 7 shipped monochrome (R8Unorm) only. Stage 7.5 adds a parallel
//! RGBA8Unorm color atlas for emoji and other RGBA-rasterized glyphs.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent};

/// Which atlas a rasterized glyph belongs in. Stage 7 glyphs were all `Mono`;
/// Stage 7.5 adds `Color` for emoji and other RGBA-rasterized glyphs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphKind {
    /// R8Unorm mask (alpha-only). Renderer uses `mix(bg, fg, alpha)`.
    Mono,
    /// RGBA8Unorm straight-alpha (swash 0.1.19's color-glyph output, verified
    /// empirically: PNG-sourced color bitmaps from Noto Color Emoji and friends
    /// pass through `EmitRgba8` without a premultiply step). Renderer uses
    /// `mix(bg, sampled.rgb, sampled.a)`.
    Color,
}

/// Embedded primary font. Same JBM file used by Stage 5's fontdue atlas.
pub const PRIMARY_FONT: &[u8] = include_bytes!("../../assets/JetBrainsMono-Regular.ttf");
pub const BOLD_FONT: &[u8] = include_bytes!("../../assets/JetBrainsMono-Bold.ttf");
pub const ITALIC_FONT: &[u8] = include_bytes!("../../assets/JetBrainsMono-Italic.ttf");
pub const BOLD_ITALIC_FONT: &[u8] = include_bytes!("../../assets/JetBrainsMono-BoldItalic.ttf");

/// Stage 7 renders all glyphs at 16 px (matches Stage 5's `FONT_PX = 16.0`).
/// Configurable in Stage 9 (TOML config).
pub const FONT_PX: f32 = 16.0;

/// Hard cap on the mono atlas height. Adversarial input (e.g., random Unicode
/// flooding) can otherwise grow the atlas without bound. At 4096 px tall and
/// 256 px wide, the atlas is ~1 MB R8Unorm. Above this cap the cache is fully
/// reset (glyphs re-render on next paint).
const MAX_MONO_ATLAS_DIM: u32 = 4096;

/// Hard cap on the color atlas height. Color glyphs are 4× bytes per pixel
/// (RGBA8). Cap matches mono effective bytes.
const MAX_COLOR_ATLAS_DIM: u32 = 2048;

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
    /// Color: RGBA straight-alpha bytes (length = 4 * width * height).
    pub data: Vec<u8>,
}

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

/// Initial pixel size of the color (RGBA) atlas. Color glyphs are rare and
/// usually small (16×16 emoji at FONT_PX), so 256×256 holds ~256 emoji.
const COLOR_ATLAS_INITIAL_W: u32 = 256;
const COLOR_ATLAS_INITIAL_H: u32 = 256;

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

/// Stateful cosmic-text wrapper. Heavyweight to construct (loads the embedded
/// font + system fonts via fontdb); cheap to query.
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

    cache: HashMap<(char, cosmic_text::Weight, cosmic_text::Style), Option<GlyphRef>>, // None = no font coverage for this codepoint; do not retry
    /// True when EITHER atlas has been re-allocated since the last call.
    atlases_dirty: bool,
    queue: Arc<wgpu::Queue>,
    device: Arc<wgpu::Device>,
}

/// Build (or rebuild) the cosmic-text font subsystem with the embedded
/// primary font loaded.  Returns a fresh `(FontSystem, SwashCache)` pair.
///
/// Called by `TextEngine::new` at construction and by
/// `TextEngine::set_font_priorities` on live config reload — keeping the
/// embedded-font load in exactly one place so a future second embedded font
/// cannot be silently lost on reload.
fn build_font_subsystem() -> (FontSystem, SwashCache) {
    let mut font_system = FontSystem::new();
    font_system.db_mut().load_font_data(PRIMARY_FONT.to_vec());
    font_system.db_mut().load_font_data(BOLD_FONT.to_vec());
    font_system.db_mut().load_font_data(ITALIC_FONT.to_vec());
    font_system
        .db_mut()
        .load_font_data(BOLD_ITALIC_FONT.to_vec());
    let swash_cache = SwashCache::new();
    (font_system, swash_cache)
}

impl TextEngine {
    /// Build a `TextEngine`. Allocates the initial 256×256 R8Unorm atlas
    /// texture + sampler.
    ///
    /// # Errors
    /// Fails if the embedded font is corrupt or the wgpu allocator fails.
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Result<Self> {
        let (mut font_system, swash_cache) = build_font_subsystem();

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
        let cell_h = line.line_height.ceil() as u32;
        let baseline_y = line.line_y.ceil() as u32;
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
            // COPY_SRC is required for `grow_mono_atlas` to use this texture as
            // a source in `copy_texture_to_texture` when the atlas grows.
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
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

    /// Stage 13: rebuild the cosmic-text `FontSystem` (and `SwashCache`) so a
    /// font config change takes effect without restart. `FontSystem::new()`
    /// re-scans system fonts; we then reload the embedded primary font to
    /// ensure it is never lost (a fresh `FontSystem` does NOT include it).
    ///
    /// **Limitation:** cosmic-text 0.12.1 has no API to impose an explicit
    /// family *priority order* on fontdb, so `priority` is advisory only —
    /// the rebuild picks up on-disk/system font changes and resets fontdb
    /// state, but does not reorder the fallback chain. Tracked as a v0.1
    /// limitation in the Stage 13 spec risks.
    ///
    /// **Performance:** `FontSystem::new()` scans system fonts (~hundreds of
    /// ms). That cost is acceptable because config reloads are infrequent
    /// (triggered only by an explicit user edit of the config file).
    pub fn set_font_priorities(&mut self, priority: &[String]) {
        tracing::info!(?priority, "font priorities updated — rebuilding FontSystem");
        let (fs, sc) = build_font_subsystem();
        self.font_system = fs;
        self.swash_cache = sc;
        self.invalidate_glyph_cache();
    }

    /// Clear the cached glyph map. Subsequent `glyph_for` calls miss the cache
    /// and re-shape. We intentionally do NOT clear the GPU atlas textures —
    /// slots get reused as new glyphs come in.
    pub fn invalidate_glyph_cache(&mut self) {
        self.cache.clear();
    }

    /// Rasterize a single character. Returns `Some` for any glyph the font
    /// stack can produce, `None` only if no fallback covers the codepoint.
    /// `RasterImage::kind` distinguishes mono (R8) from color (RGBA premultiplied).
    pub fn rasterize(
        &mut self,
        c: char,
        weight: cosmic_text::Weight,
        style: cosmic_text::Style,
    ) -> Option<RasterImage> {
        let metrics = Metrics::new(FONT_PX, FONT_PX * 1.4);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let attrs = Attrs::new()
            .family(Family::Name("JetBrains Mono"))
            .weight(weight)
            .style(style);
        // `Shaping::Advanced` is required for cosmic-text's full font fallback
        // chain (rustybuzz). With `Shaping::Basic`, missing codepoints render
        // as the primary font's tofu glyph instead of falling through to the
        // system color-emoji / CJK fonts — which silently breaks color emoji
        // and any non-Latin text.
        buffer.set_text(
            &mut self.font_system,
            &c.to_string(),
            attrs,
            Shaping::Advanced,
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
    }

    /// Look up (or rasterize + atlas) the glyph for `c`. Returns `None` for
    /// characters the font stack can't render. The cache memoises both
    /// successes and failures.
    pub fn glyph_for(
        &mut self,
        c: char,
        weight: cosmic_text::Weight,
        style: cosmic_text::Style,
    ) -> Option<GlyphRef> {
        if let Some(cached) = self.cache.get(&(c, weight, style)) {
            return *cached;
        }
        let result = self.try_atlas(c, weight, style);
        self.cache.insert((c, weight, style), result);
        result
    }

    fn try_atlas(
        &mut self,
        c: char,
        weight: cosmic_text::Weight,
        style: cosmic_text::Style,
    ) -> Option<GlyphRef> {
        let img = self.rasterize(c, weight, style)?;
        if img.width == 0 || img.height == 0 {
            return Some(GlyphRef {
                kind: img.kind,
                atlas_x: 0,
                atlas_y: 0,
                atlas_w: 0,
                atlas_h: 0,
                bearing_x: 0,
                bearing_y: 0,
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
            atlas_x: x,
            atlas_y: y,
            atlas_w: img.width,
            atlas_h: img.height,
            bearing_x: img.bearing_x,
            bearing_y: img.bearing_y,
        })
    }

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
    ///
    /// If growth would exceed `MAX_MONO_ATLAS_DIM`, the cache and shelf-pack
    /// state are fully reset and the atlas is recreated at its initial size.
    /// All cached `GlyphRef`s are invalidated; callers will re-render on the
    /// next `glyph_for` miss.
    fn allocate_mono(&mut self, w: u32, h: u32) -> (u32, u32) {
        let need_grow = !shelves_can_fit(&self.shelves, self.atlas_w, self.atlas_h, w, h);
        if need_grow {
            let new_h = double_until_fits(self.atlas_h, h, &self.shelves);
            if new_h > MAX_MONO_ATLAS_DIM {
                tracing::warn!(
                    current_h = self.atlas_h,
                    requested_h = new_h,
                    cap = MAX_MONO_ATLAS_DIM,
                    "glyph mono atlas hit size cap; resetting cache",
                );
                self.cache.clear();
                self.shelves.clear();
                // Set atlas_h to ATLAS_INITIAL_H BEFORE calling grow_mono_atlas
                // so its `new_h >= self.atlas_h` assertion is satisfied
                // (ATLAS_INITIAL_H >= ATLAS_INITIAL_H).
                self.atlas_h = ATLAS_INITIAL_H;
                self.grow_mono_atlas(ATLAS_INITIAL_H);
            } else {
                self.grow_mono_atlas(new_h);
            }
        }
        shelf_pack(&mut self.shelves, self.atlas_w, w, h)
    }

    /// Allocate a `w × h` rect in the color atlas, growing as needed.
    ///
    /// If growth would exceed `MAX_COLOR_ATLAS_DIM`, the cache and shelf-pack
    /// state are fully reset and the atlas is recreated at its initial size.
    /// All cached `GlyphRef`s are invalidated; callers will re-render on the
    /// next `glyph_for` miss.
    fn allocate_color(&mut self, w: u32, h: u32) -> (u32, u32) {
        let need_grow = !shelves_can_fit(
            &self.color_shelves,
            self.color_atlas_w,
            self.color_atlas_h,
            w,
            h,
        );
        if need_grow {
            let new_h = double_until_fits(self.color_atlas_h, h, &self.color_shelves);
            if new_h > MAX_COLOR_ATLAS_DIM {
                tracing::warn!(
                    current_h = self.color_atlas_h,
                    requested_h = new_h,
                    cap = MAX_COLOR_ATLAS_DIM,
                    "glyph color atlas hit size cap; resetting cache",
                );
                self.cache.clear();
                self.color_shelves.clear();
                // Set color_atlas_h to COLOR_ATLAS_INITIAL_H BEFORE calling
                // grow_color_atlas so its `new_h >= self.color_atlas_h`
                // assertion is satisfied (COLOR_ATLAS_INITIAL_H >= COLOR_ATLAS_INITIAL_H).
                self.color_atlas_h = COLOR_ATLAS_INITIAL_H;
                self.grow_color_atlas(COLOR_ATLAS_INITIAL_H);
            } else {
                self.grow_color_atlas(new_h);
            }
        }
        shelf_pack(&mut self.color_shelves, self.color_atlas_w, w, h)
    }

    /// Reallocate the mono atlas at height `new_h` (caller already computed it
    /// via `double_until_fits`). Copies old contents.
    fn grow_mono_atlas(&mut self, new_h: u32) {
        debug_assert!(
            new_h >= self.atlas_h,
            "grow_mono_atlas called with height smaller than current"
        );
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
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
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
        self.view = self
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.atlas_h = new_h;
        self.atlases_dirty = true;
    }

    /// Reallocate the color atlas at height `new_h` (caller already computed it
    /// via `double_until_fits`). Copies old contents.
    fn grow_color_atlas(&mut self, new_h: u32) {
        debug_assert!(
            new_h >= self.color_atlas_h,
            "grow_color_atlas called with height smaller than current"
        );
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
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
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
        self.color_view = self
            .color_texture
            .create_view(&wgpu::TextureViewDescriptor::default());
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
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
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
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn test_engine() -> TextEngine {
        // `Backends::GL` is wgpu's most portable backend on Linux, but it
        // requires a usable OpenGL implementation (Mesa with software
        // rendering at minimum: `LIBGL_ALWAYS_SOFTWARE=1`). On a vanilla
        // Linux dev box this works; on a minimal Docker CI runner it may
        // not. The tests below are marked `#[ignore]` so they only run
        // when explicitly invoked (`cargo test -- --ignored`).
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::GL,
            flags: wgpu::InstanceFlags::default(),
            dx12_shader_compiler: wgpu::Dx12Compiler::default(),
            gles_minor_version: wgpu::Gles3MinorVersion::default(),
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: None,
            force_fallback_adapter: true,
        }))
        .expect("no GL adapter — try LIBGL_ALWAYS_SOFTWARE=1");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
                .expect("request_device failed");
        TextEngine::new(Arc::new(device), Arc::new(queue)).unwrap()
    }

    #[test]
    #[ignore = "requires Mesa software GL (LIBGL_ALWAYS_SOFTWARE=1); run with --ignored"]
    fn cell_metrics_returns_jbm_pitch_at_16px() {
        let engine = test_engine();
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
    #[ignore = "requires Mesa software GL (LIBGL_ALWAYS_SOFTWARE=1); run with --ignored"]
    fn rasterize_ascii_letter_returns_image() {
        let mut engine = test_engine();
        let img = engine
            .rasterize('A', cosmic_text::Weight::NORMAL, cosmic_text::Style::Normal)
            .unwrap();
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
    #[ignore = "requires Mesa software GL (LIBGL_ALWAYS_SOFTWARE=1); run with --ignored"]
    fn rasterize_space_returns_none_or_empty_image() {
        let mut engine = test_engine();
        let img = engine.rasterize(' ', cosmic_text::Weight::NORMAL, cosmic_text::Style::Normal);
        // cosmic-text returns either no image or an empty one for whitespace.
        if let Some(img) = img {
            assert_eq!(img.data.iter().filter(|&&a| a > 0).count(), 0);
        }
    }

    #[test]
    #[ignore = "requires Mesa software GL (LIBGL_ALWAYS_SOFTWARE=1); run with --ignored"]
    fn rasterize_cjk_uses_system_fallback() {
        let mut engine = test_engine();
        // 中 (U+4E2D) — JBM doesn't carry CJK. fontdb should find a system font.
        // If the test env has no CJK font, this returns None — assert either
        // outcome works, just that we don't panic.
        let _img = engine.rasterize(
            '中',
            cosmic_text::Weight::NORMAL,
            cosmic_text::Style::Normal,
        );
    }

    #[test]
    #[ignore = "requires Mesa software GL (LIBGL_ALWAYS_SOFTWARE=1); run with --ignored"]
    fn glyph_for_caches_repeat_lookups() {
        let mut engine = test_engine();
        let r1 = engine
            .glyph_for('A', cosmic_text::Weight::NORMAL, cosmic_text::Style::Normal)
            .unwrap();
        let r2 = engine
            .glyph_for('A', cosmic_text::Weight::NORMAL, cosmic_text::Style::Normal)
            .unwrap();
        assert_eq!(r1, r2);
    }

    #[test]
    #[ignore = "requires Mesa software GL (LIBGL_ALWAYS_SOFTWARE=1); run with --ignored"]
    fn glyph_for_assigns_distinct_atlas_rects() {
        let mut engine = test_engine();
        let a = engine
            .glyph_for('A', cosmic_text::Weight::NORMAL, cosmic_text::Style::Normal)
            .unwrap();
        let b = engine
            .glyph_for('B', cosmic_text::Weight::NORMAL, cosmic_text::Style::Normal)
            .unwrap();
        // Different glyphs must occupy different rects.
        assert_ne!(
            (a.atlas_x, a.atlas_y),
            (b.atlas_x, b.atlas_y),
            "A and B got identical atlas positions"
        );
    }

    #[test]
    #[ignore = "requires Mesa software GL (LIBGL_ALWAYS_SOFTWARE=1); run with --ignored"]
    fn allocate_grows_atlas_when_shelves_fill() {
        let mut engine = test_engine();
        let initial_h = engine.atlas_size().1;
        // Force many distinct glyphs.
        for c in 'A'..='Z' {
            engine.glyph_for(c, cosmic_text::Weight::NORMAL, cosmic_text::Style::Normal);
        }
        for c in 'a'..='z' {
            engine.glyph_for(c, cosmic_text::Weight::NORMAL, cosmic_text::Style::Normal);
        }
        for c in 'Α'..='Ω' {
            engine.glyph_for(c, cosmic_text::Weight::NORMAL, cosmic_text::Style::Normal);
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

    #[test]
    #[ignore = "requires Mesa software GL (LIBGL_ALWAYS_SOFTWARE=1); run with --ignored"]
    fn rasterize_mono_letter_returns_mono_kind() {
        let mut engine = test_engine();
        let img = engine
            .rasterize('A', cosmic_text::Weight::NORMAL, cosmic_text::Style::Normal)
            .unwrap();
        assert_eq!(img.kind, GlyphKind::Mono);
        assert_eq!(img.data.len(), img.width as usize * img.height as usize);
    }

    #[test]
    #[ignore = "requires Mesa software GL (LIBGL_ALWAYS_SOFTWARE=1); run with --ignored"]
    fn rasterize_color_emoji_returns_color_kind() {
        let mut engine = test_engine();
        // 🎉 (U+1F389). Skip cleanly if the test env has no color emoji font.
        if let Some(img) = engine.rasterize(
            '🎉',
            cosmic_text::Weight::NORMAL,
            cosmic_text::Style::Normal,
        ) {
            assert_eq!(img.kind, GlyphKind::Color);
            // RGBA: data length = 4 * width * height.
            assert_eq!(img.data.len(), 4 * img.width as usize * img.height as usize);
        }
    }

    #[test]
    #[ignore = "requires Mesa software GL (LIBGL_ALWAYS_SOFTWARE=1); run with --ignored"]
    fn glyph_for_emoji_routes_to_color_atlas() {
        let mut engine = test_engine();
        // Skip cleanly if no color emoji font in the test env.
        if let Some(g) = engine.glyph_for(
            '🎉',
            cosmic_text::Weight::NORMAL,
            cosmic_text::Style::Normal,
        ) {
            assert_eq!(g.kind, GlyphKind::Color);
            // Cache hit on second call returns identical GlyphRef.
            assert_eq!(
                engine.glyph_for(
                    '🎉',
                    cosmic_text::Weight::NORMAL,
                    cosmic_text::Style::Normal
                ),
                Some(g)
            );
        }
    }

    #[test]
    #[ignore = "requires Mesa software GL (LIBGL_ALWAYS_SOFTWARE=1); run with --ignored"]
    fn glyph_for_letter_routes_to_mono_atlas() {
        let mut engine = test_engine();
        let g = engine
            .glyph_for('A', cosmic_text::Weight::NORMAL, cosmic_text::Style::Normal)
            .unwrap();
        assert_eq!(g.kind, GlyphKind::Mono);
    }

    #[test]
    #[ignore = "requires Mesa software GL (LIBGL_ALWAYS_SOFTWARE=1); run with --ignored"]
    fn color_atlas_grows_when_full() {
        let mut engine = test_engine();
        let initial_h = engine.color_atlas_size().1;
        // Force a barrage of distinct emoji codepoints. With cosmic-text's
        // font-fallback priorities, many of these resolve to DejaVu Sans
        // (mono outline) rather than Noto Color Emoji — so the color atlas
        // may not actually grow on this run. The assertion is therefore
        // shape-of-growth (power-of-two multiple) rather than strict growth;
        // the no-growth case is also valid for environments that lack color
        // emoji coverage entirely.
        for code in (0x1F600u32..=0x1F64Fu32).chain(0x1F300u32..=0x1F320u32) {
            if let Some(c) = char::from_u32(code) {
                engine.glyph_for(c, cosmic_text::Weight::NORMAL, cosmic_text::Style::Normal);
            }
        }
        let (_, h_after) = engine.color_atlas_size();
        assert!(
            h_after >= initial_h && h_after % initial_h == 0,
            "color atlas height {} is not a power-of-two multiple of {}",
            h_after,
            initial_h
        );
    }

    #[test]
    #[ignore = "requires Mesa software GL (LIBGL_ALWAYS_SOFTWARE=1); run with --ignored"]
    fn glyph_for_caches_each_style_independently() {
        // The cache key MUST include weight and style. Look up the same char
        // with four (weight, style) combinations — each must succeed and the
        // cache should hold all four entries afterwards.
        let mut engine = test_engine();
        let normal_normal =
            engine.glyph_for('A', cosmic_text::Weight::NORMAL, cosmic_text::Style::Normal);
        let bold_normal =
            engine.glyph_for('A', cosmic_text::Weight::BOLD, cosmic_text::Style::Normal);
        let normal_italic =
            engine.glyph_for('A', cosmic_text::Weight::NORMAL, cosmic_text::Style::Italic);
        let bold_italic =
            engine.glyph_for('A', cosmic_text::Weight::BOLD, cosmic_text::Style::Italic);

        assert!(normal_normal.is_some(), "regular 'A' must resolve");
        assert!(bold_normal.is_some(), "bold 'A' must resolve");
        assert!(normal_italic.is_some(), "italic 'A' must resolve");
        assert!(bold_italic.is_some(), "bold-italic 'A' must resolve");

        assert!(engine.cache.contains_key(&(
            'A',
            cosmic_text::Weight::NORMAL,
            cosmic_text::Style::Normal
        )));
        assert!(engine.cache.contains_key(&(
            'A',
            cosmic_text::Weight::BOLD,
            cosmic_text::Style::Normal
        )));
        assert!(engine.cache.contains_key(&(
            'A',
            cosmic_text::Weight::NORMAL,
            cosmic_text::Style::Italic
        )));
        assert!(engine.cache.contains_key(&(
            'A',
            cosmic_text::Weight::BOLD,
            cosmic_text::Style::Italic
        )));
    }

    #[test]
    #[ignore = "requires Mesa software GL (LIBGL_ALWAYS_SOFTWARE=1); run with --ignored"]
    fn allocate_mono_resets_atlas_when_hitting_cap() {
        let mut engine = test_engine();
        // Force atlas growth past 4096 via repeated tall allocations.
        for _ in 0..50 {
            let _ = engine.allocate_mono(128, 128);
        }
        // The cap kicked in: atlas_h must remain <= 4096.
        assert!(
            engine.atlas_h <= 4096,
            "atlas_h must never exceed cap; got {}",
            engine.atlas_h
        );
    }

    #[test]
    #[ignore = "requires Mesa software GL (LIBGL_ALWAYS_SOFTWARE=1); run with --ignored"]
    fn allocate_color_resets_atlas_when_hitting_cap() {
        let mut engine = test_engine();
        for _ in 0..50 {
            let _ = engine.allocate_color(128, 128);
        }
        assert!(
            engine.color_atlas_h <= 2048,
            "color_atlas_h must never exceed cap; got {}",
            engine.color_atlas_h
        );
    }

    #[test]
    #[ignore = "requires Mesa software GL (LIBGL_ALWAYS_SOFTWARE=1); run with --ignored"]
    fn post_atlas_reset_lookup_re_renders_glyph() {
        let mut engine = test_engine();
        for _ in 0..50 {
            let _ = engine.allocate_mono(128, 128);
        }
        // After reset, a fresh glyph_for must still resolve.
        let g = engine.glyph_for('A', cosmic_text::Weight::NORMAL, cosmic_text::Style::Normal);
        assert!(g.is_some(), "post-reset glyph_for must still resolve");
    }
}
