//! Tab-bar rendering. Three pieces:
//!  * [`TabBarLayout`] — pure logic, computes per-tab rectangles + button hit zones.
//!  * `TabBarPipeline` — wgpu pipeline-state for solid-color rectangles
//!    (tab backgrounds, indicator stripes, separators, button bodies).
//!  * `TabBarRenderer` — glue that builds the per-frame instance lists from
//!    [`crate::app::App`] state + tracker states, including the Notice
//!    indicator pulse animation on `Waiting` tabs.

use std::time::Instant;

use anyhow::Result;
use bytemuck::{Pod, Zeroable};

use crate::app::App;
use crate::session::tracker::TabState;

/// Stage-6 default tab-bar height in pixels, expressed as (line_height × 2 + padding).
/// Computed at runtime from the atlas's cell pitch.
#[must_use]
pub fn tab_bar_height_px(cell_h_px: u32) -> u32 {
    cell_h_px * 2 + 8
}

/// Pixel width of the `+` (new tab) button at the right end of the bar.
pub const NEW_TAB_BUTTON_WIDTH_PX: u32 = 32;

/// Pixel width of the per-tab `×` close button.
pub const CLOSE_BUTTON_WIDTH_PX: u32 = 20;

/// Maximum pixel width any single tab is allowed to stretch to.
pub const MAX_TAB_WIDTH_PX: u32 = 250;

/// Minimum pixel width any tab is shown with (below this, the close button overlaps the title).
pub const MIN_TAB_WIDTH_PX: u32 = 80;

/// Layout result. Owns no GPU state — purely numeric.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabBarLayout {
    pub bar_height_px: u32,
    pub tabs: Vec<TabRect>,
    pub new_tab_button: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    #[must_use]
    pub fn contains(&self, px: u32, py: u32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabRect {
    pub idx: usize,
    pub body: Rect,
    pub close_button: Rect,
}

/// What a click at a given (px, py) hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabBarHit {
    /// Click on the tab body (anywhere except the close button) — should focus.
    TabBody(usize),
    /// Click on a tab's `×` close button — should close that tab.
    TabClose(usize),
    /// Click on the `+` button — should spawn a new tab.
    NewTab,
    /// Click landed on a separator or empty space — no action.
    None,
}

/// State for an in-progress inline tab rename. Owned by `WindowApp`; passed
/// by reference into `push_text_glyphs` so the renamed tab renders the
/// editable buffer + caret instead of the static label title.
///
/// Defined in `render::tabs` (not `window`) to avoid a circular module
/// dependency: `window` already imports `render::tabs`, so `render::tabs`
/// cannot import from `window`.
#[derive(Debug, Clone)]
pub struct RenameInputState {
    /// Index in `app.tabs()` of the tab being renamed.
    pub tab_idx: usize,
    /// User's typed text so far.
    pub buffer: String,
    /// Byte index in `buffer` for cursor position.
    pub cursor_pos: usize,
    /// Original title before rename, for Esc-cancel restore.
    pub original: String,
    /// #8: true from rename entry until the first edit/navigation. While set, the
    /// whole buffer is shown selected (highlighted, no caret) and the first
    /// insert/Backspace replaces it. Cleared by any insert, Backspace/Delete, or
    /// cursor movement.
    pub select_all: bool,
}

