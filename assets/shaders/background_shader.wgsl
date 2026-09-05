#import bevy_sprite::mesh2d_vertex_output::VertexOutput

@group(2) @binding(0) var<uniform> paint_grid: vec4<f32>;

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn mineral_noise(p: vec2<f32>) -> f32 {
    let cell = floor(p);
    let f = fract(p);
    let blend = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(cell), hash(cell + vec2<f32>(1.0, 0.0)), blend.x),
        mix(hash(cell + vec2<f32>(0.0, 1.0)), hash(cell + vec2<f32>(1.0)), blend.x), blend.y);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let world = in.world_position.xy;
    let footprint = max(length(dpdx(world)), length(dpdy(world)));
    let patches = mineral_noise(world / 750.0);
    let grain = (mineral_noise(world / 5.0) - 0.5) * (1.0 - smoothstep(3.0, 12.0, footprint));
    var color = vec3<f32>(0.023, 0.028, 0.033) + patches * 0.012 + grain * 0.007;
    let cell = world / paint_grid.y;
    let edge = min(fract(cell), 1.0 - fract(cell)) * paint_grid.y;
    let line = 1.0 - smoothstep(0.5 * footprint, 1.5 * footprint, min(edge.x, edge.y));
    color += vec3<f32>(0.027, 0.033, 0.038) * line * paint_grid.x;
    return vec4<f32>(color, 1.0);
}
