#import bevy_sprite::mesh2d_vertex_output::VertexOutput

@group(2) @binding(0) var<uniform> appearance: vec4<f32>;

fn crystal(p: vec2<f32>, center: vec2<f32>, size: vec2<f32>, tilt: f32) -> vec4<f32> {
    let offset = p - center;
    let q = vec2<f32>(offset.x + offset.y * tilt, offset.y) / max(size, vec2<f32>(0.0001));
    let edge = abs(q.x) * 0.72 + abs(q.y);
    let mask = 1.0 - smoothstep(1.0 - fwidth(edge), 1.0 + fwidth(edge), edge);
    let facet = select(0.36, 0.85, q.x < 0.0) + select(0.0, 0.20, q.y < -0.25);
    let seam = (1.0 - smoothstep(0.015, 0.06, abs(q.x))) * smoothstep(10.0, 35.0, appearance.y);
    let light = mix(0.35, 1.0, sqrt(appearance.x));
    return vec4<f32>((vec3<f32>(0.025, 0.63, 0.79) * facet + vec3<f32>(0.27, 0.62, 0.64) * seam) * light, mask);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let p = (in.uv - 0.5) * 2.0;
    let distance = length(p);
    let aa = max(fwidth(distance), 0.001);
    let footprint = 1.0 - smoothstep(1.0 - aa, 1.0, distance);
    let bevel = smoothstep(0.80, 0.97, distance);
    let facing = dot(normalize(p + vec2<f32>(0.0001)), normalize(vec2<f32>(-1.0, -1.0)));
    var color = vec3<f32>(0.075, 0.115, 0.14) + bevel * (0.025 + 0.025 * facing);
    let detail = smoothstep(14.0, 45.0, appearance.y);
    let crack = 1.0 - smoothstep(0.008, 0.025, abs(p.y * 0.73 + p.x * 0.28 + 0.11));
    color *= 1.0 - crack * detail * 0.25;
    let growth = sqrt(appearance.x);
    let size = 0.22 + 0.78 * growth;
    var shapes: array<vec4<f32>, 5>;
    shapes[0] = crystal(p, vec2<f32>(-0.37, 0.12) * size, vec2<f32>(0.19, 0.30) * size, 0.28);
    shapes[1] = crystal(p, vec2<f32>(0.30, 0.15) * size, vec2<f32>(0.24, 0.35) * size, -0.30);
    shapes[2] = crystal(p, vec2<f32>(0.03, -0.16) * size, vec2<f32>(0.30, 0.56) * size, 0.08);
    shapes[3] = crystal(p, vec2<f32>(-0.18, 0.42) * size, vec2<f32>(0.18, 0.24) * size, -0.15);
    shapes[4] = crystal(p, vec2<f32>(0.20, 0.43) * size, vec2<f32>(0.15, 0.21) * size, 0.30);
    for (var i = 0u; i < 5u; i += 1u) {
        color = mix(color, shapes[i].rgb, shapes[i].a * select(0.0, 1.0, appearance.x > 0.0));
    }
    return vec4<f32>(color, footprint);
}
