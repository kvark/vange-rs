struct VoxelLod {
    dim: vec3<i32>,
    offset: u32,
}

struct BitAddress {
    offset: u32,
    mask: u32,
}

fn linearize(coords: vec3<u32>, vlod: VoxelLod) -> BitAddress {
    let words_per_tile = `morton_tile_size` * `morton_tile_size` * `morton_tile_size` / 32u;
    // Per-component construction sidesteps a naga MSL bug where
    // `vec3<u32>(vlod.dim)` is emitted as an `as_type` bitcast between
    // Metal's packed_int3 (12 B storage layout) and uint3 (16 B SIMD
    // layout). Same-sized bitcast fails to compile on macOS WebGPU.
    let dim = vec3<u32>(u32(vlod.dim.x), u32(vlod.dim.y), u32(vlod.dim.z));
    let tile_size = vec3<u32>(`morton_tile_size`);
    let tile_counts = (dim - 1u) / tile_size + 1u;
    let bit_index = encode_morton3(coords % tile_size);
    let tile_coord = coords / tile_size;
    let tile_index = (tile_coord.z * tile_counts.y + tile_coord.y) * tile_counts.x + tile_coord.x;
    var addr: BitAddress;
    addr.offset = vlod.offset + tile_index * words_per_tile + bit_index / 32u;
    addr.mask = 1u << (bit_index & 31u);
    return addr;
}
