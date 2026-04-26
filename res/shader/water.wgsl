//!include globals.inc surface.inc shadow.inc

// Flood map has the water level per Y.
@group(1) @binding(4) var t_Flood: texture_2d<f32>;
@group(1) @binding(8) var s_Flood: sampler;

const c_TerrainWater = 0u;
const c_WaterColor = vec3<f32>(0.0, 0.1, 0.4);

struct Varyings {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
};

@vertex
fn main_vs(@location(0) pos: vec2<f32>, @location(1) flood_id: i32) -> Varyings {
    let z = textureLoad(t_Flood, vec2<i32>(flood_id, 0), 0).x * u_Surface.texture_scale.z;
    let clip_pos = u_Globals.view_proj * vec4<f32>(pos, z, 1.0);
    return Varyings( clip_pos, vec3<f32>(pos, z) );
}

@fragment
fn main_fs(in: Varyings) -> @location(0) vec4<f32> {
    let surface = get_surface(in.world_pos.xy);
    if (surface.low_type != c_TerrainWater) {
        return vec4<f32>(0.0);
    }

    let shadow = fetch_shadow(in.world_pos);

    let view = normalize(in.world_pos - u_Globals.camera_pos.xyz);
    //TODO: screen-space reflections
    //TODO: read the depth texture to find out actual transparency
    // Reduced base opacity (1.0 → 0.7) so the underwater terrain tint
    // from `apply_underwater` actually shows through the water quad at
    // intermediate viewing angles. Top-down stays mostly transparent;
    // horizontal stays semi-opaque but no longer fully blocks the
    // underwater colour.
    return vec4<f32>(shadow * c_WaterColor, 0.7 + 0.7 * view.z);
}
