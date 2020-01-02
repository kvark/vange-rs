//!include vs:body.inc vs:globals.inc vs:quat.inc fs:globals.inc

layout(location = 0) varying vec4 v_Color;
layout(location = 1) varying vec3 v_Normal;
layout(location = 2) varying vec3 v_Light;

#ifdef SHADER_VS

const uint BODY_COLOR_ID = 1;

layout(set = 1, binding = 0) uniform utexture1D t_ColorTable;
layout(set = 1, binding = 1) uniform texture1D t_Palette;
layout(set = 1, binding = 2) uniform sampler s_ColorTableSampler;

layout(set = 0, binding = 1) uniform sampler s_PaletteSampler;
layout(set = 0, binding = 2, std430) readonly buffer Storage {
    Body s_Bodies[];
};

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
    uvec2 color_params = texelFetch(usampler1D(t_ColorTable, s_ColorTableSampler), int(color_id), 0).xy;
    uint palette_coord = color_params.x + (128U >> color_params.y) - 1U;
    v_Color = texelFetch(sampler1D(t_Palette, s_PaletteSampler), int(palette_coord), 0);

    vec3 n = normalize(a_Normal.xyz);
    v_Normal = qrot(body_orientation, qrot(a_Orientation, n));
    v_Light = u_LightPos.xyz - world * u_LightPos.w;
}
#endif //VS


#ifdef SHADER_FS

const float c_Emissive = 0.0, c_Ambient = 0.3, c_Diffuse = 0.5;

layout(location = 0) out vec4 o_Color;

void main() {
    vec3 normal = normalize(v_Normal) * (gl_FrontFacing ? -1.0 : 1.0);
    vec3 light_dir = normalize(v_Light);
    float n_dot_l = max(0.0, dot(normal, light_dir));
    float kd = c_Ambient + c_Diffuse * n_dot_l;

    o_Color = v_Color * (c_Emissive + kd * u_LightColor);
}
#endif //FS
