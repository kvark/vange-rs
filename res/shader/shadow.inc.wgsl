// Shadow sampling.

@group(0) @binding(3) var t_Shadow: texture_depth_2d;
@group(0) @binding(4) var s_Shadow: sampler_comparison;

const c_Ambient: f32 = 0.25;

// Raw 4-tap PCF visibility in [0, 1]. Unlike `fetch_shadow`, this does
// NOT mix in the ambient floor — the caller composes shadow visibility
// with surface lighting (e.g. cosine diffuse) and adds ambient at the
// end. Used by terrain shading, where the cosine term needs the raw
// occlusion value rather than a pre-mixed brightness.
fn fetch_shadow_visibility(pos: vec3<f32>) -> f32 {
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
    let texel = vec2<f32>(1.0) / vec2<f32>(textureDimensions(t_Shadow));
    var shadow = 0.0;
    shadow += textureSampleCompareLevel(t_Shadow, s_Shadow, light_local + texel * vec2<f32>(-0.5, -0.5), depth);
    shadow += textureSampleCompareLevel(t_Shadow, s_Shadow, light_local + texel * vec2<f32>( 0.5, -0.5), depth);
    shadow += textureSampleCompareLevel(t_Shadow, s_Shadow, light_local + texel * vec2<f32>(-0.5,  0.5), depth);
    shadow += textureSampleCompareLevel(t_Shadow, s_Shadow, light_local + texel * vec2<f32>( 0.5,  0.5), depth);
    // Fade shadows near the edge of the shadow map so they don't
    // cut off abruptly. `fade` goes from 1 (fully shadowed) at the
    // interior to 0 (fully lit) at the boundary.
    let edge = min(light_local, vec2<f32>(1.0) - light_local);
    let fade = saturate(min(edge.x, edge.y) * 10.0);
    return mix(1.0, 0.25 * shadow, fade);
}

// Convenience wrapper: visibility mixed with the ambient floor. Object
// and water shaders use this directly as a brightness multiplier.
fn fetch_shadow(pos: vec3<f32>) -> f32 {
    return mix(c_Ambient, 1.0, fetch_shadow_visibility(pos));
}
