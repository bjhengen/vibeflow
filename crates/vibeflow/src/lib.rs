//! `vibeflow` — GPU-accelerated terminal emulator for Linux that signals AI-tool state.
//!
//! Stage 4 of v0.1 adds the winit window and the wgpu render pipeline. The
//! visible content is still just a solid clear color — the cell-grid renderer
//! arrives in Stage 5. Public surface: [`session`], [`app`], [`render`], [`window`].
//!
//! See `docs/superpowers/specs/2026-05-01-vibeflow-design.md` for the full design.

pub mod app;
pub mod clipboard;
pub mod keymap;
pub mod render;
pub mod session;
pub mod window;
