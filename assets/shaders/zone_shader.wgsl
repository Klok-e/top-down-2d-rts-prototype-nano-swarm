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

fn player_present(value: u32, kind: u32) -> f32 {
    return present(value, kind) * f32(owner(value, kind) & 1u);
}

fn opponent_present(value: u32, kind: u32) -> f32 {
    return present(value, kind) * f32((owner(value, kind) >> 1u) & 1u);
}

fn intent_color(layers: vec4<f32>) -> vec3<f32> {
    let count = dot(layers, vec4<f32>(1.0));
    return (vec3<f32>(1.0, 0.0, 0.0) * layers.x
        + vec3<f32>(1.0, 0.0, 1.0) * layers.y
        + vec3<f32>(0.0, 0.0, 1.0) * layers.z
        + vec3<f32>(1.0, 1.0, 0.0) * layers.w) / max(count, 1.0);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let x: u32 = min(u32(in.uv.x * f32(width)), width - 1u);
    let y: u32 = min(u32(in.uv.y * f32(height)), height - 1u);
    let value = zone_map[y * width + x];
    let player = vec4<f32>(player_present(value, 0u), player_present(value, 1u),
        player_present(value, 2u), player_present(value, 3u));
    let opponent = vec4<f32>(opponent_present(value, 0u), opponent_present(value, 1u),
        opponent_present(value, 2u), opponent_present(value, 3u));
    let has_player = dot(player, vec4<f32>(1.0)) > 0.0;
    let has_opponent = dot(opponent, vec4<f32>(1.0)) > 0.0;
    let local = fract(vec2<f32>(in.uv.x * f32(width), in.uv.y * f32(height)));
    let hatch = fract((local.x + local.y) * 8.0) < 0.5;
    var color = intent_color(player);
    if has_player && has_opponent {
        color = select(color, vec3<f32>(1.0, 0.12, 0.08), hatch);
    } else if has_opponent {
        color = mix(intent_color(opponent), vec3<f32>(1.0, 0.12, 0.06), 0.4)
            * select(0.5, 0.9, hatch);
    }
    let alpha = select(0.0, 0.8, has_player || has_opponent);
    return vec4<f32>(color, alpha);
}
