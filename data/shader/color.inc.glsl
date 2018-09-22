// Common FS routines for evaluating terrain color.

//uniform sampler2DArray t_Height;
uniform sampler2DArray t_Table;
uniform sampler1D t_Palette;

const float c_HorFactor = 0.5; //H_CORRECTION

vec4 evaluate_color(uint type, vec3 tex_coord, float height_normalized, float lit_factor) {
    float terrain = float(type) + 0.5;
    float diff =
        textureOffset(t_Height, tex_coord, ivec2(1, 0)).x
        - textureOffset(t_Height, tex_coord, ivec2(-1, 0)).x;
    float light_clr =
        texture(t_Table, vec3(0.5 * diff + 0.5, 0.25, terrain)).x;
    float tmp =
        light_clr - c_HorFactor * (1.0 - height_normalized);
    float color_id =
        texture(t_Table, vec3(0.5 * lit_factor * tmp + 0.5, 0.75, terrain)).x;

    return texture(t_Palette, color_id);
}
