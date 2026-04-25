// Shadow sampling.

@group(0) @binding(3) var t_Shadow: texture_depth_2d;
@group(0) @binding(4) var s_Shadow: sampler_comparison;

const c_Ambient: f32 = 0.25;

fn fetch_shadow(pos: vec3<f32>) -> f32 {
    let flip_correction = vec2<f32>(1.0, -1.0);

    if (u_Globals.light_view_proj[3][3] == 0.0) {
        // shadow is disabled
        return 1.0;
    }
    let homogeneous_coords = u_Globals.light_view_proj * vec4<f32>(pos, 1.0);
    if (homogeneous_coords.w <= 0.0) {
        // outside of shadow projection
        return 0.0;
    }

    let light_local = 0.5 * (homogeneous_coords.xy * flip_correction / homogeneous_coords.w + 1.0);
    let depth = homogeneous_coords.z / homogeneous_coords.w;
    // 4-tap PCF: sample at the four corners of the texel and average.
    // The hardware comparison sampler does linear filtering between the
    // depth comparisons inside each tap, so the resulting kernel is
    // effectively a 3×3 box blur for the cost of 4 samples — a clear
    // step up from the previous single-tap aliased shadow edges.
    let texel = vec2<f32>(1.0) / vec2<f32>(textureDimensions(t_Shadow));
    var shadow = 0.0;
    shadow += textureSampleCompareLevel(t_Shadow, s_Shadow, light_local + texel * vec2<f32>(-0.5, -0.5), depth);
    shadow += textureSampleCompareLevel(t_Shadow, s_Shadow, light_local + texel * vec2<f32>( 0.5, -0.5), depth);
    shadow += textureSampleCompareLevel(t_Shadow, s_Shadow, light_local + texel * vec2<f32>(-0.5,  0.5), depth);
    shadow += textureSampleCompareLevel(t_Shadow, s_Shadow, light_local + texel * vec2<f32>( 0.5,  0.5), depth);
    shadow *= 0.25;
    return mix(c_Ambient, 1.0, shadow);
}
