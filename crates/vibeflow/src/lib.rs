//! `vibeflow` — GPU-accelerated terminal emulator for Linux that signals AI-tool state.
//!
//! Stage 3 of v0.1 wires up a real PTY child process behind the streaming OSC
//! dispatcher and the per-tab state tracker introduced in Stages 1–2. Stage 4
//! introduces the window and rendering. Public surface: [`session`] and [`app`].
//!
//! See `docs/superpowers/specs/2026-05-01-vibeflow-design.md` for the full design.

pub mod app;
pub mod session;
