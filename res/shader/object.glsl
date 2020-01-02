//!include vs:body.inc vs:globals.inc vs:quat.inc fs:globals.inc

layout(location = 0) varying vec2 v_PaletteRange;
layout(location = 1) varying vec3 v_Normal;
layout(location = 2) varying vec3 v_Light;

#ifdef SHADER_VS

const uint BODY_COLOR_ID = 1;

layout(set = 0, binding = 2, std430) readonly buffer Storage {
    Body s_Bodies[];
};

layout(set = 1, binding = 0) uniform utexture1D t_ColorTable;
layout(set = 1, binding = 2) uniform sampler s_ColorTableSampler;

layout(location = 0) attribute ivec4 a_Vertex;
layout(location = 1) attribute uint a_ColorIndex;
layout(location = 2) attribute vec4 a_Normal;

layout(location = 3) attribute vec4 a_PosScale;
layout(location = 4) attribute vec4 a_Orientation;
layout(location = 6) attribute uvec2 a_BodyAndColorId;

void main() {
    int body_id = int(a_BodyAndColorId.x);
    vec4 body_pos_scale = s_Bodies[body_id].pos_scale;
    vec4 body_orientation = s_Bodies[body_id].orientation;

    vec3 local = qrot(a_Orientation, vec3(a_Vertex.xyz)) * a_PosScale.w + a_PosScale.xyz;
    vec3 world = qrot(body_orientation, local) * body_pos_scale.w + body_pos_scale.xyz;
    gl_Position = u_ViewProj * vec4(world, 1.0);

    uint color_id = a_ColorIndex == BODY_COLOR_ID ? a_BodyAndColorId.y : a_ColorIndex;
    uvec2 range = texelFetch(usampler1D(t_ColorTable, s_ColorTableSampler), int(color_id), 0).xy;
    v_PaletteRange = vec2(range.x, range.x + (128U >> range.y));

    vec3 n = normalize(a_Normal.xyz);
    v_Normal = qrot(body_orientation, qrot(a_Orientation, n));
    v_Light = u_LightPos.xyz - world * u_LightPos.w;
}
#endif //VS


#ifdef SHADER_FS

layout(set = 0, binding = 1) uniform sampler s_PaletteSampler;
layout(set = 1, binding = 1) uniform texture1D t_Palette;

layout(location = 0) out vec4 o_Color;

void main() {
    vec3 normal = normalize(v_Normal) * (gl_FrontFacing ? -1.0 : 1.0);
    float n_dot_l = max(0.0, dot(normal, normalize(v_Light)));
    float tc_raw = mix(v_PaletteRange.x, v_PaletteRange.y, n_dot_l);
    float tc = clamp(tc_raw, v_PaletteRange.x + 0.5, v_PaletteRange.y - 0.5) / 256.0;
    o_Color = texture(sampler1D(t_Palette, s_PaletteSampler), tc);
}
#endif //FS
