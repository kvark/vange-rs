//!include globals.inc shadow.inc

struct Locals {
    height_scale: f32;
};
[[group(1), binding(0)]] var<uniform> u_Locals: Locals;
// Flood map has the water level per Y.
[[group(1), binding(1)]] var t_Flood: texture_1d<f32>;

struct Varyings {
    [[builtin(position)]] clip_pos: vec4<f32>;
    [[location(0)]] world_pos: vec3<f32>;
};

[[stage(vertex)]]
fn main_vs([[location(0)]] pos: vec2<f32>, [[location(1)]] flood_id: i32) -> Varyings {
    let z = textureLoad(t_Flood, flood_id, 0).x * u_Locals.height_scale;
    let clip_pos = u_Globals.view_proj * vec4<f32>(pos, z, 1.0);
    return Varyings( clip_pos, vec3<f32>(pos, z) );
}

[[stage(fragment)]]
fn main_fs(in: Varyings) -> [[location(0)]] vec4<f32> {
    return vec4<f32>(0.0, 0.0, 1.0, 0.5);
}
