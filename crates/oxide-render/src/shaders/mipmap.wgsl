// One mip level down: a fullscreen triangle samples the level above it.
//
// wgpu has no built-in mip generation, so each level is a render pass whose
// fragment shader samples the previous level at the centre of every source
// texel pair. The bilinear sampler averages that pair in each axis, which is
// the same 2x2 box filter the hardware would apply to a mip chain — and it is
// correct for the sRGB atlas too, because the sample decodes to linear and the
// sRGB attachment re-encodes on write.

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_mip(@builtin(vertex_index) index: u32) -> VsOut {
    var out: VsOut;
    let x = f32((index << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(index & 2u) * 2.0 - 1.0;
    out.clip_position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>(x * 0.5 + 0.5, 0.5 - y * 0.5);
    return out;
}

@fragment
fn fs_mip(input: VsOut) -> @location(0) vec4<f32> {
    return textureSample(source, source_sampler, input.uv);
}
