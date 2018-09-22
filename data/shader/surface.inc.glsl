// Common routines for fetching the level surface data.

uniform c_Surface {
    vec4 u_TextureScale;    // XY = size, Z = height scale, w = number of layers
};

uniform sampler2DArray t_Height;
uniform usampler2DArray t_Meta;

const uint
    c_DoubleLevelMask = 1U<<6,
    c_ShadowMask = 1U<<7;
const uint
    c_TerrainShift = 3U,
    c_TerrainBits = 3U;
const uint
    c_DeltaShift = 0U,
    c_DeltaBits = 2U;
const float c_DeltaScale = 8.0 / 255.0;

struct Surface {
    float low_alt, high_alt, delta;
    uint low_type, high_type;
    vec3 tex_coord;
    bool is_shadowed;
};

uint get_terrain_type(uint meta) {
    return (meta >> c_TerrainShift) & ((1U << c_TerrainBits) - 1U);
}
uint get_delta(uint meta) {
    return (meta >> c_DeltaShift) & ((1U << c_DeltaBits) - 1U);
}

Surface get_surface(vec2 pos) {
    Surface suf;

    vec3 tc = vec3(pos / u_TextureScale.xy, 0.0);
    tc.z = trunc(mod(tc.y, u_TextureScale.w));
    suf.tex_coord = tc;

    uint meta = texture(t_Meta, tc).x;
    suf.is_shadowed = (meta & c_ShadowMask) != 0U;
    suf.low_type = get_terrain_type(meta);

    if ((meta & c_DoubleLevelMask) != 0U) {
        //TODO: we need either low or high for the most part
        // so this can be more efficient with a boolean param
        uint delta;
        if (mod(pos.x, 2.0) >= 1.0) {
            uint meta_low = textureOffset(t_Meta, tc, ivec2(-1, 0)).x;
            suf.high_type = suf.low_type;
            suf.low_type = get_terrain_type(meta_low);

            delta = (get_delta(meta_low) << c_DeltaBits) + get_delta(meta);
        } else {
            uint meta_high = textureOffset(t_Meta, tc, ivec2(1, 0)).x;
            suf.tex_coord.x += 1.0 / u_TextureScale.x;
            suf.high_type = get_terrain_type(meta_high);

            delta = (get_delta(meta) << c_DeltaBits) + get_delta(meta_high);
        }

        suf.low_alt =
            textureOffset(t_Height, suf.tex_coord, ivec2(-1, 0)).x
            * u_TextureScale.z;
        suf.high_alt = texture(t_Height, suf.tex_coord).x * u_TextureScale.z;
        suf.delta = float(delta) * c_DeltaScale * u_TextureScale.z;
    } else {
        suf.high_type = suf.low_type;

        suf.low_alt = texture(t_Height, tc).x * u_TextureScale.z;
        suf.high_alt = suf.low_alt;
        suf.delta = 0.0;
    }

    return suf;
}
