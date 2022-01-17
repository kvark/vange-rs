//!include globals.inc surface.inc shadow.inc

struct Varyings {
    [[builtin(position)]] clip_pos: vec4<f32>;
    [[location(0)]] world_pos: vec3<f32>;
};

[[stage(vertex)]]
fn main_vs([[location(0)]] pos: vec2<f32>, [[location(1)]] flood_id: i32) -> Varyings {
    let z = textureLoad(t_Flood, flood_id, 0).x * u_Surface.texture_scale.z;
    let clip_pos = u_Globals.view_proj * vec4<f32>(pos, z, 1.0);
    return Varyings( clip_pos, vec3<f32>(pos, z) );
}

[[stage(fragment)]]
fn main_fs(in: Varyings) -> [[location(0)]] vec4<f32> {
    // cut off the least bit of X coordiante to always point to the low end
    let tci = get_map_coordinates(in.world_pos.xy) & vec2<i32>(-2, -1);
    let meta_low = textureLoad(t_Meta, tci, 0).x;
    let alpha = select(0.0, 0.5, get_terrain_type(meta_low) == 0u);
    return vec4<f32>(0.1, 0.2, 1.0, alpha);
}
