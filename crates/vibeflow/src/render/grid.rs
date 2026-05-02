//! Cell-grid render pipeline. Owns the wgpu pipeline-state object, the bind
//! group for the atlas texture + sampler, the per-frame uniform buffer, and
//! the dynamically-grown instance buffer.
