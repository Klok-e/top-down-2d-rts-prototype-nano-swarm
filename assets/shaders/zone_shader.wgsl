#import bevy_sprite::mesh2d_vertex_output::VertexOutput

@group(2) @binding(2)
var<storage> zone_map: array<u32>;
@group(2) @binding(3)
var<uniform> width: u32;
@group(2) @binding(4)
var<uniform> height: u32;

fn present(value: u32, kind: u32) -> f32 {
    return f32((value >> kind) & 1u);
}

fn owner(value: u32, kind: u32) -> u32 {
    return (value >> (4u + kind * 2u)) & 3u;
}

fn owned_color(base: vec3<f32>, ownership: u32, local: vec2<f32>) -> vec3<f32> {
    if ownership == 1u {
        return base;
    }
    if ownership == 2u {
        let stripe = select(0.5, 0.9, fract((local.x + local.y) * 8.0) < 0.5);
        return mix(base, vec3<f32>(1.0, 0.12, 0.06), 0.4) * stripe;
    }
    if ownership == 3u {
        return select(
            vec3<f32>(0.12, 0.45, 1.0),
            vec3<f32>(1.0, 0.12, 0.08),
            fract((local.x + local.y) * 8.0) < 0.5,
        );
    }
    return mix(base, vec3<f32>(0.65, 0.65, 0.65), 0.45);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let x: u32 = min(u32(in.uv.x * f32(width)), width - 1u);
    let y: u32 = min(u32(in.uv.y * f32(height)), height - 1u);
    let value = zone_map[y * width + x];
    let gather_bit = present(value, 0u);
    let build_bit = present(value, 1u);
    let defend_bit = present(value, 2u);
    let corridor_bit = present(value, 3u);
    let layer_count = gather_bit + build_bit + defend_bit + corridor_bit;
    let local = fract(vec2<f32>(in.uv.x * f32(width), in.uv.y * f32(height)));
    let color_sum =
        owned_color(vec3<f32>(1.0, 0.0, 0.0), owner(value, 0u), local) * gather_bit
        + owned_color(vec3<f32>(1.0, 0.0, 1.0), owner(value, 1u), local) * build_bit
        + owned_color(vec3<f32>(0.0, 0.0, 1.0), owner(value, 2u), local) * defend_bit
        + owned_color(vec3<f32>(1.0, 1.0, 0.0), owner(value, 3u), local) * corridor_bit;
    let color = color_sum / max(layer_count, 1.0);
    let alpha = min(layer_count, 1.0) * 0.8;
    return vec4<f32>(color, alpha);
}
