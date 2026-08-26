struct Globals {
    camera_pos: vec4<f32>,
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    light_view_proj: mat4x4<f32>,
    light_pos: vec4<f32>,
    light_color: vec4<f32>,
    /// Closest local point light. `w` is radius; 0 disables it.
    local_light_pos: vec4<f32>,
    local_light_color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u_Globals: Globals;

fn closest_local_light(pos: vec3<f32>, normal: vec3<f32>) -> f32 {
    let pl = u_Globals.local_light_pos;
    if (pl.w <= 0.001) {
        return 0.0;
    }
    let to_l = pl.xyz - pos;
    let dist = length(to_l);
    let atten = clamp(1.0 - dist / pl.w, 0.0, 1.0);
    return atten * atten * max(0.0, dot(normal, to_l / max(dist, 0.001)));
}
