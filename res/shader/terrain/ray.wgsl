//!include globals.inc terrain/locals.inc surface.inc shadow.inc color.inc

[[stage(vertex)]]
fn main([[location(0)]] pos: vec4<i32>) -> [[builtin(position)]] vec4<f32> {
    // orhto projections don't like infinite values
    return select(
        u_Globals.view_proj * vec4<f32>(pos),
        // the expected geometry is 4 trianges meeting in the center
        vec4<f32>(vec2<f32>(pos.xy), 0.0, 0.5),
        u_Globals.view_proj[2][3] == 0.0
    );
}

//imported: Surface, u_Surface, get_surface, evaluate_color

let c_ReflectionVariance: f32 = 0.5;
let c_ReflectionPower: f32 = 0.2;
let c_TerrainWater: u32 = 0u;

fn cast_ray_to_plane(level: f32, base: vec3<f32>, dir: vec3<f32>) -> vec3<f32> {
    let t = (level - base.z) / dir.z;
    return t * dir + base;
}

struct CastResult {
    surface: Surface;
    a: vec3<f32>;
    b: vec3<f32>;
};

fn cast_ray_impl(
    a_in: vec3<f32>, b_in: vec3<f32>,
    high_in: bool, num_forward: i32, num_binary: i32
) -> CastResult {
    let step = (1.0 / f32(num_forward + 1)) * (b_in - a_in);
    var a = a_in;
    var b = b_in;
    var high = high_in;

    for (var i = 0; i < num_forward; i = i + 1) {
        let c = a + step;
        let suf = get_surface(c.xy);

        if (c.z > suf.high_alt) {
            high = true; // re-appear on the surface
            a = c;
        } else {
            let height = select(suf.low_alt, suf.high_alt, high);
            if (c.z <= height) {
                b = c;
                break;
            } else {
                a = c;
            }
        }
    }

    var result = get_surface(b.xy);

    for (var i = 0; i < num_binary; i = i + 1) {
        let c = mix(a, b, 0.5);
        let suf = get_surface(c.xy);

        let height = select(suf.low_alt, suf.high_alt, high);
        if (c.z <= height) {
            b = c;
            result = suf;
        } else {
            a = c;
        }
    }

    return CastResult(result, a, b);
}

struct CastPoint {
    pos: vec3<f32>;
    ty: u32;
    tex_coord: vec2<f32>;
    is_underground: bool;
    //is_shadowed: bool;
};

fn cast_ray_to_map(base: vec3<f32>, dir: vec3<f32>) -> CastPoint {
    var pt: CastPoint;

    let a_in = select(
        base,
        cast_ray_to_plane(u_Surface.texture_scale.z, base, dir),
        base.z > u_Surface.texture_scale.z,
    );
    var c = cast_ray_to_plane(0.0, base, dir);

    let cast_result = cast_ray_impl(a_in, c, true, 8, 4);
    var a = cast_result.a;
    var b = cast_result.b;
    var suf = cast_result.surface;
    pt.ty = suf.high_type;
    pt.is_underground = false;

    if (suf.delta != 0.0 && b.z < suf.low_alt + suf.delta) {
        // continue the cast underground, but reserve
        // the right to re-appear above the surface.
        let cr = cast_ray_impl(b, c, false, 6, 3);
        a = cr.a;
        b = cr.b;
        suf = cr.surface;
        if (b.z >= suf.low_alt + suf.delta) {
            pt.ty = suf.high_type;
        } else {
            pt.ty = suf.low_type;
            // underground is better indicated by a real shadow
            //pt.is_underground = true;
        }
    }

    pt.pos = b;
    pt.tex_coord = suf.tex_coord;
    //pt.is_shadowed = suf.is_shadowed;

    return pt;
}

fn color_point(pt: CastPoint, lit_factor: f32) -> vec4<f32> {
    return evaluate_color(pt.ty, pt.tex_coord, pt.pos.z / u_Surface.texture_scale.z, lit_factor);
}

let c_DepthBias: f32 = 0.01;

struct RayInput {
    [[builtin(position)]] frag_coord: vec4<f32>;
};

[[stage(fragment)]]
fn ray(in: RayInput) -> [[builtin(frag_depth)]] f32 {
    let sp_near_world = get_frag_world(in.frag_coord.xy, 0.0);
    let sp_far_world = get_frag_world(in.frag_coord.xy, 1.0);
    let view = normalize(sp_far_world - sp_near_world);
    let pt = cast_ray_to_map(sp_near_world, view);

    let target_ndc = u_Globals.view_proj * vec4<f32>(pt.pos, 1.0);
    return target_ndc.z / target_ndc.w + c_DepthBias;
}

struct FragOutput {
    [[location(0)]] color: vec4<f32>;
    [[builtin(frag_depth)]] depth: f32;
};

[[stage(fragment)]]
fn ray_color_debug(in: RayInput) -> FragOutput {
    let sp_near_world = get_frag_world(in.frag_coord.xy, 0.0);
    let sp_far_world = get_frag_world(in.frag_coord.xy, 1.0);
    let view = normalize(sp_far_world - sp_near_world);

    var point = cast_ray_to_plane(0.0, sp_near_world, view);
    let surface = get_surface(point.xy);
    let color = vec4<f32>(surface.low_alt, surface.high_alt, surface.delta, 0.0) / 255.0;
    return FragOutput(color, 1.0);
}

