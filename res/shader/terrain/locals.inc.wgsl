const c_DepthBias: f32 = 0.01;

struct Locals {
    screen_rect: vec4<u32>,      // XY = offset, ZW = size
    cam_origin_dir: vec4<f32>,   // XY = origin, ZW = dir
    sample_range: vec4<f32>,     // XY = X range, ZW = y range
    fog_color: vec4<f32>,
    fog_params: vec4<f32>,       // X=near, Y = far
};
@group(1) @binding(1) var<uniform> u_Locals: Locals;

fn get_frag_ndc(frag_coord: vec2<f32>, z: f32) -> vec4<f32> {
    let normalized = (frag_coord.xy - vec2<f32>(u_Locals.screen_rect.xy)) / vec2<f32>(u_Locals.screen_rect.zw);
    return vec4<f32>(
        // note the Y-flip here
        (normalized * 2.0 - vec2<f32>(1.0)) * vec2<f32>(1.0, -1.0),
        z,
        1.0,
    );
}

fn get_frag_world(frag_coord: vec2<f32>, z: f32) -> vec3<f32> {
    let ndc = get_frag_ndc(frag_coord, z);
    let homogeneous = u_Globals.inv_view_proj * ndc;
    return homogeneous.xyz / homogeneous.w;
}

fn apply_fog(terrain_color: vec4<f32>, world_pos: vec2<f32>) -> vec4<f32> {
    let cam_distance = clamp(length(world_pos - u_Locals.cam_origin_dir.xy), u_Locals.fog_params.x, u_Locals.fog_params.y);
    let fog_amount = smoothstep(u_Locals.fog_params.x, u_Locals.fog_params.y, cam_distance);
    return mix(terrain_color, u_Locals.fog_color, fog_amount);
}
