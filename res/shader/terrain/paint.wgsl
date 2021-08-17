//!include globals.inc terrain/locals.inc surface.inc shadow.inc color.inc

fn generate_paint_pos(instance_index: u32) -> vec2<f32> {
    let row_size = u32(ceil(u_Locals.sample_range.y - u_Locals.sample_range.x));
    let rel_x = f32(instance_index % row_size);
    let rel_y = f32(instance_index / row_size);
    let x = select(u_Locals.sample_range.y - rel_x, u_Locals.sample_range.x + rel_x, u_Locals.cam_origin_dir.z > 0.0);
    let y = select(u_Locals.sample_range.w - rel_y, u_Locals.sample_range.z + rel_y, u_Locals.cam_origin_dir.w > 0.0);
    return vec2<f32>(x, y);
}

struct Varyings {
    [[builtin(position)]] position: vec4<f32>;
    [[location(0)]] tex_coord: vec3<f32>;
    [[location(1), interpolate(flat)]] type: u32;
    [[location(2)]] plane_pos: vec3<f32>;
};

[[stage(vertex)]]
fn vertex(
    [[builtin(vertex_index)]] vertex_index: u32,
    [[builtin(instance_index)]] instance_index: u32,
) -> Varyings {
    let pos_center = generate_paint_pos(instance_index);

    let suf = get_surface(pos_center);
    let altitude = select(0.0,
        select(suf.low_alt,
            select(
                suf.low_alt + suf.delta,
                suf.high_alt,
                vertex_index >= 12u,
            ),
            vertex_index >= 8u,
        ),
        vertex_index >= 4u,
    );
    let plane_pos = vec3<f32>(pos_center, altitude);
        
    let cx = select(0.0, 1.0, ((vertex_index + 0u) & 3u) >= 2u);
    let cy = select(0.0, 1.0, ((vertex_index + 1u) & 3u) >= 2u);
    let pos = floor(pos_center) + vec2<f32>(cx, cy);

    let type = select(suf.low_type, suf.high_type, vertex_index >= 8u);
    let tex_coord = vec3<f32>(suf.tex_coord, altitude / u_Surface.texture_scale.z);
    return Varyings(
        u_Globals.view_proj * vec4<f32>(pos, altitude, 1.0),
        tex_coord,
        type,
        plane_pos,
    );
}

//imported: Surface, u_Globals, get_surface, evaluate_color, apply_fog, fetch_shadow

[[stage(fragment)]]
fn fragment(in: Varyings) -> [[location(0)]] vec4<f32> {
    let lit_factor = fetch_shadow(in.plane_pos);
    let terrain_color = evaluate_color(in.type, in.tex_coord.xy, in.tex_coord.z, lit_factor);
    return apply_fog(terrain_color, in.plane_pos.xy);
}
