//!include globals.inc terrain/locals.inc surface.inc color.inc

[[block]]
struct Storage {
    data: array<atomic<u32>>;
};
[[group(2), binding(0)]] var<storage, read_write> s_Storage: Storage;

// This has to match SCATTER_GROUP_SIZE
//TODO: use specialization constants
[[stage(compute), workgroup_size(16, 16, 1)]]
fn clear([[builtin(global_invocation_id)]] pos: vec3<u32>) {
    if (pos.x < u_Locals.screen_size.x && pos.y < u_Locals.screen_size.y) {
        //TODO: 0xFFFFFF00U when hex is supported
        atomicStore(&s_Storage.data[pos.y * u_Locals.screen_size.x + pos.x], 4294967040u);
    }
}

[[stage(vertex)]]
fn copy_vs([[builtin(vertex_index)]] index: u32) -> [[builtin(position)]] vec4<f32> {
    return vec4<f32>(
        select(-1.0, 1.0, index < 2u),
        select(-1.0, 1.0, (index & 1u) == 1u),
        0.0, 1.0,
    );
}

struct CopyOutput {
    [[location(0)]] color: vec4<f32>;
    [[builtin(frag_depth)]] depth: f32;
};

[[stage(fragment)]]
fn copy_fs([[builtin(position)]] pos: vec4<f32>) -> CopyOutput {
    let value = atomicLoad(&s_Storage.data[u32(pos.y) * u_Locals.screen_size.x + u32(pos.x)]);
    let color = textureLoad(t_Palette, i32(value & 255u), 0);
    let depth = f32(value >> 8u) / 16777215.0; //TODO: 0xFFFFFFu
    return CopyOutput(color, depth);
}


fn is_visible(p: vec4<f32>) -> bool {
    return p.w > 0.0 && p.z >= 0.0 &&
        p.x >= -p.w && p.x <= p.w &&
        p.y >= -p.w && p.y <= p.w;
}

fn add_voxel(pos: vec2<f32>, altitude: f32, ty: u32, lit_factor: f32) {
    let screen_pos = u_Globals.view_proj * vec4<f32>(pos, altitude, 1.0);
    if (!is_visible(screen_pos)) {
        return;
    }
    var ndc = screen_pos.xyz / screen_pos.w;
    ndc.y = -1.0 * ndc.y; // flip Y
    let color_id = evaluate_color_id(ty, pos / u_Surface.texture_scale.xy, altitude / u_Surface.texture_scale.z, lit_factor);
    let depth = clamp(ndc.z, 0.0, 1.0);
    let value = (u32(depth * 16777215.0) << 8u) | u32(color_id * 255.0); //TODO: 0xFFFFFF, 0xFF
    let tc = clamp(
        vec2<u32>(round((ndc.xy * 0.5 + 0.5) * vec2<f32>(u_Locals.screen_size.xy))),
        vec2<u32>(0u),
        u_Locals.screen_size.xy - vec2<u32>(1u),
    );
    let _old = atomicMin(&s_Storage.data[tc.y * u_Locals.screen_size.x + tc.x], value);
}

fn generate_scatter_pos(source_coord: vec2<f32>) -> vec2<f32> {
    var y: f32;
    if (true) {
        let y_sqrt = mix(
            sqrt(abs(u_Locals.sample_range.z)) * sign(u_Locals.sample_range.z),
            sqrt(abs(u_Locals.sample_range.w)) * sign(u_Locals.sample_range.w),
            source_coord.y
        );
        y = y_sqrt * y_sqrt * sign(y_sqrt);
    } else {
        y = mix(u_Locals.sample_range.z, u_Locals.sample_range.w, source_coord.y);
    }

    let x_limit = mix(
        u_Locals.sample_range.x, u_Locals.sample_range.y,
        (y - u_Locals.sample_range.z) / (u_Locals.sample_range.w - u_Locals.sample_range.z)
    );
    let x = mix(-x_limit, x_limit, source_coord.x);

    return u_Locals.cam_origin_dir.xy + u_Locals.cam_origin_dir.zw * y +
        vec2<f32>(u_Locals.cam_origin_dir.w, -u_Locals.cam_origin_dir.z) * x;
}

[[stage(compute), workgroup_size(16,16,1)]]
fn main(
    [[builtin(global_invocation_id)]] global_id: vec3<u32>,
    [[builtin(num_workgroups)]] num_workgroups: vec3<u32>,
) {
    let wg_size: vec3<u32> = vec3<u32>(16u, 16u, 1u); // has to match SCATTER_GROUP_SIZE
    let source_coord = vec2<f32>(global_id.xy) /
        vec2<f32>(num_workgroups.xy * wg_size.xy - vec2<u32>(1u));
    let pos = generate_scatter_pos(source_coord);

    let suf = get_surface(pos);
    var base = 0.0;
    let t = f32(global_id.z) / f32(num_workgroups.z * wg_size.z);

    if (suf.delta != 0.0) {
        let alt = mix(suf.low_alt, 0.0, t);
        add_voxel(pos, alt, suf.low_type, 0.25);
        base = suf.low_alt + suf.delta;
    }
    if (true) {
        let alt = mix(suf.high_alt, base, t);
        add_voxel(pos, alt, suf.high_type, 1.0);
    }
}