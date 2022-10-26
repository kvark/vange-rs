//!include surface.inc

@group(0) @binding(0)
var mip_src: texture_3d<u32>;

@group(0) @binding(1)
var mip_dst: texture_storage_3d<r32uint, write>;

var<workgroup> result: atomic<u32>;

//TODO: we can avoid running an invocation per Z, and instead
// just iterate the affected Z values and write the data out.
// This would require storing a 128-bit mask of the occupied height layers.
// We could go even further and store this mask in a 2D texture instead
// of the full 3D grid.

@compute @workgroup_size(`group_w`,`group_h`,`group_d`)
fn init(
    @builtin(global_invocation_id) global_id: vec3<u32>,
    @builtin(workgroup_id) workgroup_id: vec3<u32>,
    @builtin(local_invocation_index) local_index: u32,
) {
    if (local_index == 0u) {
        atomicStore(&result, 0u);
    }
    var suf: Surface;
    if (global_id.x < u32(u_Surface.texture_scale.x) && global_id.y < u32(u_Surface.texture_scale.y)) {
        suf = get_surface_impl(vec2<i32>(global_id.xy));
    }

    workgroupBarrier();
    let group_size = vec3<f32>(`group_w`.0,`group_h`.0,`group_d`.0);
    let c0 = vec3<f32>(workgroup_id + vec3<u32>(0u)) * group_size;
    let c1 = vec3<f32>(workgroup_id + vec3<u32>(1u)) * group_size;
    if (suf.low_alt > c0.z || (suf.high_alt > c0.z && suf.low_alt + suf.delta < c1.z)) {
        atomicAdd(&result, 1u);
    }

    workgroupBarrier();
    if (local_index == 0u) {
        textureStore(mip_dst, vec3<i32>(workgroup_id), vec4<u32>(atomicLoad(&result)));
    }
}

@compute @workgroup_size(2,2,2)
fn mip(
    @builtin(global_invocation_id) global_id: vec3<u32>,
    @builtin(workgroup_id) workgroup_id: vec3<u32>,
    @builtin(local_invocation_index) local_index: u32,
) {
    if (local_index == 0u) {
        atomicStore(&result, 0u);
    }
    let tex_size = vec3<u32>(textureDimensions(mip_src, 0));
    var value = 0u;
    if (global_id.x < tex_size.x && global_id.y < tex_size.y && global_id.z < tex_size.z)
    {
        value = textureLoad(mip_src, vec3<i32>(global_id), 0).x;
    }

    workgroupBarrier();
    atomicAdd(&result, value);

    workgroupBarrier();
    if (local_index == 0u) {
        textureStore(mip_dst, vec3<i32>(workgroup_id), vec4<u32>(atomicLoad(&result)));
    }
}
