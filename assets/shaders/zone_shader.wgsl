#import bevy_sprite::mesh2d_vertex_output::VertexOutput

@group(2) @binding(2)
var<storage, read> zone_map: array<u32>;
@group(2) @binding(3)
var<uniform> width: u32;
@group(2) @binding(4)
var<uniform> height: u32;

const OVERLAY_ALPHA: f32 = 0.8;

@fragment
fn fragment(
    in: VertexOutput
) -> @location(0) vec4<f32> {
    let x: u32 = u32(in.uv.x * f32(width));
    let y: u32 = u32(in.uv.y * f32(height));
    let active: u32 = zone_map[y * width + x];

    var color_sum = vec3<f32>(0.0, 0.0, 0.0);
    var count = 0.0;
    if ((active & 1u) != 0u) {
        color_sum += vec3<f32>(1.0, 0.0, 0.0);
        count += 1.0;
    }
    if ((active & 2u) != 0u) {
        color_sum += vec3<f32>(1.0, 0.0, 1.0);
        count += 1.0;
    }
    if ((active & 4u) != 0u) {
        color_sum += vec3<f32>(0.0, 0.0, 1.0);
        count += 1.0;
    }
    if ((active & 8u) != 0u) {
        color_sum += vec3<f32>(1.0, 1.0, 0.0);
        count += 1.0;
    }

    if (count > 0.0) {
        return vec4<f32>(color_sum / count, OVERLAY_ALPHA);
    }
    return vec4<f32>(0.0, 0.0, 0.0, 0.0);
}
