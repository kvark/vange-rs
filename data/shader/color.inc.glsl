// Common FS routines for evaluating terrain color.

//TODO: provide only minimum information to
// reconstruct the lighting model in shader code.

//uniform sampler2DArray t_Height;
// This texture has a layer per terrain type. First row used to be `lightCLR`, now zeroes. The second row is `palCLR`.
uniform sampler2DArray t_Table;
// corresponds to SDL palette
uniform sampler1D t_Palette;
// Information about materials. Each row has palette start, palette end, dx_scale, sd_scale, jj_scale
//uniform sampler2D t_Material;

// Flood map has the water level per Y.
uniform sampler1D t_Flood;

const float c_HorFactor = 0.5; //H_CORRECTION
const float c_DiffuseScale = 8.0;
const float c_ShadowDepthScale = 2.0 / 3.0;

// material coefficients are called "dx", "sd" and "jj" in the original
// see `RenderPrepare` in `land.cpp`
float evaluate_light(vec3 mat, float height_diff) {
    float dx = mat.x * c_DiffuseScale;
    float sd = mat.y * c_ShadowDepthScale;
    float jj = mat.z * height_diff * 256.0;
    float v = (dx * sd - jj) / sqrt((1.0 + sd * sd) * (dx * dx + jj * jj));
    return clamp(v, 0.0, 1.0);
}

vec4 evaluate_color(uint type, vec3 tex_coord, float height_normalized, float lit_factor) {
    float diff =
        textureOffset(t_Height, tex_coord, ivec2(1, 0)).x
        - textureOffset(t_Height, tex_coord, ivec2(-1, 0)).x;
    vec3 mat = type == 0U ? vec3(5.0, 1.25, 0.5) : vec3(1.0);
    float light_clr = evaluate_light(mat, diff);
    float tmp = light_clr - c_HorFactor * (1.0 - height_normalized);
    float color_id =
        texture(t_Table, vec3(0.5 * lit_factor * tmp + 0.5, 0.75, float(type))).x;

    return texture(t_Palette, color_id);
}
