const c_DepthBias: f32 = 0.01;

struct Locals {
    screen_rect: vec4<u32>,      // XY = offset, ZW = size
    cam_origin_dir: vec4<f32>,   // XY = origin, ZW = dir
    sample_range: vec4<f32>,     // XY = X range, ZW = y range
    fog_color: vec4<f32>,
    fog_params: vec4<f32>,       // X=near, Y = far
    lighting_flags: vec4<u32>,   // X = 1 sRGB target (write linear), 0 Unorm; Y = 1 smooth mesh normals
    terrain_params: vec4<f32>,   // X = slice spacing, Y = ray forward-step budget
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

// IEC 61966-2-1. Terrain lights in linear; sRGB is only used to lerp
// the two-tone material so mid-ramp hues match the 1998 palette.
fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

// Terrain lighting is in linear. An sRGB target (flags.x = 1) encodes
// on write; a Unorm target gets an explicit encode so the bytes match.
fn encode_output(color: vec4<f32>) -> vec4<f32> {
    if (u_Locals.lighting_flags.x != 0u) {
        return color;
    }
    return vec4<f32>(linear_to_srgb(color.rgb), color.a);
}

fn apply_fog(terrain_color: vec4<f32>, world_pos: vec2<f32>) -> vec4<f32> {
    let cam_distance = clamp(length(world_pos - u_Locals.cam_origin_dir.xy), u_Locals.fog_params.x, u_Locals.fog_params.y);
    let fog_amount = smoothstep(u_Locals.fog_params.x, u_Locals.fog_params.y, cam_distance);
    return encode_output(mix(terrain_color, u_Locals.fog_color, fog_amount));
}
