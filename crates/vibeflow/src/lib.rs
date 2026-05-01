//! `vibeflow` — GPU-accelerated terminal emulator for Linux that signals AI-tool state.
//!
//! Stage 2 of v0.1 introduces only the streaming protocol dispatcher and the per-tab
//! state tracker; PTY, window, and rendering arrive in later stages. The current public
//! surface is the [`session`] module.
//!
//! See `docs/superpowers/specs/2026-05-01-vibeflow-design.md` for the full design.

pub mod session;
