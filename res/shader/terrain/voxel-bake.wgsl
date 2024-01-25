//!include morton.inc surface.inc terrain/voxel.inc

struct VoxelData {
    lod_count: vec4<u32>,
    lods: array<vec4<u32>, 16>,
    occupancy: array<atomic<u32>>,
}
@group(0) @binding(0) var<storage, read_write> b_VoxelGrid: VoxelData;

struct BakeConstants {
    voxel_size: vec4<i32>,
    update_start: vec3<i32>,
    update_end: vec3<i32>,
}
@group(0) @binding(1) var<uniform> u_Constants: BakeConstants;

struct MipConstant {
    src_lod: u32,
}
@group(0) @binding(2) var<uniform> u_Mip: MipConstant;

fn unset_bit(addr: BitAddress) {
    atomicAnd(&b_VoxelGrid.occupancy[addr.offset], ~addr.mask);
}
fn set_bit(addr: BitAddress) {
    atomicOr(&b_VoxelGrid.occupancy[addr.offset], addr.mask);
}
fn check_bit(addr: BitAddress) -> bool {
    return (atomicLoad(&b_VoxelGrid.occupancy[addr.offset]) & addr.mask) != 0u;
}

@compute @workgroup_size(8, 8, 1)
fn init(
    @builtin(global_invocation_id) global_id: vec3<u32>,
) {
    let flat_coords = u_Constants.update_start.xy + vec2<i32>(global_id.xy);
    if (any(flat_coords >= u_Constants.update_end.xy)) {
        return;
    }

    let vlod = b_VoxelGrid.lods[0];

    if (all(flat_coords % u_Constants.voxel_size.xy == vec2<i32>(0))) {
        for(var z = u_Constants.update_start.z; z < u_Constants.update_end.z; z += u_Constants.voxel_size.z) {
            let voxel_coords = vec3<i32>(flat_coords, z) / u_Constants.voxel_size.xyz;
            unset_bit(linearize(vec3<u32>(voxel_coords), vlod));
        }
    }

    let suf = get_surface_impl(flat_coords);
    // All the voxel occupancy bits are unset now, wait until we can set them again.
    workgroupBarrier();

    for (var z=u_Constants.update_start.z; z < u_Constants.update_end.z; z += u_Constants.voxel_size.z) {
        // Do a range intersection with the terrain
        let z0 = f32(z);
        let z1 = f32(z + u_Constants.voxel_size.z);
        if (z1 <= suf.low_alt || (z1 > suf.mid_alt && z0 < suf.high_alt)) {
            let voxel_coords = vec3<i32>(flat_coords, z) / u_Constants.voxel_size.xyz;
            set_bit(linearize(vec3<u32>(voxel_coords), vlod));
        }
    }
}

@compute @workgroup_size(4, 4, 4)
fn mip(
    @builtin(global_invocation_id) global_id: vec3<u32>,
) {
    let dst_lod = u_Mip.src_lod + 1u;
    let dst_coords = (vec3<u32>(u_Constants.update_start / u_Constants.voxel_size.xyz) >> vec3<u32>(dst_lod)) + global_id;
    if (any(vec3<i32>(dst_coords << vec3<u32>(dst_lod)) * u_Constants.voxel_size.xyz >= u_Constants.update_end)) {
        return;
    }

    let slod = b_VoxelGrid.lods[u_Mip.src_lod];
    var is_occupied = false;
    for (var i = 0u; i < 8u; i += 1u) {
        let src_coords = dst_coords * 2u + ((vec3<u32>(i) >> vec3<u32>(0u, 1u, 2u)) & vec3<u32>(1u));
        let src_index = linearize(src_coords, slod);
        if (check_bit(src_index)) {
            is_occupied = true;
            break;
        }
    }

    let dst_index = linearize(dst_coords, b_VoxelGrid.lods[dst_lod]);
    if (is_occupied) {
        set_bit(dst_index);
    } else {
        unset_bit(dst_index);
    }
}
