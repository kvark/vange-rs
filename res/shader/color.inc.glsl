// Common FS routines for evaluating terrain color.

//uniform sampler2D t_Height;
// Flood map has the water level per Y.
layout(set = 1, binding = 4) uniform texture1D t_Flood;
// Terrain parameters per type: shadow offset, height shift, palette start, palette end
layout(set = 1, binding = 5) uniform utexture1D t_Table;
// corresponds to SDL palette
layout(set = 1, binding = 6) uniform texture1D t_Palette;
layout(set = 1, binding = 8) uniform sampler s_FloodSampler;

layout(set = 0, binding = 1) uniform sampler s_PaletteSampler;

const float c_HorFactor = 0.5; //H_CORRECTION
const float c_DiffuseScale = 8.0;
const float c_ShadowDepthScale = 2.0 / 3.0;

// see `RenderPrepare` in `land.cpp` for the original game logic

// material coefficients are called "dx", "sd" and "jj" in the original
float evaluate_light(vec3 mat, float height_diff) {
    float dx = mat.x * c_DiffuseScale;
    float sd = mat.y * c_ShadowDepthScale;
    float jj = mat.z * height_diff * 256.0;
    float v = (dx * sd - jj) / sqrt((1.0 + sd * sd) * (dx * dx + jj * jj));
    return clamp(v, 0.0, 1.0);
}

float evaluate_palette(uint type, float value, float ycoord) {
    value = clamp(value, 0.0, 1.0);
    vec4 terr = vec4(texelFetch(usampler1D(t_Table, s_PaletteSampler), int(type), 0));
    if (type == 0U && value > 0.0) { // water
        float flood = textureLod(sampler1D(t_Flood, s_FloodSampler), ycoord, 0.0).x;
        float d = c_HorFactor * (1.0 - flood);
        value = clamp(value * 1.25 / (1.0 - d) - 0.25, 0.0, 1.0);
    }
    return (mix(terr.z, terr.w, value) + 0.5) / 256.0;
}

float evaluate_color_id(uint type, vec2 tex_coord, float height_normalized, float lit_factor) {
    float diff =
        textureLodOffset(sampler2D(t_Height, s_MainSampler), tex_coord, 0.0, ivec2(1, 0)).x -
        textureLodOffset(sampler2D(t_Height, s_MainSampler), tex_coord, 0.0, ivec2(-1, 0)).x;
    vec3 mat = type == 0U ? vec3(5.0, 1.25, 0.5) : vec3(1.0);
    float light_clr = evaluate_light(mat, diff);
    float tmp = light_clr - c_HorFactor * (1.0 - height_normalized);
    return evaluate_palette(type, lit_factor * tmp, tex_coord.y);
}

vec4 evaluate_color(uint type, vec2 tex_coord, float height_normalized, float lit_factor) {
    float color_id = evaluate_color_id(type, tex_coord, height_normalized, lit_factor);
    return texture(sampler1D(t_Palette, s_PaletteSampler), color_id);
}
