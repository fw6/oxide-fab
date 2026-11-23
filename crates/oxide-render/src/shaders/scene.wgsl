// Opaque geometry of the fab. Colours arrive as sRGB from the scene graph,
// are lit in linear space and the sRGB target encodes the result. The bake
// gives every mesh node a base colour, a tangent-space normal and an ORM map
// (R = ambient occlusion, G = roughness, B = metallic); the unit cube the graph
// uses for status lights and beacons is drawn with `params.z = 0`, which swaps
// the samples for neutral values.

struct Camera {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    light: vec4<f32>,
    ambient: vec4<f32>,
};

struct Object {
    model: mat4x4<f32>,
    color: vec4<f32>,
    // x: emissive (0 lit, 1 self-lit), y: dimmed (0..1), z: textured (0/1), w: unused
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<uniform> object: Object;

@group(2) @binding(0) var base_map: texture_2d<f32>;
@group(2) @binding(1) var normal_map: texture_2d<f32>;
@group(2) @binding(2) var orm_map: texture_2d<f32>;
@group(2) @binding(3) var map_sampler: sampler;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

@vertex
fn vs_main(input: VsIn) -> VsOut {
    var out: VsOut;
    let world = object.model * vec4<f32>(input.position, 1.0);
    out.world_position = world.xyz;
    out.clip_position = camera.view_proj * world;
    out.normal = (object.model * vec4<f32>(input.normal, 0.0)).xyz;
    out.uv = input.uv;
    return out;
}

fn srgb_to_linear(color: vec3<f32>) -> vec3<f32> {
    let low = color / 12.92;
    let high = pow((color + 0.055) / 1.055, vec3<f32>(2.4));
    return select(low, high, color > vec3<f32>(0.04045));
}

// Tangent frame from screen-space derivatives: the mesh carries no tangents,
// and the bake is a tangent-space normal map.
fn cotangent_frame(normal: vec3<f32>, position: vec3<f32>, uv: vec2<f32>) -> mat3x3<f32> {
    let dp1 = dpdx(position);
    let dp2 = dpdy(position);
    let duv1 = dpdx(uv);
    let duv2 = dpdy(uv);
    let dp2perp = cross(dp2, normal);
    let dp1perp = cross(normal, dp1);
    let tangent = dp2perp * duv1.x + dp1perp * duv2.x;
    let bitangent = dp2perp * duv1.y + dp1perp * duv2.y;
    let length = max(dot(tangent, tangent), dot(bitangent, bitangent));
    if length < 1e-12 {
        return mat3x3<f32>(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), normal);
    }
    let inverse = inverseSqrt(length);
    return mat3x3<f32>(tangent * inverse, bitangent * inverse, normal);
}

@fragment
fn fs_main(input: VsOut) -> @location(0) vec4<f32> {
    let textured = clamp(object.params.z, 0.0, 1.0);
    let uv = input.uv;
    // sRGB view: the sample already comes back in linear space.
    let base_sample = mix(vec3<f32>(1.0), textureSample(base_map, map_sampler, uv).rgb, textured);
    let normal_sample = mix(
        vec3<f32>(0.5, 0.5, 1.0),
        textureSample(normal_map, map_sampler, uv).rgb,
        textured,
    );
    let orm = mix(vec3<f32>(1.0, 0.45, 0.0), textureSample(orm_map, map_sampler, uv).rgb, textured);
    let occlusion = mix(1.0, orm.x, 0.6);
    let roughness = clamp(orm.y, 0.04, 1.0);
    let metallic = orm.z;

    let geometric = normalize(input.normal);
    let frame = cotangent_frame(geometric, input.world_position, uv);
    let normal = normalize(frame * (normal_sample * 2.0 - 1.0));

    let light = normalize(camera.light.xyz);
    let view = normalize(camera.eye.xyz - input.world_position);
    let base = srgb_to_linear(object.color.rgb) * base_sample;
    let diffuse = max(dot(normal, light), 0.0) * 0.92;
    let half_vector = normalize(light + view);
    let shininess = mix(64.0, 8.0, roughness);
    let specular = pow(max(dot(normal, half_vector), 0.0), shininess)
        * mix(0.10, 0.55, metallic)
        * (1.0 - 0.5 * roughness);
    let rim = pow(1.0 - max(dot(normal, view), 0.0), 3.0) * 0.18;
    let ambient = camera.ambient.x * occlusion;
    let lit = base * (ambient + diffuse) + mix(vec3<f32>(1.0), base, 0.5) * (specular + rim);
    let emissive = clamp(object.params.x, 0.0, 1.0);
    let dimmed = clamp(object.params.y, 0.0, 1.0);
    let color = mix(lit, base * 1.4, emissive);
    return vec4<f32>(color * mix(1.0, 0.24, dimmed), 1.0);
}
