//! `TextPipeline` — pixel-position textured-quad pipeline that reuses
//! [`crate::render::atlas::GlyphAtlas`]. Used by tab titles/subtitles and the
//! dead-tab banner. Stage 7 (cosmic-text) will replace the simple monospace
//! advance with shaping output.
