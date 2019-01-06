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

const vec3 c_GroundMaterial = vec3(1.0);
const vec3 c_WaterMaterial = vec3(5.0, 1.25, 0.5);

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
    if (type == 0U && value > 0.0) { // water
        float flood = texture(t_Flood, ycoord).x;
        float d = c_HorFactor * (1.0 - flood);
        value = clamp(value * 1.25 / (1.0 - d) - 0.25, 0.0, 1.0);
    }

    vec4 terr = texelFetch(t_Table, int(type), 0);
    return (mix(terr.z, terr.w, value) + 0.5) / 256.0;
}

vec4 evaluate_color(uint type, vec3 tex_coord, float height_normalized, float lit_factor) {
    float diff =
        textureLodOffset(t_Height, tex_coord, 0.0, ivec2(1, 0)).x -
        textureLodOffset(t_Height, tex_coord, 0.0, ivec2(-1, 0)).x;
    vec3 mat = type == 0U ? c_WaterMaterial : c_GroundMaterial;
    float light_clr = evaluate_light(mat, diff);
    float tmp = light_clr - c_HorFactor * (1.0 - height_normalized);
    float color_id = evaluate_palette(type, lit_factor * tmp, tex_coord.y);
    return texture(t_Palette, color_id);
}

// Versions of the same functions but for 2x2 quads, which is useful to do
// bilinear filtering across different materials

vec4 evaluate_palette4(uvec4 types, vec4 values, float ycoord) {
    values = clamp(values, 0.0, 1.0);
    if (dot(values, values) > 0.0) {
        float flood = texture(t_Flood, ycoord).x;
        float d = c_HorFactor * (1.0 - flood);
        vec4 water_values = clamp(values * 1.25 / (1.0 - d) - 0.25, 0.0, 1.0);
        values = mix(values, water_values, equal(types, uvec4(0U)));
    }

    vec4 t0 = texelFetch(t_Table, int(types.x), 0);
    vec4 t1 = texelFetch(t_Table, int(types.y), 0);
    vec4 t2 = texelFetch(t_Table, int(types.z), 0);
    vec4 t3 = texelFetch(t_Table, int(types.w), 0);
    vec4 terr_z = vec4(t0.z, t1.z, t2.z, t3.z);
    vec4 terr_w = vec4(t0.w, t1.w, t2.w, t3.w);

    return (mix(terr_z, terr_w, values) + 0.5) / 256.0;
}

mat4 evaluate_color4(uvec4 types, vec3 tex_coord, float height_normalized, vec4 lit_factors) {
    float diff =
        textureOffset(t_Height, tex_coord, ivec2(1, 0)).x -
        textureOffset(t_Height, tex_coord, ivec2(-1, 0)).x;
    float light_clr_ground = evaluate_light(c_GroundMaterial, diff);
    float light_clr_water = evaluate_light(c_WaterMaterial, diff);
    vec4 light_clr = mix(vec4(light_clr_ground), vec4(light_clr_water), equal(types, uvec4(0U)));
    vec4 tmp = light_clr - c_HorFactor * vec4(1.0 - height_normalized);
    vec4 color_ids = evaluate_palette4(types, lit_factors * tmp, tex_coord.y);
    return mat4(
        texture(t_Palette, color_ids.x),
        texture(t_Palette, color_ids.y),
        texture(t_Palette, color_ids.z),
        texture(t_Palette, color_ids.w)
    );
}