impl TabBarLayout {
    /// Compute layout for `tab_count` tabs in a `window_width_px`-wide window.
    /// `cell_h_px` comes from the atlas — this is what bounds the bar height.
    #[must_use]
    pub fn compute(window_width_px: u32, cell_h_px: u32, tab_count: usize) -> Self {
        let bar_height_px = tab_bar_height_px(cell_h_px);
        let new_tab_button = Rect {
            x: window_width_px.saturating_sub(NEW_TAB_BUTTON_WIDTH_PX),
            y: 0,
            w: NEW_TAB_BUTTON_WIDTH_PX,
            h: bar_height_px,
        };

        if tab_count == 0 {
            return Self {
                bar_height_px,
                tabs: Vec::new(),
                new_tab_button,
            };
        }

        let avail_width = window_width_px.saturating_sub(NEW_TAB_BUTTON_WIDTH_PX);
        let raw_tab_w = avail_width / tab_count as u32;
        let tab_w = raw_tab_w.clamp(MIN_TAB_WIDTH_PX, MAX_TAB_WIDTH_PX);

        let mut tabs = Vec::with_capacity(tab_count);
        for idx in 0..tab_count {
            let x = (idx as u32) * tab_w;
            let body = Rect {
                x,
                y: 0,
                w: tab_w,
                h: bar_height_px,
            };
            // Close button at the right edge of the tab's body.
            let close_button = Rect {
                x: x + tab_w.saturating_sub(CLOSE_BUTTON_WIDTH_PX + 4),
                y: bar_height_px / 2 - CLOSE_BUTTON_WIDTH_PX / 2,
                w: CLOSE_BUTTON_WIDTH_PX,
                h: CLOSE_BUTTON_WIDTH_PX,
            };
            tabs.push(TabRect {
                idx,
                body,
                close_button,
            });
        }

        Self {
            bar_height_px,
            tabs,
            new_tab_button,
        }
    }

