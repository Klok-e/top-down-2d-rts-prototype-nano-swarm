#import bevy_sprite::mesh2d_vertex_output::VertexOutput

@group(2) @binding(2)
var<storage> zone_map: array<ZonePointData>;
@group(2) @binding(3)
var<uniform> width: u32;
@group(2) @binding(4)
var<uniform> height: u32;

struct ZonePointData {
    // Packed paint strength for each kind. Slot i occupies bits [5*i, 5*i+5)
    // and stores the strength in [0, PAINT_STRENGTH_CAP]. Slot 0 = Gather,
    // 1 = Build, 2 = Defend, 3 = Corridor. A slot value of 0 means the kind
    // is absent at this cell.
    strength: u32,
}

// Linear alpha ramp endpoints. Every active zone stays visible (floor), and a
// zone painted to the strength cap reads solid (ceiling). Keep STRENGTH_CAP in
// sync with the simulation contract in `src/intent.rs` (PAINT_STRENGTH_CAP = 16).
const ALPHA_FLOOR: f32 = 0.15;
const ALPHA_CEILING: f32 = 0.8;
const STRENGTH_CAP: f32 = 16.0;
const SLOT_BITS: u32 = 5u;
const SLOT_MASK: u32 = 0x1Fu;

@fragment
fn fragment(
    in: VertexOutput
) -> @location(0) vec4<f32> {

    // Per-kind base colours. Red = Gather, Magenta = Build, Blue = Defend,
    // Yellow = Corridor. Strength controls alpha, not colour.
    var zone_colors = array(
        vec3<f32>(1.0, 0.0, 0.0),  // Zone 0: Red (Gather)
        vec3<f32>(1.0, 0.0, 1.0),  // Zone 1: Magenta (Build)
        vec3<f32>(0.0, 0.0, 1.0),  // Zone 2: Blue (Defend)
        vec3<f32>(1.0, 1.0, 0.0),  // Zone 3: Yellow (Corridor)
    );

    let x: u32 = u32(in.uv.x * f32(width));
    let y: u32 = u32(in.uv.y * f32(height));
    let idx: u32 = y * width + x;
    let zone_data: ZonePointData = zone_map[idx];

    // Overlapping active kinds blend by averaging their premultiplied colour
    // and their alpha. A kind contributes only when its 5-bit slot is > 0;
    // a 0 slot means the kind is absent at this cell.
    var color_sum = vec3<f32>(0.0, 0.0, 0.0);
    var alpha_sum = 0.0;
    var count = 0.0;
    for (var i = 0u; i < 4u; i = i + 1u) {
        let strength = (zone_data.strength >> (SLOT_BITS * i)) & SLOT_MASK;
        if (strength == 0u) {
            continue;
        }
        let alpha = ALPHA_FLOOR + (f32(strength) / STRENGTH_CAP) * (ALPHA_CEILING - ALPHA_FLOOR);
        color_sum += zone_colors[i] * alpha;
        alpha_sum += alpha;
        count += 1.0;
    }

    if (count > 0.0) {
        return vec4<f32>(color_sum / count, alpha_sum / count);
    }
    return vec4<f32>(0.0, 0.0, 0.0, 0.0);
}
