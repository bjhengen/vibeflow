// vibeflow Stage 7 unified quad shader. Replaces grid.wgsl + text.wgsl.
//
// Per-instance buffer carries:
//   .xyzw screen_rect_px (x, y, w, h in surface pixels)
//   .xyzw atlas_rect_px  (x, y, w, h in atlas pixels)
//   .rgba fg
//   .rgba bg
// Vertex shader expands 6 vertices per instance into a screen-space
// rectangle with linear UV across the atlas rect. Fragment shader samples
// R8Unorm `.r` as alpha and `mix(bg, fg, alpha)`.

struct QuadUniform {
    surface_size_px: vec2<f32>,
    atlas_size_px:   vec2<f32>,
};

@group(0) @binding(0) var<uniform> u: QuadUniform;
@group(0) @binding(1) var atlas_texture: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct VsIn {
    @builtin(vertex_index) vertex_id: u32,
    @location(0) screen_rect_px: vec4<f32>,
    @location(1) atlas_rect_px:  vec4<f32>,
    @location(2) fg:             vec4<f32>,
    @location(3) bg:             vec4<f32>,
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

    let screen_pos_px = in.screen_rect_px.xy + corner * in.screen_rect_px.zw;
    let ndc = (screen_pos_px / u.surface_size_px) * 2.0 - vec2<f32>(1.0, 1.0);
    let clip_pos = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);

    let atlas_pos_px = in.atlas_rect_px.xy + corner * in.atlas_rect_px.zw;
    let uv = atlas_pos_px / u.atlas_size_px;

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