    /// Hit-test a click at (px, py). Order: close button > tab body > new-tab > none.
    #[must_use]
    pub fn hit_test(&self, px: u32, py: u32) -> TabBarHit {
        if py >= self.bar_height_px {
            return TabBarHit::None; // click below the tab bar
        }
        if self.new_tab_button.contains(px, py) {
            return TabBarHit::NewTab;
        }
        for tab in &self.tabs {
            if tab.close_button.contains(px, py) {
                return TabBarHit::TabClose(tab.idx);
            }
            if tab.body.contains(px, py) {
                return TabBarHit::TabBody(tab.idx);
            }
        }
        TabBarHit::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_height_is_double_cell_plus_padding() {
        assert_eq!(tab_bar_height_px(20), 48);
        assert_eq!(tab_bar_height_px(22), 52);
    }

    #[test]
    fn compute_with_zero_tabs_returns_empty_tabs_and_button_at_right() {
        let layout = TabBarLayout::compute(960, 22, 0);
        assert!(layout.tabs.is_empty());
        assert_eq!(layout.new_tab_button.x, 960 - NEW_TAB_BUTTON_WIDTH_PX);
        assert_eq!(layout.new_tab_button.w, NEW_TAB_BUTTON_WIDTH_PX);
    }

    #[test]
    fn compute_one_tab_takes_full_available_width_clamped_to_max() {
        // 960 - 32 (new tab btn) = 928, which exceeds MAX_TAB_WIDTH_PX = 250.
        let layout = TabBarLayout::compute(960, 22, 1);
        assert_eq!(layout.tabs.len(), 1);
        assert_eq!(layout.tabs[0].body.x, 0);
        assert_eq!(layout.tabs[0].body.w, MAX_TAB_WIDTH_PX);
    }

    #[test]
    fn compute_many_tabs_packs_to_min_width() {
        // 14 tabs in 960 px: 928 / 14 = 66 px per tab, below MIN = 80.
        let layout = TabBarLayout::compute(960, 22, 14);
        assert_eq!(layout.tabs.len(), 14);
        for (i, tab) in layout.tabs.iter().enumerate() {
            assert_eq!(tab.body.w, MIN_TAB_WIDTH_PX);
            assert_eq!(tab.body.x, (i as u32) * MIN_TAB_WIDTH_PX);
        }
    }

    #[test]
    fn hit_test_below_bar_returns_none() {
        let layout = TabBarLayout::compute(960, 22, 3);
        // y past the bar height
        assert_eq!(layout.hit_test(100, 100), TabBarHit::None);
    }

    #[test]
    fn hit_test_on_tab_body_returns_tab_body() {
        let layout = TabBarLayout::compute(960, 22, 3);
        // First tab spans x=0..250 (clamped to MAX). Click at (50, 10) is inside.
        assert_eq!(layout.hit_test(50, 10), TabBarHit::TabBody(0));
    }

    #[test]
    fn hit_test_on_close_button_returns_tab_close() {
        let layout = TabBarLayout::compute(960, 22, 3);
        // First tab's close button is near the right edge of the tab body.
        let close = layout.tabs[0].close_button;
        assert_eq!(
            layout.hit_test(close.x + 1, close.y + 1),
            TabBarHit::TabClose(0)
        );
    }

    #[test]
    fn hit_test_on_new_tab_button_returns_new_tab() {
        let layout = TabBarLayout::compute(960, 22, 3);
        // The + button is at x=960-32=928, y=0.
        assert_eq!(layout.hit_test(940, 10), TabBarHit::NewTab);
    }

    #[test]
    fn hit_test_in_gap_between_tabs_returns_none_or_body() {
        // Tabs are contiguous (no visual gap in Stage 6); every x within
        // [0, total_tabs_width) is some tab. Adding a separator is a Stage 9
        // visual polish item.
        let layout = TabBarLayout::compute(960, 22, 4);
        // 4 tabs, 928 / 4 = 232 px each (< MAX 250). x=232 is the start of tab 1.
        assert_eq!(layout.hit_test(232, 10), TabBarHit::TabBody(1));
    }

    #[test]
    fn pulse_alpha_at_t_zero_is_in_range() {
        let a = pulse_alpha(0.0);
        assert!((0.4..=1.0).contains(&a), "got {}", a);
    }

    #[test]
    fn pulse_alpha_oscillates_over_a_full_period() {
        // Full period is 1.4 s. The alpha hits both ends across that span.
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        for i in 0..100 {
            let t = (i as f32) * 0.014; // 100 samples across 1.4 s
            let a = pulse_alpha(t);
            min = min.min(a);
            max = max.max(a);
        }
        assert!(min < 0.5, "expected min near 0.4, got {}", min);
        assert!(max > 0.95, "expected max near 1.0, got {}", max);
    }

    fn default_palette() -> [[f32; 4]; 4] {
        [
            [
                0x5f as f32 / 255.0,
                0xff as f32 / 255.0,
                0x9f as f32 / 255.0,
                1.0,
            ],
            [
                0x5f as f32 / 255.0,
                0xb4 as f32 / 255.0,
                0xff as f32 / 255.0,
                1.0,
            ],
            [
                0xff as f32 / 255.0,
                0xbd as f32 / 255.0,
                0x2e as f32 / 255.0,
                1.0,
            ],
            [
                0x45 as f32 / 255.0,
                0x45 as f32 / 255.0,
                0x4f as f32 / 255.0,
                1.0,
            ],
        ]
    }

    #[test]
    fn indicator_color_is_amber_for_waiting() {
        let palette = default_palette();
        let c = indicator_color(TabState::Waiting, &palette);
        // 0xff = 1.0, 0xbd ≈ 0.74, 0x2e ≈ 0.18.
        assert!((c[0] - 1.0).abs() < 0.01);
        assert!((c[1] - 0.74).abs() < 0.05);
        assert!((c[2] - 0.18).abs() < 0.05);
    }

    #[test]
    fn indicator_color_is_transparent_for_active() {
        let palette = default_palette();
        assert_eq!(
            indicator_color(TabState::Active, &palette),
            [0.0, 0.0, 0.0, 0.0]
        );
    }

    #[test]
    fn subtitle_color_returns_amber_for_waiting() {
        let palette = default_palette();
        let fallback = [0.0, 0.0, 0.0, 1.0];
        let c = subtitle_color(TabState::Waiting, fallback, &palette);
        // Same as indicator_color(Waiting) but alpha forced to 1.0.
        assert!((c[0] - 1.0).abs() < 0.01);
        assert!((c[1] - 0.74).abs() < 0.05);
        assert!((c[2] - 0.18).abs() < 0.05);
        assert_eq!(c[3], 1.0);
    }

    #[test]
    fn subtitle_color_falls_back_to_title_for_active() {
        let palette = default_palette();
        let fallback = [0.5, 0.6, 0.7, 1.0];
        let c = subtitle_color(TabState::Active, fallback, &palette);
        assert_eq!(c, fallback);
    }
}

/// Per-instance data for [`TabBarPipeline`]. 32 bytes.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct RectInstance {
    /// .xy = pos_px (top-left), .zw = size_px (width, height).
    pub pos_size: [f32; 4],
    pub color: [f32; 4],
}

