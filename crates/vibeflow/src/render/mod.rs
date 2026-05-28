//! GPU rendering primitives. Stage 4 ships a minimal [`Renderer`] that opens a
//! wgpu surface on a [`winit::window::Window`] and clears it to a solid color.
//! Stage 5 layers the cell grid on top; Stage 6 adds the tab bar.
//! Stage 7 migrates cell rendering to QuadPipeline + cosmic-text TextEngine.

pub mod about;
pub mod bell;
pub mod colors;
pub mod confirm_close;
pub mod context_menu;
pub mod cursor;
pub mod mouse_encoder;
pub mod quad; // formerly `text` — see Step 3
pub(crate) mod scrollbar;
pub mod selection;
pub mod tabs;
pub mod text_engine;

use std::sync::Arc;

use anyhow::{Context, Result};
use winit::window::Window;

fn build_selection_rects(
    selection: &crate::render::selection::SelectionTracker,
    term: &alacritty_terminal::term::Term<alacritty_terminal::event::VoidListener>,
    cell_w: u32,
    cell_h: u32,
    bar_height_px: u32,
    selection_color: [f32; 4],
    display_offset: usize,
) -> Vec<crate::render::tabs::RectInstance> {
    use alacritty_terminal::grid::Dimensions;
    let screen_lines = term.grid().screen_lines() as i32;
    let display_offset_i = display_offset as i32;
    selection
        .cells(term)
        .filter_map(|p| {
            // Stage 12: with display_offset > 0, scrollback rows are visible.
            // `p.line.0` is negative for scrollback; we translate to on-screen
            // y as: visible_row = p.line.0 + display_offset.
            // - On live viewport (display_offset == 0): visible_row = p.line.0,
            //   so negative lines map to negative visible_row and get filtered.
            // - When scrolled up: scrollback lines that are now on-screen map
            //   to non-negative visible_row in [0, screen_lines).
            let visible_row = p.line.0 + display_offset_i;
            if visible_row < 0 || visible_row >= screen_lines {
                return None;
            }
            Some(crate::render::tabs::RectInstance::new(
                (p.column.0 as u32 * cell_w) as f32,
                (visible_row as u32 * cell_h + bar_height_px) as f32,
                cell_w as f32,
                cell_h as f32,
                selection_color,
            ))
        })
        .collect()
}

/// Default clear color for Stage 4 — matches the dark-theme background from
/// `docs/superpowers/specs/2026-05-01-vibeflow-design.md` (`#0e0e12`).
const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0x0e as f64 / 255.0,
    g: 0x0e as f64 / 255.0,
    b: 0x12 as f64 / 255.0,
    a: 1.0,
};

