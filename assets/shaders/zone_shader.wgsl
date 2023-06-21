@group(1) @binding(2)
var<storage> zone_map: array<ZoneMapPointData>;
@group(1) @binding(3)
var<uniform> width: u32;
@group(1) @binding(4)
var<uniform> height: u32;

struct ZoneMapPointData {
    zones: u32,
}

@fragment
fn fragment(
    #import bevy_pbr::mesh_vertex_output
) -> @location(0) vec4<f32> {

    // TODO: check performance implications of this constant array
    // Define colors for each zone
    var zone_colors= array(
        vec4<f32>(1.0, 0.0, 0.0, 0.8),  // Zone 0: Red
        vec4<f32>(0.0, 1.0, 0.0, 0.8),  // Zone 1: Green
        vec4<f32>(0.0, 0.0, 1.0, 0.8),  // Zone 2: Blue
        vec4<f32>(1.0, 1.0, 0.0, 0.8),  // Zone 3: Yellow
        vec4<f32>(1.0, 0.0, 1.0, 0.8),  // Zone 4: Magenta
        vec4<f32>(0.0, 1.0, 1.0, 0.8)   // Zone 5: Cyan
    );

    let x: u32 = u32(uv.x * f32(width));
    let y: u32 = u32(uv.y * f32(height));
    let idx: u32 = y * width + x;
    let zones: u32 = zone_map[idx].zones;

    var final_color = vec4<f32>(0.0, 0.0, 0.0, 0.0);  // start with fully transparent black

    var color: vec3<f32> = vec3<f32>(0.0, 0.0, 0.0);
    for(var i = 0u; i < 6u; i = i + 1u) {
        if((zones & (1u << i)) != 0u) {
            let src_color = zone_colors[i];
            final_color = mix(final_color, src_color, src_color.a);
        }
    }

    return final_color;
}
