// Grid, AMHS path, selection boxes and the level-3 beacon.

struct Camera {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    light: vec4<f32>,
    ambient: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;

struct LineIn {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
};

struct LineOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_line(input: LineIn) -> LineOut {
    var out: LineOut;
    out.clip_position = camera.view_proj * vec4<f32>(input.position, 1.0);
    out.color = input.color;
    return out;
}

fn srgb_to_linear(color: vec3<f32>) -> vec3<f32> {
    let low = color / 12.92;
    let high = pow((color + 0.055) / 1.055, vec3<f32>(2.4));
    return select(low, high, color > vec3<f32>(0.04045));
}

@fragment
fn fs_line(input: LineOut) -> @location(0) vec4<f32> {
    return vec4<f32>(srgb_to_linear(input.color.rgb), input.color.a);
}
