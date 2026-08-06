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
// instance it across the tiles the camera can currently see.
//
// The tile grid is bounded to a fixed radius around the camera's own tile.
// A far plane much larger than the level would otherwise ask for hundreds
// of tiles, and anything a whole level away is deep in the fog regardless.
// Both this and the instance count in `render::terrain` derive the grid the
// same way from the same uniforms -- they have to agree exactly, or the
// drawn tiles stop lining up with the instance indices and the camera's own
// tile can drop out of the draw entirely.
const c_MaxTileRadius: f32 = 1.0;

fn tile_grid() -> vec4<f32> {
    let level_size = u_Surface.texture_scale.xy;
    let cam_tile = floor(u_Locals.cam_origin_dir.xy / level_size);
    let visible_lo = floor(vec2<f32>(u_Locals.sample_range.x, u_Locals.sample_range.z) / level_size);
    let visible_hi = floor(vec2<f32>(u_Locals.sample_range.y, u_Locals.sample_range.w) / level_size);
    let lo = max(visible_lo, cam_tile - c_MaxTileRadius);
    let hi = max(lo, min(visible_hi, cam_tile + c_MaxTileRadius));
    return vec4<f32>(lo, hi - lo + vec2<f32>(1.0));
}

fn tile_offset(instance_index: u32) -> vec2<f32> {
    let grid = tile_grid();
    let count_x = max(1u, u32(grid.z));
    let tile = vec2<f32>(f32(instance_index % count_x), f32(instance_index / count_x));
    return (grid.xy + tile) * u_Surface.texture_scale.xy;
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

    // The polygon's own normal, from screen-space derivatives of the world
    // position. A ray marcher has to rebuild this from height map taps;
    // here it is exact and free, and it is the only thing that gives a
    // sensible normal on the vertical walls and cave ceilings, where the
    // height gradient is undefined.
    var normal = normalize(cross(dpdx(in.world_pos), dpdy(in.world_pos)));
    let to_eye = u_Globals.camera_pos.xyz - in.world_pos;
    if (dot(normal, to_eye) < 0.0) {
        normal = -normal;
    }

    let terrain_color = evaluate_color_normal(ty, in.world_pos, visibility, normal);
    return apply_fog(terrain_color, in.world_pos.xy);
}
