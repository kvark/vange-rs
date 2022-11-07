//!include globals.inc morton.inc terrain/locals.inc surface.inc shadow.inc terrain/color.inc terrain/voxel.inc

struct VoxelConstants {
    voxel_size: vec4<i32>,
    max_depth: f32,
    debug_alpha: f32,
    max_outer_steps: u32,
    max_inner_steps: u32,
}

struct VoxelData {
    lod_count: vec4<u32>,
    lods: array<VoxelLod, 16>,
    occupancy: array<u32>,
}

@group(2) @binding(0) var<storage, read> b_VoxelGrid: VoxelData;
@group(2) @binding(1) var<uniform> u_Constants: VoxelConstants;

fn check_occupancy(coordinates: vec3<i32>, lod: u32) -> bool {
    let lod_info = b_VoxelGrid.lods[lod];
    let ramainder = coordinates % lod_info.dim;
    // see https://github.com/gfx-rs/naga/issues/2122
    let sanitized = ramainder + select(lod_info.dim, vec3<i32>(0), ramainder >= vec3<i32>(0));
    let addr = linearize(vec3<u32>(sanitized), lod_info);
    return (b_VoxelGrid.occupancy[addr.offset] & addr.mask) != 0u;
}

let enable_unzoom = true;
let step_scale = 1.0;

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
    if (pos.z >= suf.mid_alt && pos.z < suf.high_alt) {
        return suf.high_type;
    } else {
        return TYPE_MISS;
    }
}

fn cast_ray_linear(a: vec3<f32>, b: vec3<f32>, num_steps: u32) -> CastPoint {
    var pt: CastPoint;
    for (var i: u32 = 0u; i < num_steps; i += 1u) {
        let c = mix(a, b, (f32(i) + 0.5) / f32(num_steps));
        pt.ty = check_hit(c);
        if (pt.ty != TYPE_MISS) {
            pt.pos = c;
            return pt;
        }
    }

    pt.ty = TYPE_MISS;
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

fn set_debug_color(num_outer_steps: u32, num_inner_steps: u32, extra_height: f32) {
    if (u_Constants.debug_alpha > 0.0) {
        debug_color = vec4<f32>(
            extra_height * 0.1,
            1.0 - f32(num_outer_steps) / f32(u_Constants.max_outer_steps),
            1.0 - f32(num_inner_steps) / f32(u_Constants.max_inner_steps),
            u_Constants.debug_alpha,
        );
    }
}

fn cast_ray_through_voxels(base: vec3<f32>, dir: vec3<f32>) -> CastPoint {
    var num_outer_steps: u32 = u_Constants.max_outer_steps;
    var num_inner_steps: u32 = u_Constants.max_inner_steps;

    var pos = base;
    if (dir.z >= 0.0 && base.z >= u_Surface.texture_scale.z) {
        return cast_miss();
    }
    if (dir.z < 0.0 && base.z > u_Surface.texture_scale.z) {
        let t = (u_Surface.texture_scale.z - base.z) / dir.z;
        pos += (t + 0.01) * dir;
    }

    let tpu = step_scale / abs(dir); // "t" step per unit of distance
    let t_step = min(tpu.x, min(tpu.y, tpu.y));

    var lod = b_VoxelGrid.lod_count.x - 1u;
    let base_lod_voxel_size = vec3<f32>(u_Constants.voxel_size.xyz << vec3<u32>(lod));
    var lod_voxel_pos = vec3<i32>(floor(pos / base_lod_voxel_size));
    loop {
        let is_occupied = check_occupancy(lod_voxel_pos, lod);
        if (is_occupied && lod != 0u) {
            lod -= 1u;
            // Now that we descended to a LOD level below,
            // we need to clarify, which of the octants contains our position.
            lod_voxel_pos *= 2;
            let lod_voxel_size = u_Constants.voxel_size.xyz << vec3<u32>(lod);
            // Get the middle of the old voxel
            let mid = vec3<f32>((lod_voxel_pos + 1) * lod_voxel_size);
            lod_voxel_pos += vec3<i32>(step(mid, pos));
            continue;
        }

        let lod_voxel_size = u_Constants.voxel_size.xyz << vec3<u32>(lod);
        // find a place where the ray hits the current voxel boundary
        // "a" and "b" define the corners of the current LOD box
        let a = vec3<f32>((lod_voxel_pos + 0) * lod_voxel_size);
        let b = vec3<f32>((lod_voxel_pos + 1) * lod_voxel_size);
        // "tc" is the distance to each of the walls of the box
        let tc = (select(a, b, dir > vec3<f32>(0.0)) - pos) / dir;
        // "t" is the closest distance to the boundary
        let t = min(tc.x, min(tc.y, tc.z));
        let new_pos = pos + t * dir;

        // If we reached the lowest level, and we know the cell is occupied,
        // try stepping through the cell.
        if (is_occupied && lod == 0u) {
            let num_linear_steps = min(u32(ceil(t / t_step)), num_inner_steps);
            num_inner_steps -= num_linear_steps;
            let cp = cast_ray_linear(pos, new_pos, num_linear_steps);
            if (cp.ty != TYPE_MISS) {
                set_debug_color(num_outer_steps, num_inner_steps, 0.0);
                return cp;
            } else if (num_inner_steps == 0u) {
                break;
            }
        }
        pos = new_pos;

        let voxel_shift_dir = select(vec3<i32>(-1), vec3<i32>(1), dir > vec3<f32>(0.0));
        let voxel_shift = select(vec3<i32>(0), voxel_shift_dir, vec3(t) == tc);
        let can_raise = (lod_voxel_pos & vec3<i32>(1)) == vec3<i32>(step(vec3<f32>(0.0), dir));
        //TODO: this should just use "||", but currently GLSL backend doesn't handle it
        let will_raise = select(vec3<bool>(true), can_raise, vec3<f32>(t) == tc);
        if (enable_unzoom && lod + 1u < b_VoxelGrid.lod_count.x && all(will_raise)) {
            lod += 1u;
            lod_voxel_pos = (lod_voxel_pos + select(vec3<i32>(0), vec3<i32>(-1), lod_voxel_pos < vec3<i32>(0))) / 2;
        }
        lod_voxel_pos += voxel_shift;

        if (num_outer_steps == 0u || lod_voxel_pos.z < 0) {
            break;
        }
        num_outer_steps -= 1u;
    }

    if (dir.z >= 0.0) {
        return CastPoint(pos, TYPE_MISS);
    }
    let suf = get_surface(pos.xy);
    set_debug_color(num_outer_steps, num_inner_steps, pos.z - suf.high_alt);
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
    //let pt = cast_ray_fallback(sp_near_world, view);
    if (debug_color.a == 1.0) {
        return FragOutput(debug_color, 1.0);
    }
    if (pt.ty == TYPE_MISS) {
        return FragOutput(u_Locals.fog_color, 1.0);
    }

    let lit_factor = fetch_shadow(pt.pos);
    let frag_color = evaluate_color(pt.ty, pt.pos, lit_factor);
    let actual_color = mix(frag_color, debug_color, debug_color.a);

    let target_ndc = u_Globals.view_proj * vec4<f32>(pt.pos, 1.0);
    let depth = target_ndc.z / target_ndc.w;
    return FragOutput(actual_color, depth);
}

struct DebugOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) lod: u32,
}