[[stage(fragment)]]
fn ray_color(in: RayInput) -> FragOutput {
    let sp_near_world = get_frag_world(in.frag_coord.xy, 0.0);
    let sp_far_world = get_frag_world(in.frag_coord.xy, 1.0);
    let view = normalize(sp_far_world - sp_near_world);
    let pt = cast_ray_to_map(sp_near_world, view);

    let lit_factor = fetch_shadow(pt.pos);
    var frag_color = color_point(pt, lit_factor);

    if (pt.ty == c_TerrainWater) {
        let a = pt.pos;
        let variance = a.xy % c_ReflectionVariance;
        let reflected = normalize(view * vec3<f32>(1.0 + variance, -1.0));
        let outside = cast_ray_to_plane(u_Surface.texture_scale.z, a, reflected);

        let cr = cast_ray_impl(a, outside, true, 4, 4);
        if (any(cr.b != outside)) {
            let other = CastPoint(cr.b, cr.surface.high_type, cr.surface.tex_coord, false);
            let ref_color = color_point(other, 0.8);
            frag_color = frag_color + c_ReflectionPower * ref_color;
        }
    }

    let target_ndc = u_Globals.view_proj * vec4<f32>(pt.pos, 1.0);
    let depth = target_ndc.z / target_ndc.w;
    return FragOutput(frag_color, depth);
}

let c_Step: f32 = 0.6;

// Algorithm is based on "http://www.tevs.eu/project_i3d08.html"
//"Maximum Mipmaps for Fast, Accurate, and Scalable Dynamic Height Field Rendering"
fn cast_ray_mip(base_point: vec3<f32>, dir: vec3<f32>) -> vec3<f32> {
    var point = base_point;
    var lod = u_Locals.params.x;
    var ipos = vec2<i32>(floor(point.xy)); // integer coordinate of the cell
    var num_jumps = u_Locals.params.y;
    var num_steps = u_Locals.params.z;
    loop {
        // step 0: at lowest LOD, just advance
        if (lod == 0u) {
            let surface = get_surface(point.xy);
            if (point.z < surface.low_alt || (point.z < surface.high_alt && point.z >= surface.low_alt + surface.delta)) {
                break;
            }
            if (surface.low_alt == surface.high_alt) {
                lod = lod + 1u; //try to escape the low level and LOD
            }
            point = point + c_Step * dir;
            ipos = vec2<i32>(floor(point.xy));
            num_steps = num_steps - 1u;
            if (num_steps == 0u) {
                break;
            }
            continue;
        }

        // step 1: get the LOD height and early out
        let height = get_lod_height(ipos, lod);
        if (point.z <= height) {
            lod = lod - 1u;
            continue;
        }
        // assumption: point.z >= height

        // step 2: figure out the closest intersection with the cell
        // it can be X axis, Y axis, or the depth
        let cell_id = floor(vec2<f32>(ipos) / f32(1 << lod)); // careful!
        let cell_tl = vec2<i32>(cell_id) << vec2<u32>(lod);
        let cell_offset = vec2<f32>(cell_tl) + f32(1 << lod) * step(vec2<f32>(0.0), dir.xy) - point.xy;
        let units = vec3<f32>(cell_offset, height - point.z) / dir;
        let min_side_unit = min(units.x, units.y);

        // advance the point
        point = point + min(units.z, min_side_unit) * dir;
        ipos = vec2<i32>(floor(point.xy));
        num_jumps = num_jumps - 1u;

        if (units.z < min_side_unit) {
            lod = lod - 1u;
        } else {
            // adjust the integer position on cell boundary
            // figure out if we hit the higher LOD bound and switch to it
            var affinity = 0.0;
            let proximity = cell_id % 2.0 - vec2<f32>(0.5);
            if (units.x <= units.y) {
                ipos.x = select(cell_tl.x - 1, cell_tl.x + (1 << lod), dir.x >= 0.0);
                affinity = dir.x * proximity.x;
            }
            if (units.y <= units.x) {
                ipos.y = select(cell_tl.y - 1, cell_tl.y + (1 << lod), dir.y >= 0.0);
                affinity = dir.y * proximity.y;
            }
            if (lod < u_Locals.params.x && affinity > 0.0) {
                lod = lod + 1u;
            }
        }
        if (num_jumps == 0u) {
            break;
        }
    }

    return point;
}

[[stage(fragment)]]
fn ray_mip_color(in: RayInput) -> FragOutput {
    let sp_near_world = get_frag_world(in.frag_coord.xy, 0.0);
    let sp_far_world = get_frag_world(in.frag_coord.xy, 1.0);
    let view = normalize(sp_far_world - sp_near_world);
    let point = cast_ray_mip(sp_near_world, view);

    let lit_factor = fetch_shadow(point);
    let surface = get_surface(point.xy);
    let ty = select(surface.low_type, surface.high_type, point.z > surface.low_alt);
    let frag_color = evaluate_color(ty, surface.tex_coord, point.z / u_Surface.texture_scale.z, lit_factor);

    let target_ndc = u_Globals.view_proj * vec4<f32>(point, 1.0);
    let depth = target_ndc.z / target_ndc.w;
    return FragOutput(frag_color, depth);
}
