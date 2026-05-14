//! Unified textured-quad pipeline. Per-instance: screen rect + atlas rect +
//! fg + bg. Used for cell glyphs, tab title/subtitle text, and the dead-tab
//! banner. Stage 7 replaces both Stage 5's `GridPipeline` and Stage 6's
//! `TextPipeline` with this single pipeline.

use anyhow::Result;
use bytemuck::{Pod, Zeroable};

/// Glyph kind: mono (R8 coverage) vs colour (premultiplied RGBA).
pub const KIND_MONO: u32 = 0;
/// Glyph kind: colour emoji from the colour atlas.
pub const KIND_COLOR: u32 = 1;

/// One textured quad. 80 bytes total (5 × 16).
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct QuadInstance {
    /// Top-left + size in surface pixels.
    pub screen_rect_px: [f32; 4],
    /// Top-left + size in atlas pixels.
    pub atlas_rect_px: [f32; 4],
    pub fg: [f32; 4],
    pub bg: [f32; 4],
    /// Lane 0: glyph kind (0 = Mono, 1 = Color). Lanes 1..=3 reserved for
    /// future per-instance flags. Total instance size: 80 bytes (5 × 16).
    pub flags: [u32; 4],
}

impl QuadInstance {
    #[allow(clippy::too_many_arguments)]
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

/// 32-byte uniform: surface size + mono atlas size + colour atlas size + pad.
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
    /// Build the pipeline. The `mono_view`, `color_view`, and `sampler` come
    /// from `TextEngine`; the bind group is rebuilt by `rebind_atlases`
    /// whenever the engine grows either texture.
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        mono_view: &wgpu::TextureView,
        color_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
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
            mono_view,
            color_view,
            sampler,
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
                        wgpu::VertexAttribute {
                            offset: 64,
                            shader_location: 4,
                            format: wgpu::VertexFormat::Uint32x4,
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

    /// Rebuild the bind group with new atlas views (after `TextEngine` grew
    /// either texture). Caller polls `TextEngine::texture_dirty()` and calls
    /// this when it returns `true`.
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

    /// Write the per-frame uniform + the full concatenated instance list.
    ///
    /// **Call this BEFORE `begin_render_pass` and ONLY ONCE per frame.**
    /// `wgpu::Queue::write_buffer` schedules its writes ahead of all encoder
    /// commands at submit time, so multiple writes to the same buffer offset
    /// within one render pass clobber each other and every draw reads only
    /// the LAST data written. Instead, the renderer concatenates cells +
    /// tab-text + banner-glyph instances into a single vec and writes once;
    /// `draw_range` then issues per-batch draws from the unified buffer.
    pub fn write_uniform_and_instances(
        &self,
        queue: &wgpu::Queue,
        surface_size_px: (u32, u32),
        mono_atlas_size_px: (u32, u32),
        color_atlas_size_px: (u32, u32),
        instances: &[QuadInstance],
    ) {
        let uniform = QuadUniform {
            surface_size_px: [surface_size_px.0 as f32, surface_size_px.1 as f32],
            mono_atlas_size_px: [mono_atlas_size_px.0 as f32, mono_atlas_size_px.1 as f32],
            color_atlas_size_px: [color_atlas_size_px.0 as f32, color_atlas_size_px.1 as f32],
            _pad: [0.0, 0.0],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
        if !instances.is_empty() {
            queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(instances));
        }
    }

    /// Draw a sub-range of the concatenated instance buffer. The range is
    /// expressed in instance indices (not bytes); each instance expands to
    /// six vertices via the shared quad shader.
    pub fn draw_range<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, range: std::ops::Range<u32>) {
        if range.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
        pass.draw(0..6, range);
    }
}

use crate::render::cursor::CursorBlink;
use crate::render::text_engine::{GlyphKind, GlyphRef, TextEngine};

