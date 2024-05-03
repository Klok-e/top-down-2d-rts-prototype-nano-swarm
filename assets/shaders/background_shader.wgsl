#import bevy_pbr::forward_io::VertexOutput

@fragment
fn fragment(
    in: VertexOutput
) -> @location(0) vec4<f32> {
    let tile_count: f32 = 0.01; // increase to add more tiles
    let sum: f32 = floor(in.world_position.x * tile_count) + floor(in.world_position.y * tile_count);
    let tiles: f32 = sum - floor(sum * 0.5) * 2.0; // manual mod 2 operation

    // Color based on tiles (dark green when tiles = 0, green when tiles = 1)
    var color: vec3<f32>;
    if (tiles > 0.5) {
        color = vec3<f32>(0.0, 1.0, 0.0); // green
    } else {
        color = vec3<f32>(0.0, 0.5, 0.0); // dark green
    }

    return vec4<f32>(color, 1.0);

}
