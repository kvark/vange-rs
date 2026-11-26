// Common FS routines for evaluating terrain color.
//
// Dynamic lighting, shared by every terrain method (paint, ray, mesh,
// slice, scatter, voxel). Brightness comes from a 3D n·L of the sun
// (and any local light) against a height-field normal, then indexes
// the per-terrain palette ramp — the same BEGCOLOR..ENDCOLOR mapping
// as `RenderPrepare` / `LINE_render` in KranX/Vangers `src/terra/land.cpp`.
//
// This is the 3D generalization of the original `evaluate_light` table,
// which was a 2D n·L in the (sun-azimuth, Z) plane with a fixed elevation
// `atan(c_ShadowDepthScale)`. When the sun is directional at
// `settings::PALETTE_MATCH_LIGHT_POS` — `(1, 2, 0.6*length(1,2), 0)` —
// slopes along the sun's azimuth reproduce that table exactly (land
// material). Moving the sun's elevation or azimuth changes the lighting
// for real; the old baked path ignored the sun's Z and the old unbaked
// path used only the brightest ramp entry as albedo.

// Terrain parameters per type: shadow offset, height shift, palette start, palette end
@group(1) @binding(5) var t_Table: texture_2d<u32>;
// corresponds to SDL palette
@group(1) @binding(6) var t_Palette: texture_2d<f32>;

@group(0) @binding(1) var s_Palette: sampler;

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

fn evaluate_palette(ty: u32, value_in: f32) -> f32 {
    var value = clamp(value_in, 0.0, 1.0);
    let terr = vec4<f32>(textureLoad(t_Table, vec2<i32>(i32(ty), 0), 0));
    //Note: the original game had specific logic here to process water
    return (mix(terr.z, terr.w, value) + 0.5) / 256.0;
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

/// Dynamic n·L → palette-ramp brightness.
///
/// The original table was `dot(N, L) / (|N||L|)` in the (sun-azimuth, Z)
/// plane, with `N = (dx, -jj)` and a fixed `L = (sd, 1)`. In 3D that is
/// `N = (-slope.xy, dx)` against the actual sun vector. Local lights
/// can only raise the term, the horizon factor darkens low ground,
/// and shadows compress the index toward the dark end of the ramp.
fn ramp_lit(ty: u32, pos: vec3<f32>, slope: vec2<f32>, shadow_visibility: f32) -> f32 {
    let material = terrain_material(ty);
    let dx = material.x * c_DiffuseScale;
    var n = vec3<f32>(-slope.x, -slope.y, dx);
    let n_len = length(n);
    n = n / max(n_len, 1e-5);
    let light = u_Globals.light_pos.xyz - pos * u_Globals.light_pos.w;
    let light_len = length(light);
    let sun = select(0.0, max(0.0, dot(n, light / light_len)), light_len > 1e-5);
    let n_dot_l = max(sun, closest_local_light(pos, n));
    let height_term = c_HorFactor * (1.0 - pos.z / u_Surface.texture_scale.z);
    let lit = clamp(n_dot_l - height_term, 0.0, 1.0);
    let lit_factor = mix(c_TerrainAmbient, 1.0, shadow_visibility);
    return lit_factor * lit;
}

fn sample_ramp(ty: u32, lit: f32) -> vec4<f32> {
    let color_id = evaluate_palette(ty, lit);
    return textureSampleLevel(t_Palette, s_Palette, vec2<f32>(color_id, 0.5), 0.0);
}

fn evaluate_color_normal(
    ty: u32,
    pos: vec3<f32>,
    shadow_visibility: f32,
    normal: vec3<f32>,
) -> vec4<f32> {
    let slope = palette_slope_from_normal(ty, normal);
    return sample_ramp(ty, ramp_lit(ty, pos, slope, shadow_visibility));
}

fn evaluate_color(ty: u32, pos: vec3<f32>, shadow_visibility: f32) -> vec4<f32> {
    let slope = palette_slope(ty, get_surface_gradient(pos));
    return sample_ramp(ty, ramp_lit(ty, pos, slope, shadow_visibility));
}
