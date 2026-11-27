// Common FS routines for evaluating terrain color.
//
// Pure linear-RGB dynamic lighting, shared by every terrain method
// (paint, ray, mesh, slice, scatter, voxel). Per-terrain shadow and
// lit colors are fitted at load (and whenever the world's palette
// animates) from each type's BEGCOLOR..=ENDCOLOR ramp in harmony.pal.
// The shader lerps those two tones in sRGB — so Fostral's dark
// olive-to-ochre hue shift stays — then decodes to linear and scales
// the added energy by the real sun / local-light colors. There is no
// runtime t_Palette index.
//
// Brightness is the same 3D n·L as before, against a height-field
// normal, with the horizon term and the 0.25 shadow floor. At
// settings::PALETTE_MATCH_LIGHT_POS — (1, 2, 0.6*length(1,2), 0) —
// slopes along the sun's azimuth match the original evaluate_light
// table, and the two-tone lerp reproduces the 1998 ramp under a
// white sun. Moving the sun or tinting light_color / local_light_color
// changes the lighting in RGB.

// Per-terrain two-tone material: row 0 = shadow sRGB, row 1 = lit sRGB.
// Stored as unorm (not sRGB) so the lerp happens in sRGB before we decode.
@group(1) @binding(5) var t_Material: texture_2d<f32>;

const c_HorFactor: f32 = 0.5; //H_CORRECTION
const c_DiffuseScale: f32 = 8.0;
const c_ShadowDepthScale: f32 = 0.6; //~ 2.0 / 3.0;
// Floor brightness applied to terrain in deep shadow. Matches the
// `c_Ambient` used by `fetch_shadow` so unobstructed surfaces and
// fully-shadowed ones meet at the same darkness when the cosine term
// is zero. Defined here so every terrain method uses the same floor.
const c_TerrainAmbient: f32 = 0.25;

// material coefficients are called "dx", "sd" and "jj" in the original
fn terrain_material(ty: u32) -> vec3<f32> {
    return select(vec3<f32>(1.0), vec3<f32>(5.0, 1.25, 0.5), ty == 0u);
}

fn get_surface_height(pos: vec3<f32>) -> f32 {
    // Smooth altitude lookup gives a bilinearly-interpolated height
    // sample, which feeds the surface gradient + horizon factor in
    // `evaluate_color`. Using the cell-quantised `get_surface`
    // produced visible block edges in the lighting; the smooth
    // variant costs 4× the texture samples but only on the four
    // gradient taps.
    let alt = get_surface_alt_smooth(pos.xy);
    return select(alt.low, alt.high, pos.z >= alt.mid);
}

fn get_surface_gradient(pos: vec3<f32>) -> vec2<f32> {
    if (pos.z <= 0.0) {
        return vec2<f32>(0.0);
    }
    let vl = get_surface_height(pos + vec3<f32>(-1.0, 0.0, 0.0));
    let vr = get_surface_height(pos + vec3<f32>(1.0, 0.0, 0.0));
    let vt = get_surface_height(pos + vec3<f32>(0.0, 1.0, 0.0));
    let vb = get_surface_height(pos + vec3<f32>(0.0, -1.0, 0.0));
    return vec2<f32>(vr - vl, vt - vb);
}

/// Slope in the same units as the original `jj` term: material.z *
/// height gradient / height-scale * 256. The 2-cell gradient is the
/// same one `get_surface_gradient` always measured.
fn palette_slope(ty: u32, gradient: vec2<f32>) -> vec2<f32> {
    let material = terrain_material(ty);
    return material.z * gradient / u_Surface.texture_scale.z * 256.0;
}

/// Recover that slope from a geometric normal, as used by the mesh
/// path. `get_surface_gradient` measures Δheight over Δxy = 2, so
/// dz/dxy = -nx/nz is scaled back by 2.
fn palette_slope_from_normal(ty: u32, normal: vec3<f32>) -> vec2<f32> {
    let nz = select(normal.z, 0.001, abs(normal.z) < 0.001);
    return palette_slope(ty, -2.0 * normal.xy / nz);
}

/// Dynamic n·L shade term in 0..1, same units the 1998 ramp was indexed with.
fn shade_term(ty: u32, pos: vec3<f32>, slope: vec2<f32>, shadow_visibility: f32) -> f32 {
    let material = terrain_material(ty);
    let dx = material.x * c_DiffuseScale;
    var n = vec3<f32>(-slope.x, -slope.y, dx);
    let n_len = length(n);
    n = n / max(n_len, 1e-5);
    let light = u_Globals.light_pos.xyz - pos * u_Globals.light_pos.w;
    let light_len = length(light);
    let sun = select(0.0, max(0.0, dot(n, light / light_len)), light_len > 1e-5);
    let height_term = c_HorFactor * (1.0 - pos.z / u_Surface.texture_scale.z);
    let lit = clamp(sun - height_term, 0.0, 1.0);
    let lit_factor = mix(c_TerrainAmbient, 1.0, shadow_visibility);
    return lit_factor * lit;
}

fn surface_normal(ty: u32, slope: vec2<f32>) -> vec3<f32> {
    let material = terrain_material(ty);
    let dx = material.x * c_DiffuseScale;
    var n = vec3<f32>(-slope.x, -slope.y, dx);
    return n / max(length(n), 1e-5);
}

fn shade_rgb(ty: u32, pos: vec3<f32>, slope: vec2<f32>, shadow_visibility: f32) -> vec4<f32> {
    let t = shade_term(ty, pos, slope, shadow_visibility);
    let ty_i = i32(ty);
    let shadow_srgb = textureLoad(t_Material, vec2<i32>(ty_i, 0), 0).xyz;
    let lit_srgb = textureLoad(t_Material, vec2<i32>(ty_i, 1), 0).xyz;
    let base = srgb_to_linear(mix(shadow_srgb, lit_srgb, t));
    let dark = srgb_to_linear(shadow_srgb);
    let albedo = srgb_to_linear(lit_srgb) - dark;
    let n = surface_normal(ty, slope);
    // White sun: dark + (base - dark) = base, matching the 1998 lerp.
    // A colored sun tints only the added energy, in linear RGB.
    var color = dark + (base - dark) * u_Globals.light_color.xyz;
    color += albedo * closest_local_light(pos, n) * u_Globals.local_light_color.xyz;
    return vec4<f32>(color, 1.0);
}

fn evaluate_color_normal(
    ty: u32,
    pos: vec3<f32>,
    shadow_visibility: f32,
    normal: vec3<f32>,
) -> vec4<f32> {
    let slope = palette_slope_from_normal(ty, normal);
    return shade_rgb(ty, pos, slope, shadow_visibility);
}

fn evaluate_color(ty: u32, pos: vec3<f32>, shadow_visibility: f32) -> vec4<f32> {
    let slope = palette_slope(ty, get_surface_gradient(pos));
    return shade_rgb(ty, pos, slope, shadow_visibility);
}
