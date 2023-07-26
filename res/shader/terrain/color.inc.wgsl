// Common FS routines for evaluating terrain color.

// Terrain parameters per type: shadow offset, height shift, palette start, palette end
@group(1) @binding(5) var t_Table: texture_1d<u32>;
// corresponds to SDL palette
@group(1) @binding(6) var t_Palette: texture_1d<f32>;

@group(0) @binding(1) var s_Palette: sampler;

const c_HorFactor: f32 = 0.5; //H_CORRECTION
const c_DiffuseScale: f32 = 8.0;
const c_ShadowDepthScale: f32 = 0.6; //~ 2.0 / 3.0;

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
    let terr = vec4<f32>(textureLoad(t_Table, i32(ty), 0));
    //Note: the original game had specific logic here to process water
    return (mix(terr.z, terr.w, value) + 0.5) / 256.0;
}

fn get_surface_height(pos: vec3<f32>) -> f32 {
    let suf = get_surface(pos.xy);
    return select(suf.low_alt, suf.high_alt, pos.z >= suf.mid_alt);
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

fn evaluate_color(ty: u32, pos: vec3<f32>, lit_factor: f32) -> vec4<f32> {
    let color_id = evaluate_color_id(ty, pos, lit_factor);
    return textureSample(t_Palette, s_Palette, color_id);
}
