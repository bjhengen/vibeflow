//! Glyph atlas. Pre-rasterises printable ASCII (0x20..=0x7e) via fontdue at the
//! configured pixel size, packs the glyphs into a single wgpu texture, and
//! exposes UV / metric lookups by character. Stage 7 will replace fontdue with
//! cosmic-text shaping for full Unicode + ligatures + emoji.

use fontdue::{Font, FontSettings};

/// Range of code points pre-rendered into the Stage 5 atlas.
const ATLAS_FIRST: u32 = 0x20; // space
const ATLAS_LAST: u32 = 0x7e; // tilde
/// Number of glyphs in the atlas.
#[cfg(test)]
const ATLAS_GLYPHS: u32 = ATLAS_LAST - ATLAS_FIRST + 1;
/// Layout: 16 glyphs per row, 6 rows.
pub const ATLAS_COLS: u32 = 16;
pub const ATLAS_ROWS: u32 = 6; // 16 * 6 = 96 >= 95 glyphs

/// Atlas grid layout (cols, rows). Used by the grid shader's UV math.
pub const ATLAS_LAYOUT: (u32, u32) = (ATLAS_COLS, ATLAS_ROWS);

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
            let y_offset =
                (ascent.ceil() as i32 - (metrics.ymin + metrics.height as i32)).max(0) as u32;
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
