//!include globals.inc terrain/locals.inc surface.inc shadow.inc terrain/color.inc

// Triangulated irregular network. The mesh is fitted to the height map on
// the CPU (see `level::tin`), so the vertex stage is just a transform.
//
// Everything about the *shading* still comes from the terrain texture --
// the terrain type below, and the surface gradient inside `evaluate_color`.
// That matters: the triangles are deliberately coarse, but terrain type
// boundaries stay at full texel resolution, so this mode keeps the ray
// traced colouring instead of flat-shading each triangle from whichever
// vertex happened to be provoking.

struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    // 0 = the `low` floor, 1 = the `mid`/`high` slab.
    @location(1) @interpolate(flat) layer: u32,
};

// The mesh spans the level exactly once, but the level itself wraps, so we
// instance it across every tile the camera can currently see.
// `u_Locals.sample_range` is the visible XY bounds (see `Constants` in
// `render::terrain`); the instance count on the CPU side is derived from
// the same numbers, so the two agree on the tile grid.
fn tile_offset(instance_index: u32) -> vec2<f32> {
    let level_size = u_Surface.texture_scale.xy;
    let lo = floor(vec2<f32>(u_Locals.sample_range.x, u_Locals.sample_range.z) / level_size);
    let hi = floor(vec2<f32>(u_Locals.sample_range.y, u_Locals.sample_range.w) / level_size);
    let count_x = max(1u, u32(hi.x - lo.x) + 1u);
    let tile = vec2<f32>(f32(instance_index % count_x), f32(instance_index / count_x));
    return (lo + tile) * level_size;
}

@vertex
fn vertex(
    @builtin(instance_index) instance_index: u32,
    @location(0) pos: vec3<f32>,
    @location(1) layer: u32,
) -> Varyings {
    let world = vec3<f32>(pos.xy + tile_offset(instance_index), pos.z);
    return Varyings(
        u_Globals.view_proj * vec4<f32>(world, 1.0),
        world,
        layer,
    );
}

//imported: u_Globals, get_surface, evaluate_color, apply_fog, fetch_shadow_visibility

@fragment
fn fragment(in: Varyings) -> @location(0) vec4<f32> {
    let suf = get_surface(in.world_pos.xy);
    let ty = select(suf.low_type, suf.high_type, in.layer != 0u);
    let visibility = fetch_shadow_visibility(in.world_pos);
    let terrain_color = evaluate_color(ty, in.world_pos, visibility);
    return apply_fog(terrain_color, in.world_pos.xy);
}