@vertex
fn vert_bound(
    @builtin(vertex_index) vert_index: u32,
    @builtin(instance_index) inst_index: u32,
) -> DebugOutput {
    var lod = 0u;
    var index = i32(inst_index);
    var coord = vec3<i32>(0);
    while (lod < b_VoxelGrid.lod_count.x) {
        let dim = b_VoxelGrid.lods[lod].dim;
        let total = i32(dim.x * dim.y * dim.z);
        if (index < total) {
            coord = vec3<i32>(index % dim.x, (index / dim.x) % dim.y, index / (dim.x * dim.y));
            break;
        }
        index -= total;
        lod += 1u;
    }

    let is_occupied = check_occupancy(coord, lod);
    if (lod == b_VoxelGrid.lod_count.x || !is_occupied) {
        return DebugOutput(vec4<f32>(0.0, 0.0, 0.0, 1.0), 0u);
    };

    let lod_voxel_size = u_Constants.voxel_size.xyz << vec3<u32>(lod);
    let origin = vec3<f32>(coord * lod_voxel_size);
    let axis = (vec3<u32>(vert_index) & vec3<u32>(1u, 2u, 4u)) != vec3<u32>(0u);
    let shift = (u_Globals.camera_pos.xyz > origin) != axis;
    let world_pos = origin + vec3<f32>(shift) * vec3<f32>(lod_voxel_size);

    var out: DebugOutput;
    out.pos = u_Globals.view_proj * vec4<f32>(world_pos, 1.0);
    out.lod = lod;
    return out;
}

@fragment
fn draw_bound(in: DebugOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 0.0, 0.0, 0.5);
}
