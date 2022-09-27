//!include globals.inc terrain/locals.inc surface.inc shadow.inc terrain/color.inc

fn generate_paint_pos(instance_index: u32) -> vec2<f32> {
    let row_size = u32(ceil(u_Locals.sample_range.y - u_Locals.sample_range.x));
    let rel_x = f32(instance_index % row_size);
    let rel_y = f32(instance_index / row_size);
    let x = select(u_Locals.sample_range.y - rel_x, u_Locals.sample_range.x + rel_x, u_Locals.cam_origin_dir.z > 0.0);
    let y = select(u_Locals.sample_range.w - rel_y, u_Locals.sample_range.z + rel_y, u_Locals.cam_origin_dir.w > 0.0);
    return vec2<f32>(x, y);
}

struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) @interpolate(flat) ty: u32,
    @location(2) plane_pos: vec3<f32>,
};

@vertex
fn vertex(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> Varyings {
    let pos_center = generate_paint_pos(instance_index);

    let suf = get_surface(pos_center);

    let axis = (vec3<u32>(vertex_index) & vec3<u32>(1u, 2u, 4u)) != vec3<u32>(0u);
    let is_high = vertex_index > 10u;
    let heights = vec4<f32>(0.0, suf.low_alt, suf.low_alt + suf.delta, suf.high_alt);
    let shift = (u_Globals.camera_pos.xyz > vec3<f32>(pos_center, heights.z)) != axis;
    let h2 = select(heights.xy, heights.zw, is_high);
    let altitude = select(h2.x, h2.y, shift.z);
    let plane_pos = vec3<f32>(pos_center, altitude);

    let pos = vec3<f32>(floor(pos_center) + vec2<f32>(shift.xy), altitude);
    let ty = select(suf.low_type, suf.high_type, is_high);

    return Varyings(
        u_Globals.view_proj * vec4<f32>(pos, 1.0),
        pos,
        ty,
        plane_pos,
    );
}

//imported: Surface, u_Globals, get_surface, evaluate_color, apply_fog, fetch_shadow

@fragment
fn fragment(in: Varyings) -> @location(0) vec4<f32> {
    let lit_factor = fetch_shadow(in.plane_pos);
    let terrain_color = evaluate_color(in.ty, in.world_pos, lit_factor);
    return apply_fog(terrain_color, in.plane_pos.xy);
}
