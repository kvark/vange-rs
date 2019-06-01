// Common routines for fetching the level surface data.

layout(set = 1, binding = 0) uniform c_Surface {
    vec4 u_TextureScale;    // XY = size, Z = height scale, w = number of layers
};

layout(set = 1, binding = 2) uniform texture2D t_Height;
layout(set = 1, binding = 3) uniform utexture2D t_Meta;
layout(set = 1, binding = 7) uniform sampler s_MainSampler;

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
    vec2 tex_coord;
    bool is_shadowed;
};

uint get_terrain_type(uint meta) {
    return (meta >> c_TerrainShift) & ((1U << c_TerrainBits) - 1U);
}
uint get_delta(uint meta) {
    return (meta >> c_DeltaShift) & ((1U << c_DeltaBits) - 1U);
}

int modulo(int a, int b) {
    int c = a % b;
    return c < 0 ? c + b : c;
}

float get_lod_height(ivec2 ipos, int lod) {
    int x = modulo(ipos.x, int(u_TextureScale.x));
    int y = modulo(ipos.y, int(u_TextureScale.y));
    ivec2 tc = ivec2(x, y) >> lod;
    float alt = texelFetch(sampler2D(t_Height, s_MainSampler), tc, lod).x;
    return alt * u_TextureScale.z;
}

// The alternative version of this routine that doesn't use
// integer operations or `texelFetch`.
float get_lod_height_alt(ivec2 ipos, int lod) {
    vec2 xy = (vec2(ipos.xy) + 0.5) / u_TextureScale.xy;
    float alt = textureLod(sampler2D(t_Height, s_MainSampler), xy, float(lod)).x;
    return alt * u_TextureScale.z;
}

Surface get_surface(vec2 pos) {
    Surface suf;

    vec2 tc = suf.tex_coord = pos / u_TextureScale.xy;
    ivec2 tci = ivec2(mod(pos, u_TextureScale.xy));

    uint meta = texelFetch(usampler2D(t_Meta, s_MainSampler), tci, 0).x;
    suf.is_shadowed = (meta & c_ShadowMask) != 0U;
    suf.low_type = get_terrain_type(meta);

    if ((meta & c_DoubleLevelMask) != 0U) {
        //TODO: we need either low or high for the most part
        // so this can be more efficient with a boolean param
        uint delta;
        if (mod(pos.x, 2.0) >= 1.0) {
            uint meta_low = texelFetch(usampler2D(t_Meta, s_MainSampler), tci + ivec2(-1, 0), 0).x;
            suf.high_type = suf.low_type;
            suf.low_type = get_terrain_type(meta_low);

            delta = (get_delta(meta_low) << c_DeltaBits) + get_delta(meta);
        } else {
            uint meta_high = texelFetch(usampler2D(t_Meta, s_MainSampler), tci + ivec2(1, 0), 0).x;
            suf.tex_coord.x += 1.0 / u_TextureScale.x;
            suf.high_type = get_terrain_type(meta_high);

            delta = (get_delta(meta) << c_DeltaBits) + get_delta(meta_high);
        }

        suf.low_alt = //TODO: the `LodOffset` doesn't appear to work in Metal compute
            //textureLodOffset(sampler2D(t_Height, s_MainSampler), suf.tex_coord, 0.0, ivec2(-1, 0)).x
            textureLod(sampler2D(t_Height, s_MainSampler), suf.tex_coord - vec2(1.0 / u_TextureScale.x, 0.0), 0.0).x
            * u_TextureScale.z;
        suf.high_alt = textureLod(sampler2D(t_Height, s_MainSampler), suf.tex_coord, 0.0).x * u_TextureScale.z;
        suf.delta = float(delta) * c_DeltaScale * u_TextureScale.z;
    } else {
        suf.high_type = suf.low_type;

        suf.low_alt = textureLod(sampler2D(t_Height, s_MainSampler), tc, 0.0).x * u_TextureScale.z;
        suf.high_alt = suf.low_alt;
        suf.delta = 0.0;
    }

    return suf;
}
