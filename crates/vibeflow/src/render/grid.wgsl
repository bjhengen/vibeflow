// vibeflow Stage 5 cell-grid shader.
//
// One draw call. Per-frame uniform supplies grid + surface dimensions in
// pixels. Per-instance buffer supplies one CellInstance per visible cell:
// (col, row, glyph_index, fg, bg). Vertex shader expands the instance into
// 6 vertices (two triangles) covering the cell rectangle. Fragment shader
// samples the atlas's grayscale alpha and mixes bg → fg.

struct GridUniform {
    surface_size_px: vec2<f32>,   // viewport size in physical pixels
    cell_size_px:    vec2<f32>,   // per-cell pitch in physical pixels
    atlas_size_px:   vec2<f32>,   // atlas texture size in pixels
    atlas_cells:     vec2<u32>,   // atlas layout (cols, rows of glyphs)
    _pad:            vec2<u32>,
};

@group(0) @binding(0) var<uniform> u: GridUniform;
@group(0) @binding(1) var atlas_texture: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct VsIn {
    @builtin(vertex_index)   vertex_id:    u32,
    @builtin(instance_index) instance_id:  u32,
    @location(0) cell:        vec4<u32>, // .x=col .y=row .z=glyph_index .w=_pad
    @location(1) fg:          vec4<f32>,
    @location(2) bg:          vec4<f32>,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv:             vec2<f32>,
    @location(1) fg:             vec4<f32>,
    @location(2) bg:             vec4<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    // The 6 vertices of the cell quad, mapping vertex_id → (corner, uv).
    // Triangle 1: (0,0) (1,0) (0,1). Triangle 2: (1,0) (1,1) (0,1).
    var quad_offsets = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    let corner = quad_offsets[in.vertex_id];

    let col      = f32(in.cell.x);
    let row      = f32(in.cell.y);
    let glyph    = in.cell.z;

    // Cell-pixel position: top-left of the cell.
    let cell_top_left_px = vec2<f32>(col, row) * u.cell_size_px;
    let pos_px           = cell_top_left_px + corner * u.cell_size_px;

    // Convert to clip space. Viewport [0..W,0..H] → NDC [-1..1, 1..-1] (Y flipped).
    let ndc = (pos_px / u.surface_size_px) * 2.0 - vec2<f32>(1.0, 1.0);
    let clip_pos = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);

    // Atlas UV: locate the glyph's cell, then sample within it.
    let atlas_col = f32(glyph % u.atlas_cells.x);
    let atlas_row = f32(glyph / u.atlas_cells.x);
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
    // Atlas is R8Unorm → only .r is meaningful; treat as alpha.
    let alpha = textureSample(atlas_texture, atlas_sampler, in.uv).r;
    let rgb   = mix(in.bg.rgb, in.fg.rgb, alpha);
    return vec4<f32>(rgb, 1.0);
}
