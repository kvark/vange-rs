// Common routines for fetching the level surface data.

struct SurfaceConstants {
    texture_scale: vec4<f32>,    // XY = size, Z = height scale, w = number of layers
    terrain_bits: vec4<u32>,     // X_low = shift, X_high = mask
};
struct TerrainData {
    inner: array<u32>,
};

@group(1) @binding(0) var<uniform> u_Surface: SurfaceConstants;
@group(1) @binding(2) var<storage, read> b_Terrain: TerrainData;

let c_DoubleLevelMask: u32 = 64u;
let c_ShadowMask: u32 = 128u;
let c_DeltaShift: u32 = 0u;
let c_DeltaBits: u32 = 2u;
let c_DeltaScale: f32 = 0.03137254901; //8.0 / 255.0;

struct Surface {
    low_alt: f32,
    high_alt: f32,
    delta: f32,
    low_type: u32,
    high_type: u32,
    is_shadowed: bool,
};

fn get_terrain_type(meta_data: u32) -> u32 {
    let bits = u_Surface.terrain_bits.x;
    return (meta_data >> (bits & 0xFu)) & (bits >> 4u);
}
fn get_delta(meta_data: u32) -> u32 {
    return (meta_data >> c_DeltaShift) & ((1u << c_DeltaBits) - 1u);
}

fn modulo(a: i32, b: i32) -> i32 {
    let c = a % b;
    return select(c, c+b, c < 0);
}

fn get_lod_height(ipos: vec2<i32>, lod: u32) -> f32 {
    return 0.0; //TODO
}

fn get_map_coordinates(pos: vec2<f32>) -> vec2<i32> {
    return vec2<i32>(pos - floor(pos / u_Surface.texture_scale.xy) * u_Surface.texture_scale.xy);
}

fn get_surface_impl(tci: vec2<i32>) -> Surface {
    var suf: Surface;

    let tc_index = tci.y * i32(u_Surface.texture_scale.x) + tci.x;
    let data_raw = b_Terrain.inner[tc_index / 2];
    let data = (vec4<u32>(data_raw) >> vec4<u32>(0u, 8u, 16u, 24u)) & vec4<u32>(0xFFu);
    suf.is_shadowed = (data.y & c_ShadowMask) != 0u;

    if ((data.y & c_DoubleLevelMask) != 0u) {
        let delta = (get_delta(data.y) << c_DeltaBits) + get_delta(data.w);
        suf.low_type = get_terrain_type(data.y);
        suf.high_type = get_terrain_type(data.w);
        suf.low_alt = f32(data.x) / 255.0 * u_Surface.texture_scale.z;
        suf.high_alt = f32(data.z) / 255.0 * u_Surface.texture_scale.z;
        suf.delta = f32(delta) * c_DeltaScale * u_Surface.texture_scale.z;
    } else {
        let subdata = select(data.xy, data.zw, (tc_index & 1) != 0);
        let altitude = f32(subdata.x) / 255.0 * u_Surface.texture_scale.z;
        let ty = get_terrain_type(subdata.y);
        suf.low_type = ty;
        suf.high_type = ty;
        suf.low_alt = altitude;
        suf.high_alt = altitude;
        suf.delta = 0.0;
    }

    return suf;
}

fn get_surface(pos: vec2<f32>) -> Surface {
    let tci = get_map_coordinates(pos);
    return get_surface_impl(tci);
}

struct SurfaceAlt {
    low: f32,
    high: f32,
    delta: f32,
};

fn get_surface_alt(pos: vec2<f32>) -> SurfaceAlt {
    let surface = get_surface(pos);
    var s: SurfaceAlt;
    s.low = surface.low_alt;
    s.high = surface.high_alt;
    s.delta = surface.delta;
    return s;
}

fn merge_alt(a: SurfaceAlt, b: SurfaceAlt, ratio: f32) -> SurfaceAlt {
    var suf: SurfaceAlt;
    let mid = 0.5 * (b.low + b.high);
    suf.low = mix(a.low, select(b.low, b.high, a.low >= mid), ratio);
    suf.high = mix(a.high, select(b.low, b.high, a.high >= mid), ratio);
    suf.delta = mix(a.delta, select(0.0, b.delta, a.high >= mid), ratio);
    suf = a;
    return suf;
}

fn get_surface_alt_smooth(pos: vec2<f32>) -> SurfaceAlt {
    let tci = get_map_coordinates(pos);
    let sub_pos = fract(pos);
    let offsets = step(vec2<f32>(0.5), sub_pos) * 2.0 - vec2<f32>(1.0);
    let s00 = get_surface_alt(pos);
    let s10 = get_surface_alt(pos + vec2<f32>(offsets.x, 0.0));
    let s01 = get_surface_alt(pos + vec2<f32>(0.0, offsets.y));
    let s11 = get_surface_alt(pos + offsets);

    let s00_10 = merge_alt(s00, s10, abs(sub_pos.x - 0.5));
    let s01_11 = merge_alt(s01, s11, abs(sub_pos.x - 0.5));
    return merge_alt(s00_10, s01_11, abs(sub_pos.y - 0.5));
}

fn get_surface_smooth(pos: vec2<f32>) -> Surface {
    var suf = get_surface(pos);
    let alt = get_surface_alt_smooth(pos);
    suf.low_alt = alt.low;
    suf.high_alt = alt.high;
    suf.delta = alt.delta;
    return suf;
}
