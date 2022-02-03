//!include globals.inc terrain/locals.inc surface.inc color.inc

struct Varyings {
    @location(0) vpos: vec4<f32>;
    @builtin(position) proj_pos: vec4<f32>;
};

@stage(vertex)
fn main_vs(
    @builtin(vertex_index) vert_index: u32,
    @builtin(instance_index) inst_index: u32,
) -> Varyings {
    let r = u_Locals.sample_range;
    let vpos = vec4<f32>(
        select(r.x, r.y, (vert_index & 1u) != 0u),
        select(r.z, r.w, (vert_index & 2u) != 0u),
        u_Surface.texture_scale.z - f32(inst_index + 1u),
        1.0);
    return Varyings(vpos, u_Globals.view_proj * vpos);
}


//imported: Surface, u_TextureScale, get_surface, evaluate_color

@stage(fragment)
fn main_fs(in: Varyings) -> @location(0) vec4<f32> {
    let surface = get_surface(in.vpos.xy);
    var ty = 0u;
    if (in.vpos.z <= surface.low_alt) {
        ty = surface.low_type;
    } else {
        if (in.vpos.z >= surface.low_alt + surface.delta && in.vpos.z <= surface.high_alt) {
            ty = surface.high_type;
        } else {
            discard;
        };
    }

    let lit_factor = select(0.25, 1.0, in.vpos.z > surface.low_alt || surface.delta == 0.0);
    return evaluate_color(ty, surface.tex_coord, in.vpos.z / u_Surface.texture_scale.z, lit_factor);
}
