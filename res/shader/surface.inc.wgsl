// Common routines for fetching the level surface data.

[[block]]
struct SurfaceConstants {
    texture_scale: vec4<f32>;    // XY = size, Z = height scale, w = number of layers
};

[[group(1), binding(0)]] var<uniform> u_Surface: SurfaceConstants;

[[group(1), binding(2)]] var t_Height: texture_2d<f32>;
[[group(1), binding(3)]] var t_Meta: texture_2d<u32>;
[[group(1), binding(7)]] var s_Main: sampler;

let c_DoubleLevelMask: u32 = 64u;
let c_ShadowMask: u32 = 128u;
let c_TerrainShift: u32 = 3u;
let c_TerrainBits: u32 = 3u;
let c_DeltaShift: u32 = 0u;
let c_DeltaBits: u32 = 2u;
let c_DeltaScale: f32 = 0.03137254901; //8.0 / 255.0;

struct Surface {
    low_alt: f32;
    high_alt: f32;
    delta: f32;
    low_type: u32;
    high_type: u32;
    tex_coord: vec2<f32>;
    is_shadowed: bool;
};

fn get_terrain_type(meta: u32) -> u32 {
    return (meta >> c_TerrainShift) & ((1u << c_TerrainBits) - 1u);
}
fn get_delta(meta: u32) -> u32 {
    return (meta >> c_DeltaShift) & ((1u << c_DeltaBits) - 1u);
}

fn modulo(a: i32, b: i32) -> i32 {
    let c = a % b;
    return select(c, c+b, c < 0);
}

fn get_lod_height(ipos: vec2<i32>, lod: u32) -> f32 {
    let x = modulo(ipos.x, i32(u_Surface.texture_scale.x));
    let y = modulo(ipos.y, i32(u_Surface.texture_scale.y));
    let tc = vec2<i32>(x, y) >> vec2<u32>(lod);
    let alt = textureLoad(t_Height, tc, i32(lod)).x;
    return alt * u_Surface.texture_scale.z;
}

fn get_surface(pos: vec2<f32>) -> Surface {
    var suf: Surface;

    let tc = pos / u_Surface.texture_scale.xy;
    let tci = vec2<i32>(pos % u_Surface.texture_scale.xy);
    suf.tex_coord = tc;

    let meta = textureLoad(t_Meta, tci, 0).x;
    suf.is_shadowed = (meta & c_ShadowMask) != 0u;
    suf.low_type = get_terrain_type(meta);

    if ((meta & c_DoubleLevelMask) != 0u) {
        //TODO: we need either low or high for the most part
        // so this can be more efficient with a boolean param
        var delta = 0u;
        if (pos.x % 2.0 >= 1.0) {
            let meta_low = textureLoad(t_Meta, tci + vec2<i32>(-1, 0), 0).x;
            suf.high_type = suf.low_type;
            suf.low_type = get_terrain_type(meta_low);
            delta = (get_delta(meta_low) << c_DeltaBits) + get_delta(meta);
        } else {
            let meta_high = textureLoad(t_Meta, tci + vec2<i32>(1, 0), 0).x;
            suf.tex_coord.x = suf.tex_coord.x + 1.0 / u_Surface.texture_scale.x;
            suf.high_type = get_terrain_type(meta_high);
            delta = (get_delta(meta) << c_DeltaBits) + get_delta(meta_high);
        }

        suf.low_alt = //TODO: the `LodOffset` doesn't appear to work in Metal compute
            //textureLodOffset(sampler2D(t_Height, s_Main), suf.tex_coord, 0.0, ivec2(-1, 0)).x
            textureSampleLevel(t_Height, s_Main, suf.tex_coord - vec2<f32>(1.0 / u_Surface.texture_scale.x, 0.0), 0.0).x
            * u_Surface.texture_scale.z;
        suf.high_alt = textureSampleLevel(t_Height, s_Main, suf.tex_coord, 0.0).x * u_Surface.texture_scale.z;
        suf.delta = f32(delta) * c_DeltaScale * u_Surface.texture_scale.z;
    } else {
        suf.high_type = suf.low_type;

        suf.low_alt = textureSampleLevel(t_Height, s_Main, tc, 0.0).x * u_Surface.texture_scale.z;
        suf.high_alt = suf.low_alt;
        suf.delta = 0.0;
    }

    return suf;
}
