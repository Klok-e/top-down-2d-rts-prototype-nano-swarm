@group(1) @binding(2)
var<storage> zone_map: array<ZonePointData>;
@group(1) @binding(3)
var<uniform> width: u32;
@group(1) @binding(4)
var<uniform> height: u32;
@group(1) @binding(1)
var<uniform> highlight_zone_id: u32;

struct ZonePointData {
    /// First 4 bits are zone color indicators, rest are zone id (14 bits for each)
    zones: u32,
    /// 2 zone id indicators 16 bits each
    bits: u32,
}

@fragment
fn fragment(
    #import bevy_pbr::mesh_vertex_output
) -> @location(0) vec4<f32> {

    // TODO: check performance implications of this constant array
    // Define colors for each zone
    var zone_colors = array(
        vec4<f32>(1.0, 0.0, 0.0, 0.6),  // Zone 0: Red
        vec4<f32>(1.0, 0.0, 1.0, 0.6),  // Zone 1: Magenta
        vec4<f32>(0.0, 0.0, 1.0, 0.6),  // Zone 2: Blue
        vec4<f32>(1.0, 1.0, 0.0, 0.6),  // Zone 3: Yellow
    );

    let x: u32 = u32(uv.x * f32(width));
    let y: u32 = u32(uv.y * f32(height));
    let idx: u32 = y * width + x;
    let zone_data: ZonePointData = zone_map[idx];

    var color_sum = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    var alpha_sum = 0.0;
    var count = 0.0;
    var highlighted = 0.0;
    for(var i = 0u; i < 4u; i = i + 1u) {
        var bit_zone_id: u32;
        if(i < 2u) {
            bit_zone_id = (zone_data.zones >> (4u + 14u * i)) & 0x3FFFu;
        } else {
            bit_zone_id = (zone_data.bits >> (14u * (i - 2u))) & 0x3FFFu;
        }
        if((zone_data.zones & (1u << i)) != 0u) {
            var src_color = zone_colors[i];
            if (bit_zone_id == highlight_zone_id) {
                src_color.a = 0.8;
                highlighted += 1.0;
            }
            color_sum += src_color * src_color.a;
            alpha_sum += src_color.a;
            count += 1.0;
        }
    }

    // if there were any colors, divide by the total alpha
    if(count > 0.0) {
        color_sum /= count;
        if (highlighted > 0.0){
            color_sum.a = 0.8;
        } else {
            color_sum.a = 0.6;
        }
    }

    return color_sum;
}