impl RectInstance {
    #[must_use]
    pub fn new(x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) -> Self {
        Self {
            pos_size: [x, y, w, h],
            color,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct RectUniform {
    surface_size_px: [f32; 2],
    _pad: [f32; 2],
}

/// Solid-color-rectangle pipeline for the tab bar.
pub struct TabBarPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    instance_capacity: u64,
}

const INITIAL_RECT_CAPACITY: u64 = 64;
const RECT_STRIDE: u64 = std::mem::size_of::<RectInstance>() as u64;

impl TabBarPipeline {
    /// Build the pipeline.
    ///
    /// # Errors
    /// Currently infallible.
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Result<Self> {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vibeflow-tabs-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("tabs.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vibeflow-tabs-bind-group-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vibeflow-tabs-uniform"),
            size: std::mem::size_of::<RectUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vibeflow-tabs-bind-group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vibeflow-tabs-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vibeflow-tabs-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: RECT_STRIDE,
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
                    ],
                }],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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
            label: Some("vibeflow-tabs-instances"),
            size: RECT_STRIDE * INITIAL_RECT_CAPACITY,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            pipeline,
            bind_group,
            uniform_buffer,
            instance_buffer,
            instance_capacity: INITIAL_RECT_CAPACITY,
        })
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
            label: Some("vibeflow-tabs-instances"),
            size: RECT_STRIDE * new_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.instance_capacity = new_capacity;
    }

    /// Write the per-frame uniform + the full concatenated rect list. See
    /// `QuadPipeline::write_uniform_and_instances` for the full rationale —
    /// in short, multiple `queue.write_buffer` calls to the same offset
    /// within one render pass clobber each other, so the renderer must
    /// concatenate tab-rects + banner-rect + bell-rect into a single vec
    /// and write once before the pass.
    pub fn write_uniform_and_instances(
        &self,
        queue: &wgpu::Queue,
        surface_size_px: (u32, u32),
        rects: &[RectInstance],
    ) {
        let uniform = RectUniform {
            surface_size_px: [surface_size_px.0 as f32, surface_size_px.1 as f32],
            _pad: [0.0, 0.0],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
        if !rects.is_empty() {
            queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(rects));
        }
    }

    /// Draw a sub-range of the concatenated rect buffer. The range is
    /// expressed in instance indices (not bytes); each rect expands to six
    /// vertices via the shared rect shader.
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

/// Notice-indicator colors. Palette maps TabState to colors as follows:
/// - `TabState::Active` always returns transparent (hardcoded).
/// - `TabState::Done` → `palette[0]` (active/success — green by default).
/// - `TabState::Working` → `palette[1]` (blue by default).
/// - `TabState::Waiting` → `palette[2]` (amber by default).
/// - `TabState::Idle` → `palette[3]` (gray by default).
fn indicator_color(state: TabState, palette: &[[f32; 4]; 4]) -> [f32; 4] {
    match state {
        TabState::Active => [0.0, 0.0, 0.0, 0.0],
        TabState::Done => palette[0],
        TabState::Working => palette[1],
        TabState::Waiting => palette[2],
        TabState::Idle => palette[3],
    }
}

/// Subtitle text color: tracker-state-tinted for non-`Active` states,
/// falls back to the title fg for `Active`.
fn subtitle_color(state: TabState, fallback_fg: [f32; 4], palette: &[[f32; 4]; 4]) -> [f32; 4] {
    let mut c = indicator_color(state, palette);
    if c[3] == 0.0 {
        return fallback_fg;
    }
    c[3] = 1.0; // ensure opaque even though indicator may pulse
    c
}

