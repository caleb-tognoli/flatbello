// The 3D view: flat-interpolated face ids for picking, simple headlight shading for colour.

struct Uniforms {
    view_proj: mat4x4<f32>,
    light_dir: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) face: u32,
    // Bit 0: hovered, bit 1: selected.
    @location(3) state: u32,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) @interpolate(flat) face: u32,
    @location(2) @interpolate(flat) state: u32,
};

@vertex
fn vs_main(v: VsIn) -> VsOut {
    var o: VsOut;
    o.clip = u.view_proj * vec4<f32>(v.pos, 1.0);
    o.normal = v.normal;
    o.face = v.face;
    o.state = v.state;
    return o;
}

@fragment
fn fs_color(i: VsOut) -> @location(0) vec4<f32> {
    var base = vec3<f32>(0.70, 0.72, 0.76);
    if ((i.state & 2u) != 0u) {
        base = vec3<f32>(0.96, 0.55, 0.16);
    }
    if ((i.state & 1u) != 0u) {
        base = mix(base, vec3<f32>(0.30, 0.62, 1.0), 0.55);
    }
    // Two-sided: inner walls seen through holes are lit too.
    let d = abs(dot(normalize(i.normal), u.light_dir.xyz));
    return vec4<f32>(base * (0.32 + 0.68 * d), 1.0);
}

// Writes face id + 1, so that 0 means background.
@fragment
fn fs_id(i: VsOut) -> @location(0) u32 {
    return i.face + 1u;
}
