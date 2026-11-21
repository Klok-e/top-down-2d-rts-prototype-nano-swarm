#import bevy_sprite::mesh2d_vertex_output::VertexOutput
#import bevy_sprite::mesh2d_view_bindings::globals

@group(2) @binding(0) var heat_texture: texture_2d<f32>;
@group(2) @binding(1) var heat_sampler: sampler;

fn hash(point: vec3<f32>) -> f32 {
    return fract(sin(dot(point, vec3<f32>(127.1, 311.7, 74.7))) * 43758.5453);
}

fn noise_slice(cell: vec3<f32>, weight: vec2<f32>) -> f32 {
    let lower = mix(hash(cell), hash(cell + vec3<f32>(1.0, 0.0, 0.0)), weight.x);
    let upper = mix(hash(cell + vec3<f32>(0.0, 1.0, 0.0)),
        hash(cell + vec3<f32>(1.0, 1.0, 0.0)), weight.x);
    return mix(lower, upper, weight.y);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let heat = textureSample(heat_texture, heat_sampler, vec2<f32>(in.uv.x, 1.0 - in.uv.y));
    if heat.a == 0.0 {
        discard;
    }
    let point = vec3<f32>(in.world_position.xy / 256.0, globals.time * 0.5);
    let cell = floor(point);
    let weight = smoothstep(vec3<f32>(0.0), vec3<f32>(1.0), fract(point));
    let noise = mix(noise_slice(cell, weight.xy),
        noise_slice(cell + vec3<f32>(0.0, 0.0, 1.0), weight.xy), weight.z);
    let shimmer = mix(0.85, 1.15, noise);
    return vec4<f32>(heat.rgb * shimmer, 0.0);
}
