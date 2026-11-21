#import bevy_sprite::mesh2d_vertex_output::VertexOutput

@group(2) @binding(2)
var<storage> zone_map: array<u32>;
@group(2) @binding(3)
var<uniform> width: u32;
@group(2) @binding(4)
var<uniform> height: u32;
@group(2) @binding(5)
var<uniform> palette: array<vec4<f32>, 4>;
@group(2) @binding(6)
var<uniform> spectator: u32;

fn owned_layers(value: u32, side: u32) -> u32 {
    // Collapse one bit from each two-bit owner field into a kind mask.
    let owners = value >> (4u + side);
    let layers = (owners & 1u)
        | ((owners >> 1u) & 2u)
        | ((owners >> 2u) & 4u)
        | ((owners >> 3u) & 8u);
    return layers & value;
}

fn intent_color(layers: u32, local_x: f32) -> vec3<f32> {
    let count = countOneBits(layers);
    var remaining = min(u32(local_x * f32(count)), count - 1u);
    for (var kind = 0u; kind < 4u; kind++) {
        if (layers & (1u << kind)) != 0u {
            if remaining == 0u {
                return palette[kind].rgb;
            }
            remaining -= 1u;
        }
    }
    return vec3<f32>(0.0);
}

fn perimeter(value: u32, x: u32, y: u32, local: vec2<f32>, pixel: vec2<f32>) -> f32 {
    var border = 0.0;
    if local.x < pixel.x * 1.6 && (x == 0u || zone_map[y * width + x - 1u] != value) {
        border = max(border, 1.0 - smoothstep(pixel.x * 0.5, pixel.x * 1.6, local.x));
    }
    if 1.0 - local.x < pixel.x * 1.6 && (x + 1u >= width || zone_map[y * width + x + 1u] != value) {
        border = max(border, 1.0 - smoothstep(pixel.x * 0.5, pixel.x * 1.6, 1.0 - local.x));
    }
    if local.y < pixel.y * 1.6 && (y == 0u || zone_map[(y - 1u) * width + x] != value) {
        border = max(border, 1.0 - smoothstep(pixel.y * 0.5, pixel.y * 1.6, local.y));
    }
    if 1.0 - local.y < pixel.y * 1.6 && (y + 1u >= height || zone_map[(y + 1u) * width + x] != value) {
        border = max(border, 1.0 - smoothstep(pixel.y * 0.5, pixel.y * 1.6, 1.0 - local.y));
    }
    return border;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let grid = in.uv * vec2<f32>(f32(width), f32(height));
    let pixel = max(fwidth(grid), vec2<f32>(0.00001));
    let x = min(u32(grid.x), width - 1u);
    let y = min(u32(grid.y), height - 1u);
    let value = zone_map[y * width + x];
    let first = owned_layers(value, 0u);
    let second = owned_layers(value, 1u);
    if (first | second) == 0u {
        return vec4<f32>(0.0);
    }

    // Each swarm gets its own row when both paint the cell. Divide each row
    // into equal swatches of its active kinds, in palette order.
    let local = clamp(grid - vec2<f32>(f32(x), f32(y)), vec2<f32>(0.0), vec2<f32>(1.0));
    let overlap = first != 0u && second != 0u;
    let use_second = first == 0u || (overlap && local.y >= 0.5);
    let color = intent_color(select(first, second, use_second), local.x);

    var fill = 0.11;
    if use_second || spectator != 0u {
        let direction = select(-1.0, 1.0, use_second);
        let stripe = abs(fract((local.x + local.y * direction) * 4.0) - 0.5);
        let softness = clamp((pixel.x + pixel.y) * 2.0, 0.01, 0.15);
        let hatch = 1.0 - smoothstep(0.21 - softness, 0.21 + softness, stripe);
        fill = mix(0.038, 0.17, hatch);
    }
    let alpha = mix(fill, 0.46, perimeter(value, x, y, local, pixel));
    let divider = select(0.0, 1.0 - smoothstep(pixel.y * 0.55, pixel.y * 1.55, abs(local.y - 0.5)), overlap);
    return vec4<f32>(mix(color, vec3<f32>(0.72), divider), mix(alpha, 0.48, divider));
}
