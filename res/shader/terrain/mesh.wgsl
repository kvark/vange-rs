//!include globals.inc terrain/locals.inc surface.inc shadow.inc terrain/color.inc

// Triangulated irregular network. XY topology is fitted to the height map on
// the CPU (see `level::tin`); current altitude comes from the terrain texture
// so animation does not wait for CPU topology maintenance.
//
// Terrain type still comes from the height texture, so material boundaries
// stay at full texel resolution on coarse triangles. Lighting does not:
// the fragment stage uses the triangle's geometric normal
// (`evaluate_color_normal`) because vertical walls and cave ceilings have
// no meaningful height-field gradient.

struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    // 0 = low floor, 1 = cave ceiling (`mid`), 2 = slab top (`high`).
    @location(1) @interpolate(flat) layer: u32,
};

// The mesh spans the level exactly once, but the level wraps, so nearby
// copies have to be drawn too. The renderer issues one draw per (chunk,
// copy) and encodes which copy in the instance index, as a 3x3 grid of
// offsets around the camera's own tile.
//
// The offsets are explicit small integers shared with the CPU rather than
// a grid each side derives from the visible bounds. Deriving it twice is
// what went wrong before: any disagreement silently places wrapped terrain
// at the wrong distance, which reads as solid geometry hanging in mid-air.
fn tile_offset(instance_index: u32) -> vec2<f32> {
    let level_size = u_Surface.texture_scale.xy;
    let cam_tile = floor(u_Locals.cam_origin_dir.xy / level_size);
    let copy = vec2<f32>(f32(instance_index % 3u), f32(instance_index / 3u)) - vec2<f32>(1.0);
    return (cam_tile + copy) * level_size;
}

@vertex
fn vertex(
    @builtin(instance_index) instance_index: u32,
    @location(0) pos: vec3<f32>,
    @location(1) layer: u32,
) -> Varyings {
    // XY topology changes rarely; height changes every animation frame.
    // Reading it from the texture keeps moving land and deformation current
    // without waiting for the single-threaded CPU TIN to re-emit this chunk.
    let surface = get_surface(pos.xy);
    var height = surface.low_alt;
    if (layer == 1u) {
        height = surface.mid_alt;
    } else if (layer == 2u) {
        height = surface.high_alt;
    }
    let world = vec3<f32>(pos.xy + tile_offset(instance_index), height);
    return Varyings(
        u_Globals.view_proj * vec4<f32>(world, 1.0),
        world,
        layer,
    );
}

//imported: u_Globals, get_surface, evaluate_color, apply_fog, fetch_shadow_visibility

@fragment
fn fragment(in: Varyings) -> @location(0) vec4<f32> {
    if (in.world_pos.z <= 0.0 || focus_visibility(in.world_pos) < 0.45) {
        discard;
    }
    let suf = get_surface(in.world_pos.xy);
    let ty = select(suf.low_type, suf.high_type, in.layer != 0u);
    let visibility = fetch_shadow_visibility(in.world_pos);

    // The polygon's own normal, from screen-space derivatives of the world
    // position. A ray marcher has to rebuild this from height map taps;
    // here it is exact but facets within each triangle. Smoothing it with
    // the height-field gradient (below) keeps gentle slopes continuous
    // while the geometric normal stays authoritative where the mesh
    // stops following the height field - on vertical walls and cave
    // ceilings, where the gradient is undefined.
    var geo = normalize(cross(dpdx(in.world_pos), dpdy(in.world_pos)));
    let to_eye = u_Globals.camera_pos.xyz - in.world_pos;
    if (dot(geo, to_eye) < 0.0) {
        geo = -geo;
    }

    // Smooth surface normal from the height gradient, the same way the ray
    // marcher's `evaluate_color` builds one. Costs four height taps.
    let gradient = get_surface_gradient(in.world_pos);
    let grad_normal = normalize(vec3<f32>(-0.5 * gradient.x, -0.5 * gradient.y, 1.0));

    // How vertical the polygon is: 0 on level ground, 1 on a wall or a
    // ceiling. Blend to the geometric normal as it turns, so the smooth
    // gradient never lights a cliff face like a ramp.
    let slant = 1.0 - abs(geo.z);
    // `u_Locals.lighting_flags[1]`: 1 = smooth normals on, 0 = flat.
    let normal = normalize(mix(geo, mix(grad_normal, geo, smoothstep(0.55, 0.8, slant)), f32(u_Locals.lighting_flags[1u])));

    let terrain_color = evaluate_color_normal(ty, in.world_pos, visibility, normal);
    return apply_fog(terrain_color, in.world_pos.xy);
}
