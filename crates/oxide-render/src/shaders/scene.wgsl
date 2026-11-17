// Opaque geometry of the fab. Colours arrive as sRGB from the scene graph,
// are lit in linear space and the sRGB target encodes the result.

struct Camera {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    light: vec4<f32>,
    ambient: vec4<f32>,
};

struct Object {
    model: mat4x4<f32>,
    color: vec4<f32>,
    // x: emissive (0 lit, 1 self-lit), y: dimmed (0..1), zw: unused
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<uniform> object: Object;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

@vertex
fn vs_main(input: VsIn) -> VsOut {
    var out: VsOut;
    let world = object.model * vec4<f32>(input.position, 1.0);
    out.world_position = world.xyz;
    out.clip_position = camera.view_proj * world;
    out.normal = (object.model * vec4<f32>(input.normal, 0.0)).xyz;
    return out;
}

fn srgb_to_linear(color: vec3<f32>) -> vec3<f32> {
    let low = color / 12.92;
    let high = pow((color + 0.055) / 1.055, vec3<f32>(2.4));
    return select(low, high, color > vec3<f32>(0.04045));
}

@fragment
fn fs_main(input: VsOut) -> @location(0) vec4<f32> {
    let normal = normalize(input.normal);
    let light = normalize(camera.light.xyz);
    let view = normalize(camera.eye.xyz - input.world_position);
    let base = srgb_to_linear(object.color.rgb);
    let diffuse = max(dot(normal, light), 0.0) * 0.92;
    let half_vector = normalize(light + view);
    let specular = pow(max(dot(normal, half_vector), 0.0), 48.0) * 0.16;
    let rim = pow(1.0 - max(dot(normal, view), 0.0), 3.0) * 0.18;
    let ambient = camera.ambient.x;
    let lit = base * (ambient + diffuse) + vec3<f32>(specular + rim) * (0.35 + base * 0.65);
    let emissive = clamp(object.params.x, 0.0, 1.0);
    let dimmed = clamp(object.params.y, 0.0, 1.0);
    let color = mix(lit, base * 1.4, emissive);
    return vec4<f32>(color * mix(1.0, 0.24, dimmed), 1.0);
}
