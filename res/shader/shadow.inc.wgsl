// Shadow sampling.

[[group(0), binding(3)]] var t_Shadow: texture_depth_2d;
[[group(0), binding(4)]] var s_Shadow: sampler_comparison;

let c_Ambient: f32 = 0.25;

fn fetch_shadow(pos: vec3<f32>) -> f32 {
    let flip_correction = vec2<f32>(1.0, -1.0);

    let homogeneous_coords = u_Globals.light_view_proj * vec4<f32>(pos, 1.0);
    if (homogeneous_coords.w <= 0.0) {
        return 0.0;
    }
    let light_local = 0.5 * (homogeneous_coords.xy * flip_correction/homogeneous_coords.w + 1.0);
    let shadow = textureSampleCompareLevel(
        t_Shadow, s_Shadow,
        light_local,
        homogeneous_coords.z / homogeneous_coords.w
    );
    return mix(c_Ambient, 1.0, shadow);
}
