layout(set = 0, binding = 0) uniform Globals {
    vec4 u_CameraPos;
    mat4 u_ViewProj;
    mat4 u_InvViewProj;
    vec4 u_LightPos;
    vec4 u_LightColor;
};

layout(location = 0) varying vec4 v_Color;
layout(location = 1) varying vec3 v_Normal;
layout(location = 2) varying vec3 v_Light;


#ifdef SHADER_VS

layout(set = 2, binding = 0) uniform c_Locals {
    mat4 u_Model;
};

layout(set = 1, binding = 0) uniform utexture1D t_ColorTable;
layout(set = 1, binding = 1) uniform texture1D t_Palette;
layout(set = 1, binding = 2) uniform sampler s_ColorTableSampler;

layout(set = 0, binding = 1) uniform sampler s_PaletteSampler;

layout(location = 0) attribute ivec4 a_Pos;
layout(location = 1) attribute uint a_ColorIndex;
layout(location = 2) attribute vec4 a_Normal;

void main() {
    vec4 world = u_Model * a_Pos;
    gl_Position = u_ViewProj * world;

    uvec2 color_params = texelFetch(usampler1D(t_ColorTable, s_ColorTableSampler), int(a_ColorIndex), 0).xy;
    v_Color = texelFetch(sampler1D(t_Palette, s_PaletteSampler), int(color_params[0]), 0);

    vec3 n = normalize(a_Normal.xyz);
    v_Normal = mat3(u_Model) * n;
    v_Light = u_LightPos.xyz - world.xyz * u_LightPos.w;  
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
