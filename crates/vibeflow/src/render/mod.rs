//! GPU rendering primitives. Stage 4 ships a minimal [`Renderer`] that opens a
//! wgpu surface on a [`winit::window::Window`] and clears it to a solid color.
//! Stage 5 layers the cell grid on top; Stage 6 adds the tab bar.
//! Stage 7 migrates cell rendering to QuadPipeline + cosmic-text TextEngine.

pub mod bell;
pub mod colors;
pub mod cursor;
pub mod quad; // formerly `text` — see Step 3
pub mod tabs;
pub mod text_engine;

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
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    surface_config: wgpu::SurfaceConfiguration,
    /// cosmic-text-backed glyph rasterizer + dynamic atlas. Replaces Stage 5's
    /// static `GlyphAtlas` (fontdue).
    text_engine: crate::render::text_engine::TextEngine,
    /// Unified textured-quad pipeline used for cells, tab text, and the
    /// dead-tab banner. Replaces Stage 5's `GridPipeline` + Stage 6's
    /// `TextPipeline`.
    quad_pipeline: crate::render::quad::QuadPipeline,
    /// Solid-color rectangle pipeline (tab backgrounds, indicator stripes,
    /// button bodies, bell flash overlay).
    tab_bar_pipeline: crate::render::tabs::TabBarPipeline,
    /// Tab-bar layout glue with pulse animation.
    tab_bar: crate::render::tabs::TabBarRenderer,
    /// Cursor blink state.
    cursor: crate::render::cursor::CursorBlink,
    /// Bell flash state.
    bell: crate::render::bell::BellFlash,
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

        let device = Arc::new(device);
        let queue = Arc::new(queue);

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

        let text_engine =
            crate::render::text_engine::TextEngine::new(Arc::clone(&device), Arc::clone(&queue))?;
        let quad_pipeline = crate::render::quad::QuadPipeline::new(
            &device,
            format,
            &text_engine.view,
            &text_engine.color_view,
            &text_engine.sampler,
        )?;
        let tab_bar_pipeline = crate::render::tabs::TabBarPipeline::new(&device, format)?;
        let tab_bar = crate::render::tabs::TabBarRenderer::new();
        let cursor = crate::render::cursor::CursorBlink::new();
        let bell = crate::render::bell::BellFlash::new();

        Ok(Self {
            _window: window,
            surface,
            device,
            queue,
            surface_config,
            text_engine,
            quad_pipeline,
            tab_bar_pipeline,
            tab_bar,
            cursor,
            bell,
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
        app: &crate::app::App,
    ) -> std::result::Result<(), wgpu::SurfaceError> {
        use crate::render::tabs::TabBarLayout;

        // Cell metrics are stable for the engine lifetime; pull up front so the
        // builders can compute pixel positions. Atlas size is captured later
        // (after build_* calls) so it reflects any growth that happened this frame.
        let (cell_w, cell_h) = self.text_engine.cell_metrics();
        let surface_size = (self.surface_config.width, self.surface_config.height);
        let layout = TabBarLayout::compute(surface_size.0, cell_h, app.tabs().len());
        let now = std::time::Instant::now();

        // Banner detection — same DRY pattern as Stage 6 commit 0adc62f.
        const BANNER_TEXT: &str = "session died -- press Ctrl+Shift+R to retry";
        let banner_glyph_count = app
            .tabs()
            .get(app.active())
            .filter(|s| !s.is_alive())
            .map(|_| {
                let count = BANNER_TEXT.chars().count();
                self.tab_bar_pipeline
                    .ensure_instance_capacity(&self.device, 1);
                count
            });

        // Build per-pass instance lists OUTSIDE the render-pass scope so we can
        // call `&mut self` methods on the engine and pipelines without conflicting
        // with the render-pass borrow.
        let is_active_session_alive = app
            .tabs()
            .get(app.active())
            .map(|s| s.is_alive())
            .unwrap_or(false);
        let cell_instances = if let Some(term) = term {
            crate::render::quad::build_cell_instances(
                term,
                &mut self.text_engine,
                &self.cursor,
                now,
                cell_w,
                cell_h,
                layout.bar_height_px,
                is_active_session_alive,
            )
        } else {
            Vec::new()
        };
        let tab_rects = self.tab_bar.build_rects(app, &layout);
        let tab_glyphs = self
            .tab_bar
            .build_glyphs(app, &layout, &mut self.text_engine);
        let banner_quads = banner_glyph_count.map(|count| {
            crate::render::quad::build_banner_instances(
                BANNER_TEXT,
                count,
                &mut self.text_engine,
                cell_w,
                cell_h,
                &layout,
                surface_size.0,
            )
        });
        let bell_alpha = self.bell.tint_alpha(now);

        // Now grow the GPU buffers (still outside the render-pass scope).
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
        let total_quads =
            cell_instances.len() + tab_glyphs.len() + banner_quads.as_ref().map_or(0, |v| v.len());
        self.quad_pipeline
            .ensure_instance_capacity(&self.device, total_quads as u64);
        self.tab_bar_pipeline
            .ensure_instance_capacity(&self.device, (tab_rects.len() + 2) as u64); // +2 = banner rect + bell

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

            // ---- Cell grid pass ----
            if !cell_instances.is_empty() {
                self.quad_pipeline.draw(
                    &mut pass,
                    &self.queue,
                    &cell_instances,
                    surface_size,
                    mono_atlas_size,
                    color_atlas_size,
                );
            }

            // ---- Tab bar rects pass ----
            if !tab_rects.is_empty() {
                self.tab_bar_pipeline
                    .draw(&mut pass, &self.queue, &tab_rects, surface_size);
            }

            // ---- Tab bar text pass ----
            if !tab_glyphs.is_empty() {
                self.quad_pipeline.draw(
                    &mut pass,
                    &self.queue,
                    &tab_glyphs,
                    surface_size,
                    mono_atlas_size,
                    color_atlas_size,
                );
            }

            // ---- Dead-tab banner ----
            if let Some(quads) = banner_quads {
                // Background rect first.
                let banner_h = (cell_h as f32) * 2.0;
                let banner_y = layout.bar_height_px as f32 + 16.0;
                let banner_w = surface_size.0 as f32;
                let banner_rect = crate::render::tabs::RectInstance::new(
                    0.0,
                    banner_y,
                    banner_w,
                    banner_h,
                    [0.0, 0.0, 0.0, 0.85],
                );
                self.tab_bar_pipeline.draw(
                    &mut pass,
                    &self.queue,
                    std::slice::from_ref(&banner_rect),
                    surface_size,
                );
                self.quad_pipeline.draw(
                    &mut pass,
                    &self.queue,
                    &quads,
                    surface_size,
                    mono_atlas_size,
                    color_atlas_size,
                );
            }

            // ---- Bell flash overlay ----
            if bell_alpha > 0.0 {
                let bell_rect = crate::render::tabs::RectInstance::new(
                    0.0,
                    0.0,
                    surface_size.0 as f32,
                    surface_size.1 as f32,
                    [1.0, 1.0, 1.0, bell_alpha],
                );
                self.tab_bar_pipeline.draw(
                    &mut pass,
                    &self.queue,
                    std::slice::from_ref(&bell_rect),
                    surface_size,
                );
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

    /// Per-cell pixel pitch reported by the text engine. Stage 6 wires this
    /// into the window event loop's resize math.
    #[must_use]
    pub fn cell_pitch(&self) -> (u32, u32) {
        self.text_engine.cell_metrics()
    }

    /// Note that the active tab's session rang the bell. Triggers a 200 ms
    /// white-tint fade.
    pub fn note_bell(&mut self) {
        self.bell.note(std::time::Instant::now());
    }
}
