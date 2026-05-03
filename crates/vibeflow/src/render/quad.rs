//! `TextPipeline` — pixel-position textured-quad pipeline that reuses
//! [`crate::render::atlas::GlyphAtlas`]. Used by tab titles/subtitles and the
//! dead-tab banner. Stage 7 (cosmic-text) will replace the simple monospace
//! advance with shaping output.

use anyhow::Result;
use bytemuck::{Pod, Zeroable};

use crate::render::atlas::GlyphAtlas;

/// Per-instance data for `TextPipeline`. Layout matches `VsIn` in `text.wgsl`.
/// 48 bytes total. Glyph index is stored as `f32` for ergonomics (`vec4<f32>`
/// is one attribute slot); the shader casts back to `u32`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct GlyphInstance {
    /// .xy = pixel position (top-left of the glyph cell), .z = glyph_index as f32, .w = unused.
    pub pos_glyph: [f32; 4],
    pub fg: [f32; 4],
    pub bg: [f32; 4],
}

impl GlyphInstance {
    #[must_use]
    pub fn new(x_px: f32, y_px: f32, glyph: u32, fg: [f32; 4], bg: [f32; 4]) -> Self {
        Self {
            pos_glyph: [x_px, y_px, glyph as f32, 0.0],
            fg,
            bg,
        }
    }
}

/// Per-frame uniform. Layout matches `TextUniform` in `text.wgsl`. 32 bytes —
/// already a multiple of 16, no padding needed.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct TextUniform {
    surface_size_px: [f32; 2],
    cell_size_px: [f32; 2],
    atlas_size_px: [f32; 2],
    atlas_cells: [u32; 2],
}

/// Pixel-position text pipeline. Owns its bind group, uniform buffer, and a
/// dynamically-grown instance buffer.
pub struct TextPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    instance_capacity: u64,
}

const INITIAL_INSTANCE_CAPACITY: u64 = 256;
const INSTANCE_STRIDE: u64 = std::mem::size_of::<GlyphInstance>() as u64;

impl TextPipeline {
    /// Build the pipeline, sharing the atlas with `GridPipeline`.
    ///
    /// # Errors
    /// Currently infallible after the atlas is built; returns `Result` for
    /// future-proofing.
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        atlas: &GlyphAtlas,
    ) -> Result<Self> {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vibeflow-text-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("quad.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vibeflow-text-bind-group-layout"),
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
            label: Some("vibeflow-text-uniform"),
            size: std::mem::size_of::<TextUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vibeflow-text-bind-group"),
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
            label: Some("vibeflow-text-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vibeflow-text-pipeline"),
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
            label: Some("vibeflow-text-instances"),
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
            label: Some("vibeflow-text-instances"),
            size: INSTANCE_STRIDE * new_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.instance_capacity = new_capacity;
    }

    /// Upload uniforms + instance data and submit one instanced draw call.
    #[allow(clippy::too_many_arguments)]
    pub fn draw<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        queue: &wgpu::Queue,
        instances: &[GlyphInstance],
        surface_size_px: (u32, u32),
        atlas_size_px: (u32, u32),
        cell_size_px: (u32, u32),
        atlas_cells: (u32, u32),
    ) {
        if instances.is_empty() {
            return;
        }
        let uniform = TextUniform {
            surface_size_px: [surface_size_px.0 as f32, surface_size_px.1 as f32],
            cell_size_px: [cell_size_px.0 as f32, cell_size_px.1 as f32],
            atlas_size_px: [atlas_size_px.0 as f32, atlas_size_px.1 as f32],
            atlas_cells: [atlas_cells.0, atlas_cells.1],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
        queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(instances));

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
        pass.draw(0..6, 0..(instances.len() as u32));
    }
}