/// Stage 13 follow-up: the wgpu clear color for a frame. Uses the active
/// session's resolved theme background when set, so the uncovered
/// remainder strip (`width % cell_w` / `height % cell_h`, never covered by
/// a cell rect) matches the theme instead of the hardcoded default.
/// Falls back to `CLEAR_COLOR` when there is no theme / no Background slot.
/// `Colors` stores `Rgb` (no alpha) so the clear color is always opaque.
fn theme_clear_color(
    theme_colors: Option<&alacritty_terminal::term::color::Colors>,
) -> wgpu::Color {
    use alacritty_terminal::vte::ansi::NamedColor;
    match theme_colors.and_then(|c| c[NamedColor::Background]) {
        Some(rgb) => wgpu::Color {
            r: rgb.r as f64 / 255.0,
            g: rgb.g as f64 / 255.0,
            b: rgb.b as f64 / 255.0,
            a: 1.0,
        },
        None => CLEAR_COLOR,
    }
}

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
    /// Stage 9: configurable selection-rect color. Defaults to Stage 8's hardcoded value.
    selection_color: [f32; 4],
    /// Stage 9: configurable indicator-stripe palette. Order: [active_done, working, waiting, inactive_idle].
    /// `TabState::Active` always renders transparent (hardcoded).
    indicator_colors: [[f32; 4]; 4],
    /// Stage 10: context-menu color palette. Written by `set_menu_colors` from hot-reload.
    /// Read by `build_rects` (Task 13). Defaults to all-zeros until `apply_config` fires.
    menu_colors: crate::render::context_menu::MenuColors,
    /// Stage 12: scrollbar track + thumb colors. Defaults from
    /// `Config::default_values()` at construction; overwritten by
    /// `WindowApp::apply_config` from `[colors] scrollbar_*` keys.
    scrollbar_colors: crate::render::scrollbar::ScrollbarColors,
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
        let defaults = crate::config::Config::default_values();

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
            // Initial palette comes from `Config::default_values` so the
            // pre-`apply_config` frame matches what the user sees once their
            // config (or its defaults) lands. Single source of truth.
            selection_color: defaults.colors.selection,
            indicator_colors: [
                defaults.colors.indicator_active,
                defaults.colors.indicator_working,
                defaults.colors.indicator_waiting,
                defaults.colors.indicator_inactive,
            ],
            menu_colors: crate::render::context_menu::MenuColors {
                bg: defaults.colors.menu_bg,
                border: defaults.colors.menu_border,
                text: defaults.colors.menu_text,
                text_disabled: defaults.colors.menu_text_disabled,
                shortcut: defaults.colors.menu_shortcut,
                focus_bg: defaults.colors.menu_focus_bg,
            },
            scrollbar_colors: crate::render::scrollbar::ScrollbarColors {
                track: defaults.colors.scrollbar_track,
                thumb: defaults.colors.scrollbar_thumb,
            },
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
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        term: Option<&alacritty_terminal::term::Term<alacritty_terminal::event::VoidListener>>,
        app: &crate::app::App,
        error_banner: &crate::config::error_banner::ErrorBannerState,
        rename_state: Option<&crate::render::tabs::RenameInputState>,
        context_menu: Option<&crate::render::context_menu::ContextMenuState>,
        about_open: bool,
        confirm_close: Option<&crate::render::confirm_close::ConfirmCloseState>,
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
        let (is_active_session_alive, active_display_offset) = app
            .tabs()
            .get(app.active())
            .map(|s| (s.is_alive(), s.display_offset()))
            .unwrap_or((false, 0));
        // Stage 13: copy the active session's optional theme colors so the
        // borrow of `app` ends before the `build_cell_instances` call (which
        // also mutably borrows `self.text_engine`). `Colors` is `Copy`
        // (fixed-size array of `Copy` elements), so this is a cheap stack copy.
        let active_theme_colors: Option<alacritty_terminal::term::color::Colors> =
            app.tabs().get(app.active()).and_then(|s| s.theme_colors);
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
                active_display_offset,
                active_theme_colors.as_ref(),
            )
        } else {
            Vec::new()
        };
        let tab_rects = self.tab_bar.build_rects(
            app,
            &layout,
            &self.indicator_colors,
            rename_state,
            (cell_w, cell_h),
            &self.cursor,
            now,
        );
        let selection_rects = if let Some(active) = app.tabs().get(app.active()) {
            if active.selection.current().is_some() {
                let display_offset = active.display_offset();
                build_selection_rects(
                    &active.selection,
                    active.term(),
                    cell_w,
                    cell_h,
                    layout.bar_height_px,
                    self.selection_color,
                    display_offset,
                )
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        let tab_glyphs = self.tab_bar.build_glyphs(
            app,
            &layout,
            &mut self.text_engine,
            &self.indicator_colors,
            rename_state,
        );

        // Stage 9 config-error banner. Left-aligned at x=8, dark-red bg.
        let config_banner_h = (cell_h as f32) * 1.5;
        let config_banner_bg: [f32; 4] = [0.40, 0.10, 0.10, 0.85];
        let config_banner_fg: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
        let (config_banner_rect, config_banner_glyphs) = if error_banner.visible() {
            let rect = crate::render::tabs::RectInstance::new(
                0.0,
                layout.bar_height_px as f32,
                surface_size.0 as f32,
                config_banner_h,
                config_banner_bg,
            );
            let text = error_banner.display_text();
            let glyphs = crate::render::quad::build_config_banner_instances(
                &text,
                &mut self.text_engine,
                cell_w,
                cell_h,
                layout.bar_height_px,
                config_banner_fg,
                config_banner_bg,
            );
            (Some(rect), glyphs)
        } else {
            (None, Vec::new())
        };

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

        // Build the per-frame banner rect + bell rect up front so we can
        // concatenate them with the tab-bar rects into a single instance
        // buffer write before the render pass starts. (See the doc comment
        // on `TabBarPipeline::write_uniform_and_instances` for why.)
        let banner_rect = banner_quads.as_ref().map(|_| {
            let banner_h = (cell_h as f32) * 2.0;
            let banner_y = layout.bar_height_px as f32 + 16.0;
            let banner_w = surface_size.0 as f32;
            crate::render::tabs::RectInstance::new(
                0.0,
                banner_y,
                banner_w,
                banner_h,
                [0.0, 0.0, 0.0, 0.85],
            )
        });
        let bell_rect = (bell_alpha > 0.0).then(|| {
            crate::render::tabs::RectInstance::new(
                0.0,
                0.0,
                surface_size.0 as f32,
                surface_size.1 as f32,
                [1.0, 1.0, 1.0, bell_alpha],
            )
        });

        // Compute offsets for the unified buffers. Each pipeline lays its
        // instances out in draw order so each `draw_range` reads its own
        // disjoint slice.
        let cell_count = cell_instances.len() as u32;
        let tab_glyph_offset = cell_count;
        let tab_glyph_count = tab_glyphs.len() as u32;
        let config_banner_glyph_offset = tab_glyph_offset + tab_glyph_count;
        let config_banner_glyph_count = config_banner_glyphs.len() as u32;
        let banner_glyph_offset = config_banner_glyph_offset + config_banner_glyph_count;
        let banner_glyph_count = banner_quads.as_ref().map_or(0, |v| v.len()) as u32;
        // menu_glyph_offset and total_quads are computed after menu_glyphs is built
        // (menu glyphs may cause atlas uploads via glyph_for, so build first).

        let tab_rect_count = tab_rects.len() as u32;
        let selection_rect_offset = tab_rect_count;
        let selection_rect_count = selection_rects.len() as u32;
        let config_banner_rect_offset = selection_rect_offset + selection_rect_count;
        let config_banner_rect_count = u32::from(config_banner_rect.is_some());
        let banner_rect_offset = config_banner_rect_offset + config_banner_rect_count;
        let banner_rect_count = u32::from(banner_rect.is_some());
        let bell_rect_offset = banner_rect_offset + banner_rect_count;
        let bell_rect_count = u32::from(bell_rect.is_some());
        // Stage 12: scrollbar (fade-in on user scroll). Appended after bell flash
        // so it sits above grid/banner; appended BEFORE menu so the context menu
        // renders on top of it.
        let scrollbar_rects: Vec<crate::render::tabs::RectInstance> = {
            use alacritty_terminal::grid::Dimensions;
            let active = app.active();
            match app.tabs().get(active) {
                Some(s) => {
                    let fade_alpha = s.scrollbar_fade.alpha(now);
                    let history_size = s.term().grid().history_size();
                    let screen_lines = s.term().grid().screen_lines();
                    let display_offset = s.display_offset();
                    crate::render::scrollbar::build_scrollbar_rects(
                        fade_alpha,
                        display_offset,
                        history_size,
                        screen_lines,
                        (
                            self.surface_config.width as f32,
                            self.surface_config.height as f32,
                        ),
                        layout.bar_height_px as f32,
                        self.scrollbar_colors,
                    )
                }
                None => Vec::new(),
            }
        };
        let menu_rects: Vec<crate::render::tabs::RectInstance> = context_menu
            .map(|m| crate::render::context_menu::build_rects(m, &self.menu_colors))
            .unwrap_or_default();
        // Menu glyph quads — built after menu rects so atlas uploads (if any)
        // complete before the render pass begins. Both are `None`-safe: when
        // `context_menu` is `None`, both vecs are empty and the draw_range
        // for menu glyphs is a no-op (QuadPipeline::draw_range skips empty ranges).
        let menu_glyphs: Vec<crate::render::quad::QuadInstance> = context_menu
            .map(|m| {
                crate::render::context_menu::build_glyphs(
                    m,
                    &self.menu_colors,
                    &mut self.text_engine,
                )
            })
            .unwrap_or_default();
        // v0.1.2: About overlay. Resolved AFTER menu so the panel paints on
        // top of every other layer. Uses the active session's theme bg/fg
        // when present (mirrors `theme_clear_color` for bg).
        let about_colors = if about_open {
            use alacritty_terminal::vte::ansi::NamedColor;
            let panel_bg: [f32; 4] =
                match active_theme_colors.and_then(|c| c[NamedColor::Background]) {
                    Some(rgb) => [
                        rgb.r as f32 / 255.0,
                        rgb.g as f32 / 255.0,
                        rgb.b as f32 / 255.0,
                        1.0,
                    ],
                    None => [
                        CLEAR_COLOR.r as f32,
                        CLEAR_COLOR.g as f32,
                        CLEAR_COLOR.b as f32,
                        1.0,
                    ],
                };
            let fg: [f32; 4] = match active_theme_colors.and_then(|c| c[NamedColor::Foreground]) {
                Some(rgb) => [
                    rgb.r as f32 / 255.0,
                    rgb.g as f32 / 255.0,
                    rgb.b as f32 / 255.0,
                    1.0,
                ],
                None => [0.9, 0.9, 0.9, 1.0],
            };
            Some(crate::render::about::AboutColors {
                backdrop: [0.0, 0.0, 0.0, 0.5],
                panel_bg,
                border_fg: fg,
                text_fg: fg,
            })
        } else {
            None
        };
        let about_rects: Vec<crate::render::tabs::RectInstance> = about_colors
            .as_ref()
            .map(|c| crate::render::about::build_about_rects(surface_size, c))
            .unwrap_or_default();
        let about_glyphs: Vec<crate::render::quad::QuadInstance> = about_colors
            .as_ref()
            .map(|c| {
                crate::render::about::build_about_glyphs(surface_size, &mut self.text_engine, c)
            })
            .unwrap_or_default();
        // v0.1.3 confirm-close overlay: re-use About's palette, then derive
        // button colours by lightening the panel_bg. Built after About so the
        // confirm dialog paints on top (though in practice the two are mutually
        // exclusive — CloseRequested never fires while About captures input).
        let confirm_close_colors: Option<crate::render::confirm_close::ConfirmCloseColors> =
            if confirm_close.is_some() {
                fn lighten(c: [f32; 4], amount: f32) -> [f32; 4] {
                    [
                        (c[0] + amount).min(1.0),
                        (c[1] + amount).min(1.0),
                        (c[2] + amount).min(1.0),
                        c[3],
                    ]
                }
                let proto = about_colors.unwrap_or_else(|| {
                    // About may not be open; derive the same palette inline.
                    use alacritty_terminal::vte::ansi::NamedColor;
                    let panel_bg: [f32; 4] =
                        match active_theme_colors.and_then(|c| c[NamedColor::Background]) {
                            Some(rgb) => [
                                rgb.r as f32 / 255.0,
                                rgb.g as f32 / 255.0,
                                rgb.b as f32 / 255.0,
                                1.0,
                            ],
                            None => [
                                CLEAR_COLOR.r as f32,
                                CLEAR_COLOR.g as f32,
                                CLEAR_COLOR.b as f32,
                                1.0,
                            ],
                        };
                    let fg: [f32; 4] =
                        match active_theme_colors.and_then(|c| c[NamedColor::Foreground]) {
                            Some(rgb) => [
                                rgb.r as f32 / 255.0,
                                rgb.g as f32 / 255.0,
                                rgb.b as f32 / 255.0,
                                1.0,
                            ],
                            None => [0.9, 0.9, 0.9, 1.0],
                        };
                    crate::render::about::AboutColors {
                        backdrop: [0.0, 0.0, 0.0, 0.5],
                        panel_bg,
                        border_fg: fg,
                        text_fg: fg,
                    }
                });
                Some(crate::render::confirm_close::ConfirmCloseColors {
                    backdrop: proto.backdrop,
                    panel_bg: proto.panel_bg,
                    border_fg: proto.border_fg,
                    text_fg: proto.text_fg,
                    button_idle_bg: lighten(proto.panel_bg, 0.08),
                    button_focus_bg: lighten(proto.panel_bg, 0.22),
                    button_text_fg: proto.text_fg,
                })
            } else {
                None
            };
        let confirm_close_rects: Vec<crate::render::tabs::RectInstance> = confirm_close
            .and_then(|s| confirm_close_colors.as_ref().map(|c| (s, c)))
            .map(|(s, c)| {
                crate::render::confirm_close::build_confirm_close_rects(surface_size, s, c)
            })
            .unwrap_or_default();
        let confirm_close_glyphs: Vec<crate::render::quad::QuadInstance> = confirm_close
            .and_then(|s| confirm_close_colors.as_ref().map(|c| (s, c)))
            .map(|(s, c)| {
                crate::render::confirm_close::build_confirm_close_glyphs(
                    surface_size,
                    s,
                    &mut self.text_engine,
                    c,
                )
            })
            .unwrap_or_default();
        let menu_glyph_offset = banner_glyph_offset + banner_glyph_count;
        let menu_glyph_count = menu_glyphs.len() as u32;
        let about_glyph_offset = menu_glyph_offset + menu_glyph_count;
        let about_glyph_count = about_glyphs.len() as u32;
        let confirm_close_glyph_offset = about_glyph_offset + about_glyph_count;
        let confirm_close_glyph_count = confirm_close_glyphs.len() as u32;
        let total_quads = confirm_close_glyph_offset + confirm_close_glyph_count;
        let scrollbar_rect_count = scrollbar_rects.len() as u32;
        let scrollbar_rect_offset = bell_rect_offset + bell_rect_count;
        let menu_rect_offset = scrollbar_rect_offset + scrollbar_rect_count;
        let menu_rect_count = menu_rects.len() as u32;
        let about_rect_offset = menu_rect_offset + menu_rect_count;
        let about_rect_count = about_rects.len() as u32;
        let confirm_close_rect_offset = about_rect_offset + about_rect_count;
        let confirm_close_rect_count = confirm_close_rects.len() as u32;
        let total_rects = confirm_close_rect_offset + confirm_close_rect_count;

        let mut all_quads = Vec::with_capacity(total_quads as usize);
        all_quads.extend_from_slice(&cell_instances);
        all_quads.extend_from_slice(&tab_glyphs);
        all_quads.extend_from_slice(&config_banner_glyphs);
        if let Some(b) = &banner_quads {
            all_quads.extend_from_slice(b);
        }
        all_quads.extend_from_slice(&menu_glyphs);
        // v0.1.2 About overlay paints above the menu glyph batch.
        all_quads.extend_from_slice(&about_glyphs);
        // v0.1.3 confirm-close overlay paints above the About glyph batch.
        all_quads.extend_from_slice(&confirm_close_glyphs);

        let mut all_rects = Vec::with_capacity(total_rects as usize);
        all_rects.extend_from_slice(&tab_rects);
        all_rects.extend_from_slice(&selection_rects);
        if let Some(r) = config_banner_rect {
            all_rects.push(r);
        }
        if let Some(r) = banner_rect {
            all_rects.push(r);
        }
        if let Some(r) = bell_rect {
            all_rects.push(r);
        }
        all_rects.extend_from_slice(&scrollbar_rects);
        all_rects.extend_from_slice(&menu_rects);
        // v0.1.2 About overlay paints above the menu rect batch.
        all_rects.extend_from_slice(&about_rects);
        // v0.1.3 confirm-close overlay paints above the About rect batch.
        all_rects.extend_from_slice(&confirm_close_rects);

        // Now grow the GPU buffers + write all instance data (still outside
        // the render-pass scope). Writes happen ONCE per pipeline per frame
        // so they don't clobber each other.
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
        self.quad_pipeline
            .ensure_instance_capacity(&self.device, total_quads as u64);
        self.tab_bar_pipeline
            .ensure_instance_capacity(&self.device, total_rects as u64);
        self.quad_pipeline.write_uniform_and_instances(
            &self.queue,
            surface_size,
            mono_atlas_size,
            color_atlas_size,
            &all_quads,
        );
        self.tab_bar_pipeline
            .write_uniform_and_instances(&self.queue, surface_size, &all_rects);

        let frame = self.surface.get_current_texture()?;
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("vibeflow-frame-encoder"),
            });

        let frame_clear = theme_clear_color(active_theme_colors.as_ref());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vibeflow-frame-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(frame_clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            // ---- Cell grid pass ----
            self.quad_pipeline.draw_range(&mut pass, 0..cell_count);

            // ---- Tab bar rects pass ----
            self.tab_bar_pipeline
                .draw_range(&mut pass, 0..tab_rect_count);

            // ---- Tab bar text pass ----
            self.quad_pipeline
                .draw_range(&mut pass, tab_glyph_offset..config_banner_glyph_offset);

            // ---- Selection highlight rects ----
            if selection_rect_count > 0 {
                self.tab_bar_pipeline
                    .draw_range(&mut pass, selection_rect_offset..config_banner_rect_offset);
            }

            // ---- Stage 9 config-error banner ----
            if config_banner_rect_count > 0 {
                self.tab_bar_pipeline
                    .draw_range(&mut pass, config_banner_rect_offset..banner_rect_offset);
                self.quad_pipeline
                    .draw_range(&mut pass, config_banner_glyph_offset..banner_glyph_offset);
            }

            // ---- Dead-tab banner ----
            if banner_rect_count > 0 {
                self.tab_bar_pipeline
                    .draw_range(&mut pass, banner_rect_offset..bell_rect_offset);
                self.quad_pipeline
                    .draw_range(&mut pass, banner_glyph_offset..menu_glyph_offset);
            }

            // ---- Bell flash overlay ----
            if bell_rect_count > 0 {
                self.tab_bar_pipeline
                    .draw_range(&mut pass, bell_rect_offset..scrollbar_rect_offset);
            }

            // ---- Scrollbar track + thumb ----
            if scrollbar_rect_count > 0 {
                self.tab_bar_pipeline
                    .draw_range(&mut pass, scrollbar_rect_offset..menu_rect_offset);
            }

            // ---- Context menu rects ----
            if menu_rect_count > 0 {
                self.tab_bar_pipeline
                    .draw_range(&mut pass, menu_rect_offset..about_rect_offset);
            }

            // ---- Context menu glyphs ----
            if menu_glyph_count > 0 {
                self.quad_pipeline
                    .draw_range(&mut pass, menu_glyph_offset..about_glyph_offset);
            }

            // ---- About overlay rects (backdrop + panel + borders) ----
            if about_rect_count > 0 {
                self.tab_bar_pipeline
                    .draw_range(&mut pass, about_rect_offset..confirm_close_rect_offset);
            }

            // ---- About overlay glyphs (topmost glyph layer below confirm-close) ----
            if about_glyph_count > 0 {
                self.quad_pipeline
                    .draw_range(&mut pass, about_glyph_offset..confirm_close_glyph_offset);
            }

            // ---- Confirm-close overlay rects (topmost rect layer) ----
            if confirm_close_rect_count > 0 {
                self.tab_bar_pipeline
                    .draw_range(&mut pass, confirm_close_rect_offset..total_rects);
            }

            // ---- Confirm-close overlay glyphs (topmost glyph layer) ----
            if confirm_close_glyph_count > 0 {
                self.quad_pipeline
                    .draw_range(&mut pass, confirm_close_glyph_offset..total_quads);
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();

        // Stage 12: if any session has an active scrollbar fade, schedule another
        // redraw so the fade-out animates smoothly. Once all fades hit 0, the
        // window returns to event-driven redraw mode.
        let any_fade_active = app.tabs().iter().any(|s| s.scrollbar_fade.alpha(now) > 0.0);
        if any_fade_active {
            self._window.request_redraw();
        }

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

    /// Update the selection-rect color (live).
    pub fn set_selection_color(&mut self, c: [f32; 4]) {
        self.selection_color = c;
    }

    /// Update the indicator-stripe color palette (live).
    pub fn set_indicator_colors(&mut self, c: [[f32; 4]; 4]) {
        self.indicator_colors = c;
    }

    /// Update the cursor blink period in milliseconds (live). 0 disables blinking.
    pub fn set_cursor_blink_ms(&mut self, ms: u64) {
        self.cursor.set_blink_ms(ms);
    }

    /// v0.1.2 dirty-redraw: expose the cursor's next blink boundary so
    /// `WindowApp::about_to_wait` can schedule a precise `WaitUntil` instead
    /// of a fixed-cadence 100 ms timer. Returns `None` when blink is disabled.
    pub fn cursor_next_toggle_at(&self, now: std::time::Instant) -> Option<std::time::Instant> {
        self.cursor.next_toggle_at(now)
    }

    /// Stage 13: live-reload the font subsystem with the new priority list.
    /// Rebuilds cosmic-text's FontSystem/SwashCache (see TextEngine::set_font_priorities).
    pub fn set_font_priorities(&mut self, priority: &[String]) {
        self.text_engine.set_font_priorities(priority);
    }

    /// Update the context-menu color palette (live). Called from
    /// `WindowApp::apply_config` on every hot-reload.
    pub fn set_menu_colors(&mut self, colors: crate::render::context_menu::MenuColors) {
        self.menu_colors = colors;
    }

    /// Update the scrollbar color palette (live). Called from
    /// `WindowApp::apply_config` on every hot-reload.
    pub fn set_scrollbar_colors(&mut self, colors: crate::render::scrollbar::ScrollbarColors) {
        self.scrollbar_colors = colors;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_color_uses_theme_bg_else_fallback() {
        use alacritty_terminal::term::color::Colors;
        use alacritty_terminal::vte::ansi::{NamedColor, Rgb};
        // No theme → fallback constant.
        assert_eq!(theme_clear_color(None), CLEAR_COLOR);
        // Theme with a Background slot → that color, alpha 1.0.
        let mut c = Colors::default();
        c[NamedColor::Background] = Some(Rgb {
            r: 0x20,
            g: 0x40,
            b: 0x60,
        });
        let got = theme_clear_color(Some(&c));
        assert!((got.r - 0x20 as f64 / 255.0).abs() < 1e-9);
        assert!((got.g - 0x40 as f64 / 255.0).abs() < 1e-9);
        assert!((got.b - 0x60 as f64 / 255.0).abs() < 1e-9);
        assert_eq!(got.a, 1.0);
        // Theme present but Background unset → fallback.
        let empty = Colors::default();
        assert_eq!(theme_clear_color(Some(&empty)), CLEAR_COLOR);
    }
}
