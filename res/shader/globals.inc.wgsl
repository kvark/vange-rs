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

/// 4×4 Bayer threshold in 0..1, stable in screen space (Diablo-style dissolve).
fn bayer4(frag_xy: vec2<f32>) -> f32 {
    let x = u32(frag_xy.x) & 3u;
    let y = u32(frag_xy.y) & 3u;
    // Row-major Bayer matrix / 16.
    let i = y * 4u + x;
    var m: array<f32, 16> = array<f32, 16>(
        0.0 / 16.0, 8.0 / 16.0, 2.0 / 16.0, 10.0 / 16.0,
        12.0 / 16.0, 4.0 / 16.0, 14.0 / 16.0, 6.0 / 16.0,
        3.0 / 16.0, 11.0 / 16.0, 1.0 / 16.0, 9.0 / 16.0,
        15.0 / 16.0, 7.0 / 16.0, 13.0 / 16.0, 5.0 / 16.0,
    );
    return m[i];
}

/// How solid terrain should stay at `pos` (1 = opaque, 0 = fully dissolved).
/// Soft along-axis and radial falloffs so the hole eases in/out like Diablo.
fn focus_cover(pos: vec3<f32>) -> f32 {
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
    // Past the car body: never dissolve the ground under/behind the mechos.
    if (dist_pos >= dist_car - f.w * 0.25) {
        return 1.0;
    }
    let axis = to_car / dist_car;
    let along = dot(to_pos, axis);
    if (along <= 0.0) {
        return 1.0;
    }
    let along_n = along / dist_car;
    // Soft ramp from the camera, soft ramp back near the car.
    let along_w = smoothstep(0.02, 0.22, along_n) * (1.0 - smoothstep(0.68, 0.96, along_n));
    let perp = length(to_pos - axis * along);
    let cone_r = max(f.w * 0.4, f.w * along_n * 2.0);
    // 1 on the view axis, 0 outside the cone.
    let radial = 1.0 - smoothstep(cone_r * 0.45, cone_r, perp);
    let dissolve = clamp(along_w * radial, 0.0, 1.0);
    return 1.0 - dissolve;
}

/// Screen opacity: soft cover + Bayer dither band (blend + dissolve).
fn focus_opacity(pos: vec3<f32>, frag_xy: vec2<f32>) -> f32 {
    let cover = focus_cover(pos);
    if (cover >= 0.999) {
        return 1.0;
    }
    if (cover <= 0.001) {
        return 0.0;
    }
    let dither = bayer4(frag_xy);
    // Soft threshold around the Bayer value — pixels dissolve gradually
    // instead of a hard screen-door or a muddy flat veil.
    return smoothstep(dither - 0.14, dither + 0.14, cover);
}

/// Back-compat alias used by older call sites; no dither (prefer focus_opacity).
fn focus_visibility(pos: vec3<f32>) -> f32 {
    return focus_cover(pos);
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