/// Compute the pulse alpha for a `Waiting` tab at time `t` (seconds since some epoch).
/// 1.4 s sine wave between 0.4 and 1.0.
#[must_use]
pub fn pulse_alpha(t_secs: f32) -> f32 {
    let omega = std::f32::consts::TAU / 1.4;
    let sin = (t_secs * omega).sin(); // -1 to 1
    0.7 + 0.3 * sin // 0.4 to 1.0
}

/// Width of the Notice indicator stripe in pixels. Spec: 6px.
pub const INDICATOR_STRIPE_WIDTH_PX: u32 = 6;

/// Tab background colors (active vs inactive). Spec: active is "slightly lighter".
const BG_ACTIVE: [f32; 4] = [
    0x1a as f32 / 255.0,
    0x1a as f32 / 255.0,
    0x22 as f32 / 255.0,
    1.0,
];
const BG_INACTIVE: [f32; 4] = [
    0x15 as f32 / 255.0,
    0x15 as f32 / 255.0,
    0x1c as f32 / 255.0,
    1.0,
];

/// Title text color (slightly muted on inactive tabs).
const FG_ACTIVE: [f32; 4] = [
    0xe5 as f32 / 255.0,
    0xe5 as f32 / 255.0,
    0xe5 as f32 / 255.0,
    1.0,
];
const FG_INACTIVE: [f32; 4] = [
    0x7a as f32 / 255.0,
    0x7a as f32 / 255.0,
    0x82 as f32 / 255.0,
    1.0,
];

/// Glue between `App` state, `TabBarLayout`, and the wgpu pipelines.
/// Stateless except for the pulse-time epoch.
pub struct TabBarRenderer {
    /// Wall-clock time at which the renderer was constructed; pulse alpha is
    /// computed as `(now - epoch).as_secs_f32()`.
    epoch: Instant,
}

