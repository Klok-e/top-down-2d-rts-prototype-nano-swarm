#import bevy_sprite::mesh2d_vertex_output::VertexOutput

@group(2) @binding(2)
var<storage> zone_map: array<u32>;
@group(2) @binding(3)
var<uniform> width: u32;
@group(2) @binding(4)
var<uniform> height: u32;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let x: u32 = min(u32(in.uv.x * f32(width)), width - 1u);
    let y: u32 = min(u32(in.uv.y * f32(height)), height - 1u);
    let value = f32(zone_map[y * width + x]);
    let gather_bit = value - floor(value / 2.0) * 2.0;
    let build_bit = floor(value / 2.0) - floor(value / 4.0) * 2.0;
    let defend_bit = floor(value / 4.0) - floor(value / 8.0) * 2.0;
    let corridor_bit = floor(value / 8.0) - floor(value / 16.0) * 2.0;
    let layer_count = gather_bit + build_bit + defend_bit + corridor_bit;
    let color_sum = vec3<f32>(
        gather_bit + build_bit + corridor_bit,
        corridor_bit,
        build_bit + defend_bit,
    );
    let color = color_sum / max(layer_count, 1.0);
    let alpha = min(layer_count, 1.0) * 0.8;
    return vec4<f32>(color, alpha);
}
