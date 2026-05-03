// vibeflow Stage 6 tab-bar rectangle shader.
//
// Per-instance buffer carries pixel-space rect (position + size) and an RGBA
// color. Vertex shader expands 6 vertices per instance into a screen-space
// rectangle. Fragment shader emits the color verbatim (alpha is used for the
// pulse animation on Notice indicators).

struct RectUniform {
    surface_size_px: vec2<f32>,
    _pad:            vec2<f32>,
};

@group(0) @binding(0) var<uniform> u: RectUniform;

struct VsIn {
    @builtin(vertex_index) vertex_id: u32,
    @location(0) pos_size: vec4<f32>, // .xy = pos_px (top-left), .zw = size_px
    @location(1) color:    vec4<f32>,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) color:          vec4<f32>,
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

    let pos_top_left_px = in.pos_size.xy;
    let size_px         = in.pos_size.zw;

    let pos_px = pos_top_left_px + corner * size_px;
    let ndc    = (pos_px / u.surface_size_px) * 2.0 - vec2<f32>(1.0, 1.0);
    let clip_pos = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);

    var out: VsOut;
    out.clip_pos = clip_pos;
    out.color    = in.color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return in.color;
}
