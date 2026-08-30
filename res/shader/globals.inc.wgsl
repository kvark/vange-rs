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
    /// Vehicle we keep visible through intervening terrain. `xyz` is the
    /// body, `w` is its radius; 0 disables the hole.
    focus_pos: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u_Globals: Globals;

/// Opacity of terrain that sits between the camera and the vehicle.
/// 1 = fully solid, ~0.2 on the view axis (see the car through a veil).
fn focus_visibility(pos: vec3<f32>) -> f32 {
    let f = u_Globals.focus_pos;
    if (f.w <= 0.001) {
        return 1.0;
    }
    let cam = u_Globals.camera_pos.xyz;
    let car = f.xyz;
    let to_car = car - cam;
    let dist_car = length(to_car);
    if (dist_car <= f.w) {
        return 1.0;
    }
    let to_pos = pos - cam;
    let dist_pos = length(to_pos);
    if (dist_pos >= dist_car - f.w * 0.4) {
        return 1.0;
    }
    let axis = to_car / dist_car;
    let along = dot(to_pos, axis);
    if (along <= 0.0) {
        return 1.0;
    }
    let perp = length(to_pos - axis * along);
    let cone_r = f.w * (along / dist_car) * 2.6;
    let edge = smoothstep(0.0, cone_r, perp);
    return mix(0.18, 1.0, edge * edge);
}

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