/// Walk the active grid and emit one [`QuadInstance`] per visible cell.
/// Skips cells whose glyph is unrenderable (`text_engine.glyph_for` returned
/// `None`) — those become invisible cells (background still drawn via the
/// shared bg pass). Toggles the cursor cell based on `CursorBlink::visible`.
///
/// `is_session_alive` gates the cursor swap. Dead sessions still keep their
/// final term state on screen (so the user can read the "exit" line) but the
/// cursor must stop blinking — the dead-tab banner is the affordance.
///
/// `display_offset` is the current scroll position (0 = live viewport).  When
/// non-zero the viewport is showing scrollback: alacritty_terminal's
/// `display_iter` yields cells with negative `line.0` for those scrollback rows
/// and we translate them to on-screen rows the same way `build_selection_rects`
/// does (Stage 9 fix).
#[allow(clippy::too_many_arguments)]
pub fn build_cell_instances(
    term: &alacritty_terminal::term::Term<alacritty_terminal::event::VoidListener>,
    text_engine: &mut TextEngine,
    cursor: &CursorBlink,
    now: std::time::Instant,
    cell_w: u32,
    cell_h: u32,
    y_offset_px: u32,
    is_session_alive: bool,
    display_offset: usize,
) -> Vec<QuadInstance> {
    use crate::render::colors::resolve_color;
    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::vte::ansi::{CursorShape, Rgb};

    fn rgb_to_f32(rgb: Rgb) -> [f32; 4] {
        [
            rgb.r as f32 / 255.0,
            rgb.g as f32 / 255.0,
            rgb.b as f32 / 255.0,
            1.0,
        ]
    }

    let display_offset_i = display_offset as i32;
    let screen_lines = term.grid().screen_lines() as i32;

    let cursor_visible_per_blink = cursor.visible(now);
    let content = term.renderable_content();
    let cursor_state = content.cursor;
    let cursor_shape_visible = cursor_state.shape != CursorShape::Hidden;
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
    let baseline_y = text_engine.baseline_y() as f32;

    let mut out = Vec::new();
    for cell in content.display_iter {
        let line = cell.point.line.0;
        let col = cell.point.column.0 as u32;
        // Translate scrollback line indices to on-screen row.
        // - At live viewport (display_offset == 0): visible_row == line, so
        //   negative lines (above viewport) remain negative and are filtered.
        // - Scrolled up (display_offset > 0): scrollback rows that are now
        //   on-screen map to visible_row in [0, screen_lines).
        let visible_row = line + display_offset_i;
        if visible_row < 0 || visible_row >= screen_lines {
            continue;
        }
        let row = visible_row as u32;

        if should_skip_cell(cell.flags) {
            continue;
        }

        // IMPORTANT: preserve Stage 6 mod.rs's resolve_color arg order:
        // `(color, &Colors, fg_default, bg_default)` — same order for both fg
        // and bg lookups.
        let fg_rgb = resolve_color(cell.fg, colors, fg_default, bg_default);
        let bg_rgb = resolve_color(cell.bg, colors, fg_default, bg_default);
        // Skip cursor highlight when scrolled into scrollback — the cursor
        // lives on the live viewport, not in history.
        let is_cursor = display_offset == 0 && cell.point == cursor_state.point;
        let (mut fg, mut bg) = (rgb_to_f32(fg_rgb), rgb_to_f32(bg_rgb));
        if is_cursor && is_session_alive && cursor_shape_visible && cursor_visible_per_blink {
            std::mem::swap(&mut fg, &mut bg);
        }

        let screen_x = (col * cell_w) as f32;
        let screen_y = (row * cell_h + y_offset_px) as f32;
        let bg_w = if cell_is_wide(cell.flags) {
            (cell_w * 2) as f32
        } else {
            cell_w as f32
        };

        let glyph = text_engine.glyph_for(cell.c).unwrap_or(GlyphRef {
            kind: GlyphKind::Mono,
            atlas_x: 0,
            atlas_y: 0,
            atlas_w: 0,
            atlas_h: 0,
            bearing_x: 0,
            bearing_y: 0,
        });

        let glyph_kind: u32 = match glyph.kind {
            GlyphKind::Mono => KIND_MONO,
            GlyphKind::Color => KIND_COLOR,
        };

        // Background rect: zero-size atlas rect → alpha=0 → pure bg.
        out.push(QuadInstance::new(
            screen_x,
            screen_y,
            bg_w,
            cell_h as f32,
            0.0,
            0.0,
            0.0,
            0.0,
            bg,
            bg,
            KIND_MONO,
        ));
        if glyph.atlas_w > 0 && glyph.atlas_h > 0 {
            out.push(QuadInstance::new(
                screen_x + glyph.bearing_x as f32,
                screen_y + baseline_y - glyph.bearing_y as f32,
                glyph.atlas_w as f32,
                glyph.atlas_h as f32,
                glyph.atlas_x as f32,
                glyph.atlas_y as f32,
                glyph.atlas_w as f32,
                glyph.atlas_h as f32,
                fg,
                bg,
                glyph_kind,
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
    let baseline_y = text_engine.baseline_y() as f32;

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
                    text_y + baseline_y - glyph.bearing_y as f32,
                    glyph.atlas_w as f32,
                    glyph.atlas_h as f32,
                    glyph.atlas_x as f32,
                    glyph.atlas_y as f32,
                    glyph.atlas_w as f32,
                    glyph.atlas_h as f32,
                    amber,
                    black,
                    KIND_MONO,
                ));
            }
        }
        x += cell_w as f32;
    }
    out
}

