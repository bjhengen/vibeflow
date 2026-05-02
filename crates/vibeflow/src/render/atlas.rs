//! Glyph atlas. Pre-rasterises printable ASCII (0x20..=0x7e) via fontdue at the
//! configured pixel size, packs the glyphs into a single wgpu texture, and
//! exposes UV / metric lookups by character. Stage 7 will replace fontdue with
//! cosmic-text shaping for full Unicode + ligatures + emoji.
