//!include vs:body.inc vs:globals.inc vs:quat.inc fs:globals.inc

layout(location = 0) varying vec4 v_Color;
layout(location = 1) varying vec3 v_Normal;
layout(location = 2) varying vec3 v_Light;


#ifdef SHADER_VS

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
layout(location = 6) attribute uint a_BodyId;

void main() {
    vec4 body_pos_scale = s_Bodies[int(a_BodyId)].pos_scale;
    vec4 body_orientation = s_Bodies[int(a_BodyId)].orientation;

    vec3 local = qrot(a_Orientation, vec3(a_Vertex.xyz)) * a_PosScale.w + a_PosScale.xyz;
    vec3 world = qrot(body_orientation, local) * body_pos_scale.w + body_pos_scale.xyz;
    gl_Position = u_ViewProj * vec4(world, 1.0);

    uvec2 color_params = texelFetch(usampler1D(t_ColorTable, s_ColorTableSampler), int(a_ColorIndex), 0).xy;
    v_Color = texelFetch(sampler1D(t_Palette, s_PaletteSampler), int(color_params[0]), 0);

    vec3 n = normalize(a_Normal.xyz);
    v_Normal = qrot(body_orientation, qrot(a_Orientation, n));
    v_Light = u_LightPos.xyz - world * u_LightPos.w;
}
#endif //VS


#ifdef SHADER_FS

const float c_Emissive = 0.3, c_Ambient = 0.5, c_Diffuse = 3.0;

layout(location = 0) out vec4 o_Color;

void main() {
    vec3 normal = normalize(v_Normal) * (gl_FrontFacing ? -1.0 : 1.0);
    vec3 light_dir = normalize(v_Light);
    float n_dot_l = max(0.0, dot(normal, light_dir));
    float kd = c_Ambient + c_Diffuse * n_dot_l;

    o_Color = v_Color * (c_Emissive + kd * u_LightColor);
}
#endif //FS
