//! GPU rendering primitives. Stage 4 ships a minimal [`Renderer`] that opens a
//! wgpu surface on a [`winit::window::Window`] and clears it to a solid color.
//! Stage 5 layers the cell grid on top; Stage 6 adds the tab bar.

pub mod atlas;
pub mod colors;
pub mod grid;
pub mod tabs;
pub mod text;

use std::sync::Arc;

use anyhow::{Context, Result};
use winit::window::Window;

/// Default clear color for Stage 4 — matches the dark-theme background from
/// `docs/superpowers/specs/2026-05-01-vibeflow-design.md` (`#0e0e12`).
const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0x0e as f64 / 255.0,
    g: 0x0e as f64 / 255.0,
    b: 0x12 as f64 / 255.0,
    a: 1.0,
};

/// All wgpu state that lives for the duration of the window. Created once in
/// [`Renderer::new`] and dropped when the window closes.
///
/// The `Surface` borrows from the `Window`; we hold an `Arc<Window>` so the
/// lifetime is tied to the renderer rather than the calling scope.
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

impl Renderer {
    /// Initialise wgpu against the given window. Blocks on the few async wgpu
    /// calls via [`pollster::block_on`]; the operations are conceptually
    /// instantaneous (no I/O), they're just async-typed for tokio compatibility.
    ///
    /// # Errors
    /// Any wgpu init step that fails — instance creation, surface creation,
    /// adapter request (no compatible GPU), device request, surface
    /// configuration. Each error is wrapped with a `context()` describing the
    /// failed step.
    pub fn new(window: Arc<Window>) -> Result<Self> {
        let size = window.inner_size();
        // Width/height of zero is invalid for surface configuration. winit may
        // hand us a (0, 0) on the very first frame on some compositors;
        // clamp to 1 so the surface configures successfully.
        let width = size.width.max(1);
        let height = size.height.max(1);

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        // SAFETY: `Surface<'static>` requires the surface target to live as
        // long as the surface. We hold an `Arc<Window>` in the returned struct,
        // so the window outlives the surface.
        let surface = instance
            .create_surface(window.clone())
            .context("create wgpu surface")?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .context("no compatible GPU adapter found — check your GPU drivers")?;

        // `DeviceDescriptor` in wgpu 0.20 has only three fields. Later wgpu
        // versions add `memory_hints`; if you've upgraded wgpu, you'll need
        // to add it back.
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("vibeflow-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
            },
            None,
        ))
        .context("request wgpu device + queue")?;

        let surface_caps = surface.get_capabilities(&adapter);
        // Prefer sRGB so colours match designer expectations; fall back to the
        // first format if no sRGB option is offered.
        let format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: surface_caps.present_modes[0],
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

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
    }

    /// Reconfigure the surface for a new physical size. `winit::WindowEvent::Resized`
    /// fires this; the new dimensions are the *physical* (post-DPI-scaling) pixels.
    pub fn resize(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if self.surface_config.width == width && self.surface_config.height == height {
            return;
        }
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
    }

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
                let instances = build_cell_instances(term);
                if !instances.is_empty() {
                    self.grid_pipeline
                        .ensure_instance_capacity(&self.device, instances.len() as u64);
                    let (atlas_w, atlas_h) = self.atlas.pixel_size();
                    let (cell_w, cell_h) = self.atlas.cell_pitch();
                    let surface_size = (self.surface_config.width, self.surface_config.height);
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

    /// Re-apply the current `surface_config`. Used to recover from
    /// `SurfaceError::Lost` / `Outdated` — those errors mean the surface needs
    /// to be re-created with its current settings.
    pub fn reconfigure(&mut self) {
        self.surface.configure(&self.device, &self.surface_config);
    }

    /// Current surface width/height in physical pixels. Stage 4's resize math
    /// uses these to compute terminal cell rows/cols.
    #[must_use]
    pub fn surface_size(&self) -> (u32, u32) {
        (self.surface_config.width, self.surface_config.height)
    }
}

/// Walk the active grid and emit one [`crate::render::grid::CellInstance`]
/// per visible cell. Cells whose character is outside the atlas range
/// (non-printable / non-ASCII for Stage 5) are emitted with the space-glyph
/// index — visually they show only the background color, which matches
/// well-behaved control characters.
///
/// Atlas state is not threaded through this function: glyph lookup uses the
/// free [`crate::render::atlas::glyph_index`] (the atlas's pixel pitch + size
/// are read in `Renderer::render` directly).
fn build_cell_instances(
    term: &alacritty_terminal::term::Term<alacritty_terminal::event::VoidListener>,
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
    // CursorShape variants in vte 0.13.x: Block, Underline, Beam, HollowBlock, Hidden.
    // Stage 5 treats every visible shape as a block (HollowBlock and Beam draw as
    // blocks too); Stage 6+ may render them properly.
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
