// Common FS routines for evaluating terrain color.

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
// is zero. Defined here because `scatter.wgsl` includes color.inc but
// not shadow.inc, so it can't pull in `c_Ambient`.
const c_TerrainAmbient: f32 = 0.25;

// see `RenderPrepare` in `land.cpp` for the original game logic

// material coefficients are called "dx", "sd" and "jj" in the original
fn evaluate_light(material: vec3<f32>, height_diff: f32) -> f32 {
    let dx = material.x * c_DiffuseScale;
    let sd = material.y * c_ShadowDepthScale;
    let jj = material.z * height_diff * 256.0;
    let v = (dx * sd - jj) / sqrt((1.0 + sd * sd) * (dx * dx + jj * jj));
    return clamp(v, 0.0, 1.0);
}

fn evaluate_palette(ty: u32, value_in: f32) -> f32 {
    var value = clamp(value_in, 0.0, 1.0);
    let terr = vec4<f32>(textureLoad(t_Table, vec2<i32>(i32(ty), 0), 0));
    //Note: the original game had specific logic here to process water
    return (mix(terr.z, terr.w, value) + 0.5) / 256.0;
}

fn get_surface_height(pos: vec3<f32>) -> f32 {
    // Smooth altitude lookup gives a bilinearly-interpolated height
    // sample, which feeds the surface gradient + horizon factor in
    // `evaluate_color_id`. Using the cell-quantised `get_surface`
    // produced visible block edges in the lighting; the smooth
    // variant costs 4× the texture samples but only on the four
    // gradient taps.
    let alt = get_surface_alt_smooth(pos.xy);
    return select(alt.low, alt.high, pos.z >= alt.mid);
}

fn get_surface_gradient(pos: vec3<f32>) -> vec2<f32> {
    let vl = get_surface_height(pos + vec3<f32>(-1.0, 0.0, 0.0));
    let vr = get_surface_height(pos + vec3<f32>(1.0, 0.0, 0.0));
    let vt = get_surface_height(pos + vec3<f32>(0.0, 1.0, 0.0));
    let vb = get_surface_height(pos + vec3<f32>(0.0, -1.0, 0.0));
    return vec2<f32>(vr - vl, vt - vb);
}

fn evaluate_color_id(ty: u32, pos: vec3<f32>, lit_factor: f32) -> f32 {
    // See the original code in "land.cpp": `LINE_render()`
    //Note: the original always used horisontal difference only,
    // presumably because it assumed the sun to be shining from the side.
    // Here, we rely on surface gradient instead.

    let light = u_Globals.light_pos.xyz - pos * u_Globals.light_pos.w;
    let gradient = get_surface_gradient(pos);
    let diff = dot(gradient / u_Surface.texture_scale.z, normalize(light.xy));

    // See the original code in "land.cpp": `TERRAIN_MATERIAL` etc
    let material = select(vec3<f32>(1.0), vec3<f32>(5.0, 1.25, 0.5), ty == 0u);
    let light_clr = evaluate_light(material, diff);
    let tmp = light_clr - c_HorFactor * (1.0 - pos.z / u_Surface.texture_scale.z);
    return evaluate_palette(ty, lit_factor * tmp);
}

// Two lighting paths, switched at runtime via `u_Locals.lighting_flags.x`:
//   0 = baked    — the original Vangers approach: lighting (cell-to-cell
//                  diff, altitude attenuation, shadow) is folded into a
//                  brightness value 0..1, which is then mapped to the
//                  per-terrain palette gradient. Faithful to the 1998
//                  look but the palette gradient itself acts as the
//                  lighting ramp, so our shadow map mostly compresses
//                  the gradient instead of darkening the surface.
//   1 = unbaked  — sample the brightest entry of the terrain's gradient
//                  as the unshaded albedo, then apply shadow * cosine
//                  diffuse explicitly: base * (ambient + (1-ambient) *
//                  visibility * n·l). Real shadow contribution; flat
//                  surfaces no longer get the gradient's mid-tone for
//                  free.
//
// The toggle lives on the terrain "Locals" uniform so it costs one
// branch on a uniform value (no dynamic divergence), and lets us A/B
// the two looks at runtime.
fn evaluate_color(ty: u32, pos: vec3<f32>, shadow_visibility: f32) -> vec4<f32> {
    let terr = vec4<f32>(textureLoad(t_Table, vec2<i32>(i32(ty), 0), 0));
    if (u_Locals.lighting_flags.x == 0u) {
        let lit_factor = mix(c_TerrainAmbient, 1.0, shadow_visibility);
        let color_id = evaluate_color_id(ty, pos, lit_factor);
        return textureSampleLevel(t_Palette, s_Palette, vec2<f32>(color_id, 0.5), 0.0);
    }

    // Brightest gradient entry for this terrain. terr.w is the
    // inclusive max palette index (see `TerrainConfig::colors`), so the
    // texel center sits at (terr.w + 0.5) / 256 — same convention as
    // `evaluate_palette` for value=1.
    let base_id = (terr.w + 0.5) / 256.0;
    let base = textureSampleLevel(t_Palette, s_Palette, vec2<f32>(base_id, 0.5), 0.0);

    // Surface normal from a 2-cell-wide central-difference height
    // gradient. The gradient measures Δheight over Δxy = 2, so
    // dheight/dxy = gradient * 0.5 — flipping signs gives the upward
    // surface normal.
    let gradient = get_surface_gradient(pos);
    let normal = normalize(vec3<f32>(-0.5 * gradient.x, -0.5 * gradient.y, 1.0));
    let light_dir = normalize(u_Globals.light_pos.xyz - pos * u_Globals.light_pos.w);
    let n_dot_l = max(0.0, dot(normal, light_dir));

    let modulation = c_TerrainAmbient + (1.0 - c_TerrainAmbient) * shadow_visibility * n_dot_l;
    return vec4<f32>(base.rgb * modulation, base.a);
}