/// Build glyphs for the Stage 9 config-error banner. Unlike the dead-tab
/// banner (centered, fixed amber/black), this is left-aligned at x=8 with
/// caller-provided fg/bg colors.
pub fn build_config_banner_instances(
    text: &str,
    text_engine: &mut TextEngine,
    cell_w: u32,
    cell_h: u32,
    bar_height_px: u32,
    fg_color: [f32; 4],
    bg_color: [f32; 4],
) -> Vec<QuadInstance> {
    let baseline_y = text_engine.baseline_y() as f32;
    let banner_y = bar_height_px as f32;
    let banner_h = (cell_h as f32) * 1.5;
    let text_y = banner_y + (banner_h - cell_h as f32) / 2.0;

    let mut out = Vec::with_capacity(text.chars().count());
    let mut x = 8.0_f32;
    for c in text.chars() {
        if let Some(glyph) = text_engine.glyph_for(c) {
            if glyph.atlas_w > 0 && glyph.atlas_h > 0 {
                out.push(QuadInstance::new(
                    x + glyph.bearing_x as f32,
                    text_y + baseline_y - glyph.bearing_y as f32,
                    glyph.atlas_w as f32,
                    glyph.atlas_h as f32,
                    glyph.atlas_x as f32,
                    glyph.atlas_y as f32,
                    glyph.atlas_w as f32,
                    glyph.atlas_h as f32,
                    fg_color,
                    bg_color,
                    KIND_MONO,
                ));
            }
        }
        x += cell_w as f32;
    }
    out
}

/// Returns `true` if a cell with the given flags should be skipped entirely
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

/// Returns `true` if a cell's background quad should be 2 × cell_w wide.
/// The WIDE_CHAR cell is the leading codepoint of a 2-column character; its
/// trailing spacer (`WIDE_CHAR_SPACER` in the next column or, if the wide
/// char wraps to a new line, `LEADING_WIDE_CHAR_SPACER` at column 0 of the
/// next line) is always filtered out by `should_skip_cell`.
pub(crate) fn cell_is_wide(flags: alacritty_terminal::term::cell::Flags) -> bool {
    flags.contains(alacritty_terminal::term::cell::Flags::WIDE_CHAR)
}

#[cfg(test)]
mod cell_layout_tests {
    use super::{cell_is_wide, should_skip_cell};
    use alacritty_terminal::term::cell::Flags;

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

    #[test]
    fn skip_wins_when_wide_char_and_spacer_both_set() {
        // `alacritty_terminal` does not emit this combination in practice,
        // but if a future grid model ever did, `should_skip_cell` should win
        // — otherwise we'd emit a 2× bg quad on top of the trailing spacer
        // and double-paint the cell.
        assert!(should_skip_cell(Flags::WIDE_CHAR | Flags::WIDE_CHAR_SPACER));
    }
}
