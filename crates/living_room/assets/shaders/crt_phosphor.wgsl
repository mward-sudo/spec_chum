#import bevy_pbr::forward_io::VertexOutput

// The phosphor is reconstructed in a fixed-resolution linear HDR tube image.
// Sampling it on this curved mesh preserves physical curvature and bezel
// occlusion, while mip filtering prevents tube detail from aliasing at smaller
// apparent sizes in the room camera.
@group(#{MATERIAL_BIND_GROUP}) @binding(0)
var tube_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1)
var tube_sampler: sampler;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4(textureSample(tube_texture, tube_sampler, in.uv).rgb, 1.0);
}