impl TabBarRenderer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            epoch: Instant::now(),
        }
    }

    /// Build the RectInstance list (tab backgrounds + indicator stripes + close
    /// buttons + new-tab button) for the current `App` state.
    #[allow(clippy::too_many_arguments)]
    pub fn build_rects(
        &self,
        app: &App,
        layout: &TabBarLayout,
        palette: &[[f32; 4]; 4],
        rename_state: Option<&RenameInputState>,
        cell_metrics: (u32, u32),
        cursor_blink: &crate::render::cursor::CursorBlink,
        now: Instant,
    ) -> Vec<RectInstance> {
        let mut rects = Vec::new();
        let bar_height = layout.bar_height_px as f32;
        let active_idx = app.active();
        // Continuous monotonic time — pulse_alpha cycles every 1.4 s naturally.
        // (Do NOT use `t.fract()` here — that resets phase every integer second
        // and produces a visible jump.)
        let pulse = pulse_alpha(self.epoch.elapsed().as_secs_f32());

        // Tab backgrounds first (so stripes draw on top).
        for tab in &layout.tabs {
            let is_active = tab.idx == active_idx && tab.idx < app.tabs().len();
            let bg = if is_active { BG_ACTIVE } else { BG_INACTIVE };
            rects.push(RectInstance::new(
                tab.body.x as f32,
                tab.body.y as f32,
                tab.body.w as f32,
                tab.body.h as f32,
                bg,
            ));

            // Notice indicator stripe (6px on the left edge).
            let session = match app.tabs().get(tab.idx) {
                Some(s) => s,
                None => continue,
            };
            let state = session.state();
            let mut color = indicator_color(state, palette);
            if state == TabState::Waiting {
                color[3] = pulse; // sine-modulated alpha
            }
            // Skip if the color is fully transparent (active state).
            if color[3] > 0.0 {
                rects.push(RectInstance::new(
                    tab.body.x as f32,
                    tab.body.y as f32,
                    INDICATOR_STRIPE_WIDTH_PX as f32,
                    bar_height,
                    color,
                ));
            }

            // Per-tab close button background (subtle).
            rects.push(RectInstance::new(
                tab.close_button.x as f32,
                tab.close_button.y as f32,
                tab.close_button.w as f32,
                tab.close_button.h as f32,
                [0.0, 0.0, 0.0, 0.3],
            ));

            // Stage 9: rename overlay — subtle white tint + caret on the
            // tab being edited.
            if let Some(rs) = rename_state {
                if rs.tab_idx == tab.idx {
                    rects.push(RectInstance::new(
                        tab.body.x as f32,
                        tab.body.y as f32,
                        tab.body.w as f32,
                        tab.body.h as f32,
                        [1.0, 1.0, 1.0, 0.10],
                    ));
                    let (cell_w, cell_h) = cell_metrics;
                    let title_x_start =
                        tab.body.x as f32 + (INDICATOR_STRIPE_WIDTH_PX as f32) + 6.0;
                    let title_y = tab.body.y as f32 + 2.0;
                    if rs.select_all && !rs.buffer.is_empty() {
                        // #8: whole-buffer selection — highlight the title (clipped
                        // to the same text area the glyphs use) and SUPPRESS the
                        // caret, so a name wider than the tab never renders its
                        // caret off-tab on rename entry.
                        let text_w = rs.buffer.chars().count() as f32 * cell_w as f32;
                        let clip_right =
                            tab.body.x as f32 + tab.body.w as f32 - tab.close_button.w as f32 - 4.0;
                        let max_w = (clip_right - title_x_start).max(0.0);
                        let hl_w = text_w.min(max_w);
                        if hl_w > 0.0 {
                            rects.push(RectInstance::new(
                                title_x_start,
                                title_y,
                                hl_w,
                                cell_h as f32,
                                [0.25, 0.50, 0.95, 0.45],
                            ));
                        }
                    } else {
                        let chars_before = rs.buffer[..rs.cursor_pos].chars().count() as f32;
                        let caret_x = title_x_start + chars_before * cell_w as f32;
                        // Gate caret visibility on the active tab's blink phase.
                        if cursor_blink.visible(now) {
                            rects.push(RectInstance::new(
                                caret_x,
                                title_y,
                                2.0,
                                cell_h as f32,
                                [1.0, 1.0, 1.0, 0.85],
                            ));
                        }
                    }
                }
            }
        }

        // New-tab button background.
        rects.push(RectInstance::new(
            layout.new_tab_button.x as f32,
            layout.new_tab_button.y as f32,
            layout.new_tab_button.w as f32,
            layout.new_tab_button.h as f32,
            [0.0, 0.0, 0.0, 0.3],
        ));

        rects
    }

    /// Build the QuadInstance list for tab titles + subtitles + `+` / `×`
    /// button glyphs. `rename_state` is `Some` while an inline rename is in
    /// progress — the renamed tab renders its editable buffer instead of the
    /// static label title.
    pub fn build_glyphs(
        &self,
        app: &App,
        layout: &TabBarLayout,
        text_engine: &mut crate::render::text_engine::TextEngine,
        palette: &[[f32; 4]; 4],
        rename_state: Option<&RenameInputState>,
    ) -> Vec<crate::render::quad::QuadInstance> {
        let mut glyphs = Vec::new();
        let active_idx = app.active();
        let (cell_w, cell_h) = text_engine.cell_metrics();
        let cell_w_f = cell_w as f32;
        let cell_h_f = cell_h as f32;

        for tab in &layout.tabs {
            let is_active = tab.idx == active_idx && tab.idx < app.tabs().len();
            let fg = if is_active { FG_ACTIVE } else { FG_INACTIVE };
            let bg = if is_active { BG_ACTIVE } else { BG_INACTIVE };

            let session = match app.tabs().get(tab.idx) {
                Some(s) => s,
                None => continue,
            };
            let label = session.label();

            // Title on line 1 (top of tab body). While renaming, substitute the
            // editable buffer for the renamed tab's title.
            let title_to_render: &str = if let Some(rs) = rename_state {
                if rs.tab_idx == tab.idx {
                    &rs.buffer
                } else {
                    &label.title
                }
            } else {
                &label.title
            };
            let title_x_start = tab.body.x as f32 + (INDICATOR_STRIPE_WIDTH_PX as f32) + 6.0;
            let title_y = tab.body.y as f32 + 2.0;
            push_text_glyphs(
                &mut glyphs,
                text_engine,
                title_to_render,
                (title_x_start, title_y),
                cell_w_f,
                fg,
                bg,
                tab.body.x + tab.body.w - tab.close_button.w - 4,
            );

            // Subtitle on line 2 (below title).
            let subtitle_x_start = title_x_start;
            let subtitle_y = title_y + cell_h_f;
            push_text_glyphs(
                &mut glyphs,
                text_engine,
                &label.subtitle,
                (subtitle_x_start, subtitle_y),
                cell_w_f,
                subtitle_color(session.state(), fg, palette),
                bg,
                tab.body.x + tab.body.w - tab.close_button.w - 4,
            );

            // `×` glyph centered in the close button. Using `bg` here makes
            // the close button visually defined by the × glyph alone.
            let close_glyph_x =
                tab.close_button.x as f32 + (tab.close_button.w as f32 - cell_w_f) / 2.0;
            let close_glyph_y =
                tab.close_button.y as f32 + (tab.close_button.h as f32 - cell_h_f) / 2.0;
            push_text_glyphs(
                &mut glyphs,
                text_engine,
                "×",
                (close_glyph_x, close_glyph_y),
                cell_w_f,
                fg,
                bg,
                tab.close_button.x + tab.close_button.w,
            );
        }

        // `+` glyph centered in the new-tab button. Use BG_INACTIVE so the
        // glyph's bg rectangle blends into the tab-bar strip; the visible mark
        // is just the `+` character itself.
        let nb = layout.new_tab_button;
        let plus_glyph_x = nb.x as f32 + (nb.w as f32 - cell_w_f) / 2.0;
        let plus_glyph_y = nb.y as f32 + (nb.h as f32 - cell_h_f) / 2.0;
        push_text_glyphs(
            &mut glyphs,
            text_engine,
            "+",
            (plus_glyph_x, plus_glyph_y),
            cell_w_f,
            FG_ACTIVE,
            BG_INACTIVE,
            nb.x + nb.w,
        );

        glyphs
    }

    /// Helper for the smoke checklist + tests: time since this renderer was
    /// constructed, in seconds. Used to drive the pulse animation.
    #[must_use]
    pub fn elapsed_secs(&self) -> f32 {
        self.epoch.elapsed().as_secs_f32()
    }
}

