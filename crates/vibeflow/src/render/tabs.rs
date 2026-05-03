//! Tab-bar rendering. Three pieces:
//!  * [`TabBarLayout`] — pure logic, computes per-tab rectangles + button hit zones.
//!  * `TabBarPipeline` — wgpu pipeline-state for solid-color rectangles
//!    (tab backgrounds, indicator stripes, separators, button bodies).
//!  * `TabBarRenderer` — glue that builds the per-frame instance lists from
//!    [`crate::app::App`] state + tracker states, including the Notice
//!    indicator pulse animation on `Waiting` tabs.

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
}

use anyhow::Result;
use bytemuck::{Pod, Zeroable};

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

    /// Submit one instanced draw call for all the rects.
    pub fn draw<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        queue: &wgpu::Queue,
        rects: &[RectInstance],
        surface_size_px: (u32, u32),
    ) {
        if rects.is_empty() {
            return;
        }
        let uniform = RectUniform {
            surface_size_px: [surface_size_px.0 as f32, surface_size_px.1 as f32],
            _pad: [0.0, 0.0],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
        queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(rects));

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
        pass.draw(0..6, 0..(rects.len() as u32));
    }
}
