// vibeflow Stage 6 text shader.
//
// Sibling of grid.wgsl. Per-instance buffer carries pixel-space position
// + glyph index + fg/bg colors. Vertex shader expands 6 vertices per
// instance. Fragment shader is identical to grid.wgsl: mix bg → fg by
// the R8Unorm atlas alpha.

struct TextUniform {
    surface_size_px: vec2<f32>,   // viewport size in physical pixels
    cell_size_px:    vec2<f32>,   // per-cell pitch in physical pixels (atlas)
    atlas_size_px:   vec2<f32>,   // atlas texture size in pixels
    atlas_cells:     vec2<u32>,   // atlas layout (cols, rows of glyphs)
};

@group(0) @binding(0) var<uniform> u: TextUniform;
@group(0) @binding(1) var atlas_texture: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct VsIn {
    @builtin(vertex_index) vertex_id: u32,
    // .xy = pos_px (top-left of the glyph cell), .z = glyph_index_as_f32, .w = unused.
    @location(0) pos_glyph: vec4<f32>,
    @location(1) fg:        vec4<f32>,
    @location(2) bg:        vec4<f32>,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv:             vec2<f32>,
    @location(1) fg:             vec4<f32>,
    @location(2) bg:             vec4<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var quad_offsets = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    let corner = quad_offsets[in.vertex_id];

    let pos_top_left_px = in.pos_glyph.xy;
    let glyph_idx       = u32(in.pos_glyph.z);

    let pos_px = pos_top_left_px + corner * u.cell_size_px;
    let ndc    = (pos_px / u.surface_size_px) * 2.0 - vec2<f32>(1.0, 1.0);
    let clip_pos = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);

    let atlas_col = f32(glyph_idx % u.atlas_cells.x);
    let atlas_row = f32(glyph_idx / u.atlas_cells.x);
    let glyph_top_left_px = vec2<f32>(atlas_col, atlas_row) * u.cell_size_px;
    let glyph_pos_px      = glyph_top_left_px + corner * u.cell_size_px;
    let uv                = glyph_pos_px / u.atlas_size_px;

    var out: VsOut;
    out.clip_pos = clip_pos;
    out.uv       = uv;
    out.fg       = in.fg;
    out.bg       = in.bg;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let alpha = textureSample(atlas_texture, atlas_sampler, in.uv).r;
    let rgb   = mix(in.bg.rgb, in.fg.rgb, alpha);
    return vec4<f32>(rgb, 1.0);
}
