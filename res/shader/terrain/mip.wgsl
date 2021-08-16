[[stage(vertex)]]
fn vertex([[location(0)]] pos: vec2<f32>) -> [[builtin(position)]] vec4<f32> {
    return vec4<f32>(2.0 * pos - 1.0, 0.0, 1.0);
}

[[group(0), binding(0)]] var t_Height: texture_2d<f32>;

[[stage(fragment)]]
fn fragment([[builtin(position)]] frag_coord: vec4<f32>) -> [[location(0)]] f32 {
    let tc = vec2<i32>(frag_coord.xy * 2.0);
    let heights = vec4<f32>(
        textureLoad(t_Height, tc - vec2<i32>(0, 0), 0).x,
        textureLoad(t_Height, tc - vec2<i32>(0, 1), 0).x,
        textureLoad(t_Height, tc - vec2<i32>(1, 0), 0).x,
        textureLoad(t_Height, tc - vec2<i32>(1, 1), 0).x
    );
    return max(max(heights.x, heights.y), max(heights.z, heights.w));
}
