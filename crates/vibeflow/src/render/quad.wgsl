// vibeflow Stage 7.5 unified quad shader. Replaces grid.wgsl + text.wgsl.
// Per-instance:
//   .xyzw screen_rect_px (x, y, w, h in surface pixels)
//   .xyzw atlas_rect_px  (x, y, w, h in atlas pixels — sized by `kind`'s atlas)
//   .rgba fg
//   .rgba bg
//   .x    kind (0 = Mono, 1 = Color); .yzw reserved
// Vertex shader expands 6 vertices per instance; UV uses the matching atlas
// size from QuadUniform. Fragment shader branches on kind:
//   Mono  → mix(bg, fg, sampled.r)
//   Color → premultiplied over: sampled.rgb + bg.rgb * (1 - sampled.a)

struct QuadUniform {
    surface_size_px:     vec2<f32>,
    mono_atlas_size_px:  vec2<f32>,
    color_atlas_size_px: vec2<f32>,
    _pad:                vec2<f32>,
};

@group(0) @binding(0) var<uniform> u: QuadUniform;
@group(0) @binding(1) var mono_texture:  texture_2d<f32>;
@group(0) @binding(2) var color_texture: texture_2d<f32>;
@group(0) @binding(3) var atlas_sampler: sampler;

struct VsIn {
    @builtin(vertex_index) vertex_id: u32,
    @location(0) screen_rect_px: vec4<f32>,
    @location(1) atlas_rect_px:  vec4<f32>,
    @location(2) fg:             vec4<f32>,
    @location(3) bg:             vec4<f32>,
    @location(4) flags:          vec4<u32>,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv:             vec2<f32>,
    @location(1) fg:             vec4<f32>,
    @location(2) bg:             vec4<f32>,
    @location(3) @interpolate(flat) kind: u32,
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

    let kind = in.flags.x;
    let atlas_size_px = select(u.mono_atlas_size_px, u.color_atlas_size_px, kind == 1u);
    let atlas_pos_px = in.atlas_rect_px.xy + corner * in.atlas_rect_px.zw;
    let uv = atlas_pos_px / atlas_size_px;

    var out: VsOut;
    out.clip_pos = clip_pos;
    out.uv       = uv;
    out.fg       = in.fg;
    out.bg       = in.bg;
    out.kind     = kind;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (in.kind == 1u) {
        // Color path. swash provides premultiplied RGBA.
        let s = textureSample(color_texture, atlas_sampler, in.uv);
        let rgb = s.rgb + in.bg.rgb * (1.0 - s.a);
        return vec4<f32>(rgb, 1.0);
    } else {
        // Mono path. Same as Stage 7.
        let alpha = textureSample(mono_texture, atlas_sampler, in.uv).r;
        let rgb   = mix(in.bg.rgb, in.fg.rgb, alpha);
        return vec4<f32>(rgb, 1.0);
    }
}
