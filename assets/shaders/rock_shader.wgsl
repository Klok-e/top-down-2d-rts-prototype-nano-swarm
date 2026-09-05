#import bevy_sprite::mesh2d_vertex_output::VertexOutput
@group(2) @binding(0) var<uniform> tint: vec4<f32>;

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1,311.7))) * 43758.5453);
}
fn noise(p: vec2<f32>) -> f32 {
    let cell = floor(p);
    let f = fract(p);
    let u = f*f*(3.0-2.0*f);
    return mix(mix(hash(cell),hash(cell+vec2<f32>(1.0,0.0)),u.x),
        mix(hash(cell+vec2<f32>(0.0,1.0)),hash(cell+vec2<f32>(1.0,1.0)),u.x),u.y);
}
fn fractures(p: vec2<f32>) -> f32 {
    let cell = floor(p);
    var first = 10.0;
    var second = 10.0;
    for (var y = -1; y <= 1; y += 1) {
        for (var x = -1; x <= 1; x += 1) {
            let c = cell + vec2<f32>(f32(x),f32(y));
            let seed = c + vec2<f32>(hash(c),hash(c+17.0));
            let d = distance(p,seed);
            if d < first { second = first; first = d; }
            else { second = min(second,d); }
        }
    }
    return 1.0-smoothstep(0.004,0.022,second-first);
}
@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let p = in.world_position.xy;
    let pixel = max(length(dpdx(p)),length(dpdy(p)));
    let fine = 1.0-smoothstep(2.0,9.0,pixel);
    let cracks = fractures(p/750.0) * (1.0-smoothstep(8.0,24.0,pixel));
    let variation = (noise(p/950.0)-0.5)*0.14 + (noise(p/8.0)-0.5)*0.035*fine;
    var color = vec4<f32>(0.12,0.16,0.21,1.0);
#ifdef VERTEX_COLORS
    color = in.color;
#endif
    return vec4<f32>(color.rgb * (1.0+variation-cracks*0.22),color.a) * tint;
}