impl Default for TabBarRenderer {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a string to a sequence of `QuadInstance`s laid out at integer
/// pixel positions, clipped to a max-x boundary so titles/subtitles don't
/// spill onto the close button.
///
/// Made `pub(crate)` in Stage 10 so `render::context_menu::build_glyphs` can
/// reuse the exact same glyph-placement logic without duplication.
#[allow(clippy::too_many_arguments)]
pub(crate) fn push_text_glyphs(
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
        if let Some(g) =
            text_engine.glyph_for(c, cosmic_text::Weight::NORMAL, cosmic_text::Style::Normal)
        {
            if g.atlas_w > 0 && g.atlas_h > 0 {
                let kind: u32 = match g.kind {
                    crate::render::text_engine::GlyphKind::Mono => crate::render::quad::KIND_MONO,
                    crate::render::text_engine::GlyphKind::Color => crate::render::quad::KIND_COLOR,
                };
                out.push(crate::render::quad::QuadInstance::new(
                    x + g.bearing_x as f32,
                    y + baseline_y - g.bearing_y as f32,
                    g.atlas_w as f32,
                    g.atlas_h as f32,
                    g.atlas_x as f32,
                    g.atlas_y as f32,
                    g.atlas_w as f32,
                    g.atlas_h as f32,
                    fg,
                    bg,
                    kind,
                ));
            }
        }
        x += cell_w;
    }
}
