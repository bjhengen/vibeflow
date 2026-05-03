//! `TextEngine` — cosmic-text-backed glyph rasterizer + dynamic R8 glyph
//! atlas. Replaces the static fontdue atlas from Stage 5. Supports the full
//! Unicode range via cosmic-text's font fallback (system fonts via fontdb).
//!
//! Stage 7 ships monochrome (R8Unorm) only. Color-emoji rendering needs an
//! RGBA atlas + a dual-format sampling path — that's Stage 7.5.

#![allow(dead_code)] // first user is `Renderer` in Stage 7 Task 4
