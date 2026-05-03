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
