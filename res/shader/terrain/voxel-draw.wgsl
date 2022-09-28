//!include globals.inc terrain/locals.inc surface.inc shadow.inc terrain/color.inc

struct VoxelConstants {
    voxel_size: vec4<i32>,
    max_depth: f32,
};

@group(2) @binding(0) var voxel_grid: texture_3d<u32>;
@group(2) @binding(1) var<uniform> u_Constants: VoxelConstants;

var<private> debug_color: vec4<f32> = vec4<f32>(0.0, 0.0, 0.0, 0.0);

@vertex
fn main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    return vec4<f32>(
        select(-1.0, 3.0, index == 1u),
        select(-1.0, 3.0, index >= 2u),
        0.0,
        1.0,
    );
}

let TYPE_MISS: u32 = 0xFFu;

struct CastPoint {
    pos: vec3<f32>,
    ty: u32,
};

fn get_cast_t_range(base: vec3<f32>, dir: vec3<f32>) -> vec2<f32> {
    let t_bounds = (vec2<f32>(0.0, u_Surface.texture_scale.z) - base.zz) / dir.zz;
    if (dir.z > 0.0) {
        return vec2<f32>(0.0, min(u_Constants.max_depth, t_bounds.y));
    } else {
        let begin = select(0.0, t_bounds.y, t_bounds.y > 0.0);
        return vec2<f32>(begin, min(u_Constants.max_depth, t_bounds.x));
    }
}

fn cast_miss() -> CastPoint {
    return CastPoint(vec3<f32>(0.0), TYPE_MISS);
}

fn check_hit(pos: vec3<f32>) -> u32 {
    let suf = get_surface(pos.xy);
    if (pos.z < suf.low_alt) {
        return suf.low_type;
    } else
    if (pos.z >= suf.low_alt + suf.delta && pos.z < suf.high_alt) {
        return suf.high_type;
    } else {
        return TYPE_MISS;
    }
}

fn cast_ray_linear(a: vec3<f32>, b: vec3<f32>, num_steps: u32) -> CastPoint {
    var pt: CastPoint;
    for (var i: u32 = 0u; i < num_steps; i += 1u) {
        let c = mix(a, b, f32(i) / f32(num_steps));
        pt.ty = check_hit(c);
        if (pt.ty != TYPE_MISS) {
            pt.pos = c;
            return pt;
        }
    }

    pt.ty = check_hit(b);
    pt.pos = b;
    return pt;
}

fn cast_ray_binary(a_in: vec3<f32>, b_in: vec3<f32>, num_steps: u32) -> CastPoint {
    var pt: CastPoint;
    pt.ty = TYPE_MISS;
    var a = a_in;
    var b = b_in;

    for (var i: u32 = 0u; i < num_steps; i += 1u) {
        let c = 0.5 * (a + b);
        let ty = check_hit(c);
        if (pt.ty == TYPE_MISS) {
            a = c;
        } else {
            pt.ty = ty;
            b = c;
        }
    }

    pt.pos = b;
    return pt;
}

fn cast_ray_fallback(base: vec3<f32>, dir: vec3<f32>) -> CastPoint {
    let tr = get_cast_t_range(base, dir);
    if (tr.x >= tr.y)
    {
        return cast_miss();
    }

    return cast_ray_linear(base + tr.x * dir, base + tr.y * dir, 10u);
}

fn cast_ray_through_voxels(base: vec3<f32>, dir: vec3<f32>) -> CastPoint {
    let num_steps: u32 = 10u;

    var pos = base;
    if (dir.z >= 0.0 && base.z >= u_Surface.texture_scale.z) {
        return cast_miss();
    }
    if (dir.z < 0.0 && base.z > u_Surface.texture_scale.z) {
        let t = (u_Surface.texture_scale.z - base.z) / dir.z;
        pos += (t + 0.01) * dir;
    }

    let lod_count = u32(textureNumLevels(voxel_grid));
    let tci = get_map_coordinates(pos.xy);
    var lod = lod_count - 1u;
    var lod_voxel_pos = vec3<i32>(tci, i32(pos.z)) / (u_Constants.voxel_size.xyz << vec3<u32>(lod));
    var cur_step = 0u;
    loop {
        let occupancy = textureLoad(voxel_grid, lod_voxel_pos, i32(lod)).x;
        if (occupancy != 0u) {
            if (lod == 0u) {
                //debug_color = vec4<f32>(1.0, 0.0, 0.0, 1.0);
                break;
            }
            lod -= 1u;
            // Now that we descended to a LOD level below,
            // we need to clarify, which of the octants contains our position.
            lod_voxel_pos *= 2;
            let lod_voxel_size = u_Constants.voxel_size.xyz << vec3<u32>(lod);
            // Get the middle of the old voxel
            let mid = vec3<f32>((lod_voxel_pos + 1) * lod_voxel_size);
            lod_voxel_pos += vec3<i32>(step(mid, pos));
        } else {
            let lod_voxel_size = u_Constants.voxel_size.xyz << vec3<u32>(lod);
            // find a place where the ray hits the current voxel boundary
            // "a" and "b" define the corners of the current LOD box
            let a = vec3<f32>((lod_voxel_pos + 0) * lod_voxel_size);
            let b = vec3<f32>((lod_voxel_pos + 1) * lod_voxel_size);
            // "tc" is the distance to each of the walls of the box
            let tc = (select(a, b, dir > vec3<f32>(0.0)) - pos) / dir;
            // "t" is the closest distance to the boundary
            let t = min(tc.x, min(tc.y, tc.z));
            pos += t * dir;

            let boundary_exit = lod_voxel_pos + select(vec3<i32>(-1), vec3<i32>(1), dir > vec3<f32>(0.0));
            lod_voxel_pos = select(lod_voxel_pos, boundary_exit, vec3<f32>(t) >= tc);
            /*
            let can_raise = (lod_voxel_pos & vec3<i32>(1)) == vec3<i32>(step(vec3<f32>(0.0), dir)) || vec3<f32>(t) < tc;
            if (lod + 1u < lod_count && all(can_raise)) {
                lod += 1u;
            }*/

            cur_step += 1u;
            if (cur_step >= num_steps) {
                debug_color = vec4<f32>(0.0, 1.0, 0.0, 1.0);
                break;
            }
        }
    }

    //let voxel_cross_size = length(vec3<f32>(u_Constants.voxel_size.xyz));
    //return cast_ray_binary(pos, pos + dir * voxel_cross_size, 4u);
    //return cast_miss();
    let suf = get_surface(pos.xy);
    let ty = select(suf.low_type, suf.high_type, pos.z > suf.low_alt);
    return CastPoint(pos, ty);
}

struct FragOutput {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

@fragment
fn draw(@builtin(position) frag_coord: vec4<f32>,) -> FragOutput {
    let sp_near_world = get_frag_world(frag_coord.xy, 0.0);
    let sp_far_world = get_frag_world(frag_coord.xy, 1.0);
    let view = normalize(sp_far_world - sp_near_world);
    let pt = cast_ray_through_voxels(sp_near_world, view);
    if (debug_color.a > 0.0) {
        return FragOutput(debug_color, 1.0);
    }
    if (pt.ty == TYPE_MISS) {
        return FragOutput(vec4<f32>(0.1, 0.2, 0.3, 1.0), 1.0);
    }

    let lit_factor = fetch_shadow(pt.pos);
    let frag_color = evaluate_color(pt.ty, pt.pos, lit_factor);

    let target_ndc = u_Globals.view_proj * vec4<f32>(pt.pos, 1.0);
    let depth = target_ndc.z / target_ndc.w;
    return FragOutput(frag_color, depth);
}
