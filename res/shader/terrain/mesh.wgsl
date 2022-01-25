//!include globals.inc surface.inc color.inc

struct Varyings {
    [[location(0)]] vpos: vec4<f32>;
    [[builtin(position)]] proj_pos: vec4<f32>;
};

[[stage(vertex)]]
fn main_vs(
    [[location(0)]] pos: vec2<f32>,
) -> Varyings {
    let surface = get_surface(pos);
    let vpos = vec4<f32>(pos, surface.low_alt, 1.0);
    return Varyings(vpos, u_Globals.view_proj * vpos);
}


//imported: Surface, u_TextureScale, get_surface, evaluate_color

[[stage(fragment)]]
fn main_fs(in: Varyings) -> [[location(0)]] vec4<f32> {
    let surface = get_surface(in.vpos.xy);
    let ty = surface.low_type;
    let lit_factor = 1.0;
    return evaluate_color(ty, surface.tex_coord, in.vpos.z / u_Surface.texture_scale.z, lit_factor);
}
