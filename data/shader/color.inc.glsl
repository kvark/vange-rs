// Common FS routines for evaluating terrain color.

//uniform sampler2DArray t_Height;
// Terrain parameters per type: shadow offset, height shift, palette start, palette end
uniform usampler1D t_Table;
// corresponds to SDL palette
uniform sampler1D t_Palette;
// Flood map has the water level per Y.
uniform sampler1D t_Flood;

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
    vec4 terr = vec4(texelFetch(t_Table, int(type), 0));
    if (type == 0U && value > 0.0) { // water
        //TODO: apparently, the flood map data isn't correct...
        float flood = texture(t_Flood, 0.0*ycoord).x;
        float d = c_HorFactor * (1.0 - flood);
        value = clamp(value * 1.25 / (1.0 - d) - 0.25, 0.0, 1.0);
    }
    return (mix(terr.z, terr.w, value) + 0.5) / 256.0;
}

vec4 evaluate_color(uint type, vec3 tex_coord, float height_normalized, float lit_factor) {
    float diff =
        textureLodOffset(t_Height, tex_coord, 0.0, ivec2(1, 0)).x -
        textureLodOffset(t_Height, tex_coord, 0.0, ivec2(-1, 0)).x;
    vec3 mat = type == 0U ? vec3(5.0, 1.25, 0.5) : vec3(1.0);
    float light_clr = evaluate_light(mat, diff);
    float tmp = light_clr - c_HorFactor * (1.0 - height_normalized);
    float color_id = evaluate_palette(type, lit_factor * tmp, tex_coord.y);
    return texture(t_Palette, color_id);
}
