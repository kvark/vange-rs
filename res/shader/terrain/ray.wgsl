//!include globals.inc terrain/locals.inc surface.inc shadow.inc terrain/color.inc

@vertex
fn main(@location(0) pos: vec4<i32>) -> @builtin(position) vec4<f32> {
    // Four triangles meet at the centre; w=0.5 expands their cardinal
    // vertices into a full-screen diamond. Projecting the old infinite
    // ground fan clipped every ray above the horizon, including visible cave
    // ceilings, so the marcher now decides whether each screen ray misses.
    return vec4<f32>(vec2<f32>(pos.xy), 0.0, 0.5);
}

//imported: Surface, u_Surface, get_surface, evaluate_color

fn cast_ray_to_plane(level: f32, base: vec3<f32>, dir: vec3<f32>) -> vec3<f32> {
    let t = (level - base.z) / dir.z;
    return t * dir + base;
}

struct CastResult {
    surface: Surface,
    a: vec3<f32>,
    b: vec3<f32>,
    hit: bool,
};

fn cast_ray_impl(
    a_in: vec3<f32>, b_in: vec3<f32>,
    num_forward: i32, num_binary: i32
) -> CastResult {
    let step = (1.0 / f32(num_forward + 1)) * (b_in - a_in);
    var a = a_in;
    var b = b_in;
    var hit = false;

    for (var i = 0; i < num_forward; i = i + 1) {
        let c = a + step;
        let suf = get_surface_alt(c.xy);
        let inside = !surface_is_void(suf) && (c.z <= suf.low ||
            (suf.low < suf.high && c.z >= suf.mid && c.z <= suf.high));
        if (inside) {
            b = c;
            hit = true;
            break;
        } else {
            a = c;
        }
    }

    if (!hit) {
        let end_surface = get_surface_alt(b_in.xy);
        hit = !surface_is_void(end_surface) && (b_in.z <= end_surface.low ||
            (end_surface.low < end_surface.high &&
             b_in.z >= end_surface.mid && b_in.z <= end_surface.high));
    }

    for (var i = 0; i < num_binary && hit; i += 1) {
        let c = mix(a, b, 0.5);
        let suf = get_surface_alt(c.xy);
        let inside = !surface_is_void(suf) && (c.z <= suf.low ||
            (suf.low < suf.high && c.z >= suf.mid && c.z <= suf.high));
        if (inside) {
            b = c;
        } else {
            a = c;
        }
    }

    let result = get_surface(b.xy);
    return CastResult(result, a, b, hit);
}

struct CastPoint {
    pos: vec3<f32>,
    ty: u32,
    is_underground: bool,
    hit: bool,
    //is_shadowed: bool,
};

fn cast_ray_to_map(base: vec3<f32>, far: vec3<f32>) -> CastPoint {
    var pt: CastPoint;
    let dir = normalize(far - base);

    let far_distance = distance(base, far);
    let floor_distance = select(
        far_distance,
        max(0.0, (0.0 - base.z) / dir.z),
        dir.z < -0.00001,
    );
    let c = base + dir * min(far_distance, floor_distance);

    let forward_steps = max(1, i32(u_Locals.terrain_params.y));
    let cast_result = cast_ray_impl(base, c, forward_steps, 4);
    let suf = cast_result.surface;
    pt.is_underground = cast_result.hit &&
        suf.low_alt < suf.high_alt && cast_result.b.z <= suf.low_alt;
    pt.ty = select(suf.high_type, suf.low_type, pt.is_underground);
    pt.pos = cast_result.b;
    pt.hit = cast_result.hit;
    //pt.is_shadowed = suf.is_shadowed;

    return pt;
}

fn color_point(pt: CastPoint, visibility: f32) -> vec4<f32> {
    let cave_visibility = select(1.0, 0.25, pt.is_underground);
    return evaluate_color(pt.ty, pt.pos, cave_visibility * visibility);
}

struct RayInput {
    @builtin(position) frag_coord: vec4<f32>,
};

@fragment
fn ray_depth(in: RayInput) -> @builtin(frag_depth) f32 {
    let sp_near_world = get_frag_world(in.frag_coord.xy, 0.0);
    let sp_far_world = get_frag_world(in.frag_coord.xy, 1.0);
    let pt = cast_ray_to_map(sp_near_world, sp_far_world);

    let target_ndc = u_Globals.view_proj * vec4<f32>(pt.pos, 1.0);
    return select(1.0, target_ndc.z / target_ndc.w + c_DepthBias, pt.hit);
}

struct FragOutput {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

@fragment
fn ray_color_debug(in: RayInput) -> FragOutput {
    let sp_near_world = get_frag_world(in.frag_coord.xy, 0.0);
    let sp_far_world = get_frag_world(in.frag_coord.xy, 1.0);
    let view = normalize(sp_far_world - sp_near_world);

    let pos = cast_ray_to_plane(0.0, sp_near_world, view);
    let surface = get_surface(pos.xy);
    let color = vec4<f32>(surface.low_alt, surface.mid_alt, surface.high_alt, 0.0) / 255.0;
    return FragOutput(color, 1.0);
}

@fragment
fn ray_color(in: RayInput) -> FragOutput {
    let sp_near_world = get_frag_world(in.frag_coord.xy, 0.0);
    let sp_far_world = get_frag_world(in.frag_coord.xy, 1.0);
    let pt = cast_ray_to_map(sp_near_world, sp_far_world);

    if (!pt.hit) {
        return FragOutput(u_Locals.fog_color, 1.0);
    }

    let visibility = fetch_shadow_visibility(pt.pos);
    var frag_color = apply_fog(color_point(pt, visibility), pt.pos.xy);
    frag_color.a = focus_visibility(pt.pos);

    let target_ndc = u_Globals.view_proj * vec4<f32>(pt.pos, 1.0);
    let depth = target_ndc.z / target_ndc.w;
    return FragOutput(frag_color, depth);
}
