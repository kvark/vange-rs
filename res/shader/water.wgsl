//!include globals.inc surface.inc shadow.inc

// Flood map has the water level per Y.
@group(1) @binding(4) var t_Flood: texture_1d<f32>;
@group(1) @binding(8) var s_Flood: sampler;

let c_TerrainWater = 0u;
let c_WaterColor = vec3<f32>(0.0, 0.1, 0.4);

struct Varyings {
    @builtin(position) clip_pos: vec4<f32>;
    @location(0) world_pos: vec3<f32>;
};

@stage(vertex)
fn main_vs(@location(0) pos: vec2<f32>, @location(1) flood_id: i32) -> Varyings {
    let z = textureLoad(t_Flood, flood_id, 0).x * u_Surface.texture_scale.z;
    let clip_pos = u_Globals.view_proj * vec4<f32>(pos, z, 1.0);
    return Varyings( clip_pos, vec3<f32>(pos, z) );
}

@stage(fragment)
fn main_fs(in: Varyings) -> @location(0) vec4<f32> {
    let surface = get_surface(in.world_pos.xy);
    if (surface.low_type != c_TerrainWater) {
        return vec4<f32>(0.0);
    }

    let shadow = fetch_shadow(in.world_pos);

    let view = normalize(in.world_pos - u_Globals.camera_pos.xyz);
    //TODO: screen-space reflections
    //TODO: read the depth texture to find out actual transparency
    return vec4<f32>(shadow * c_WaterColor, 1.0 + 0.9*view.z);
}
