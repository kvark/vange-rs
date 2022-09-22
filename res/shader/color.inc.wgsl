// Common FS routines for evaluating terrain color.

// Terrain parameters per type: shadow offset, height shift, palette start, palette end
@group(1) @binding(5) var t_Table: texture_1d<u32>;
// corresponds to SDL palette
@group(1) @binding(6) var t_Palette: texture_1d<f32>;

@group(0) @binding(1) var s_Palette: sampler;

let c_HorFactor: f32 = 0.5; //H_CORRECTION
let c_DiffuseScale: f32 = 8.0;
let c_ShadowDepthScale: f32 = 0.6; //~ 2.0 / 3.0;

// see `RenderPrepare` in `land.cpp` for the original game logic

// material coefficients are called "dx", "sd" and "jj" in the original
fn evaluate_light(material: vec3<f32>, height_diff: f32) -> f32 {
    let dx = material.x * c_DiffuseScale;
    let sd = material.y * c_ShadowDepthScale;
    let jj = material.z * height_diff * 256.0;
    let v = (dx * sd - jj) / sqrt((1.0 + sd * sd) * (dx * dx + jj * jj));
    return clamp(v, 0.0, 1.0);
}

fn evaluate_palette(ty: u32, value_in: f32, ycoord: f32) -> f32 {
    var value = clamp(value_in, 0.0, 1.0);
    let terr = vec4<f32>(textureLoad(t_Table, i32(ty), 0));
    //Note: the original game had specific logic here to process water
    return (mix(terr.z, terr.w, value) + 0.5) / 256.0;
}

fn evaluate_color_id(ty: u32, tex_coord: vec2<f32>, height_normalized: f32, lit_factor: f32) -> f32 {
    // See the original code in "land.cpp": `LINE_render()`
    //Note: we could have a different code path for double level here
    let diff = 0.0; //TODO
        //textureSampleLevel(t_Height, s_Main, tex_coord, 0.0, vec2<i32>(0, 0)).x -
        //textureSampleLevel(t_Height, s_Main, tex_coord, 0.0, vec2<i32>(-2, 0)).x;
    // See the original code in "land.cpp": `TERRAIN_MATERIAL` etc
    let material = select(vec3<f32>(1.0), vec3<f32>(5.0, 1.25, 0.5), ty == 0u);
    let light_clr = evaluate_light(material, diff);
    let tmp = light_clr - c_HorFactor * (1.0 - height_normalized);
    return evaluate_palette(ty, lit_factor * tmp, tex_coord.y);
}

fn evaluate_color(ty: u32, tex_coord: vec2<f32>, height_normalized: f32, lit_factor: f32) -> vec4<f32> {
    let color_id = evaluate_color_id(ty, tex_coord, height_normalized, lit_factor);
    return textureSample(t_Palette, s_Palette, color_id);
}
