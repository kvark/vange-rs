uniform c_Globals {
    vec4 u_CameraPos;
    mat4 u_ViewProj;
    mat4 u_InvViewProj;
    vec4 u_LightPos;
    vec4 u_LightColor;
};

varying vec4 v_Color;
varying vec3 v_Normal;
varying vec3 v_Light;


#ifdef SHADER_VS

uniform c_Locals {
    mat4 u_Model;
};

uniform usampler1D t_ColorTable;
uniform sampler1D t_Palette;

attribute ivec4 a_Pos;
attribute vec4 a_Normal;
attribute uint a_ColorIndex;

void main() {
    vec4 world = u_Model * a_Pos;
    gl_Position = u_ViewProj * world;

    uvec2 color_params = texelFetch(t_ColorTable, int(a_ColorIndex), 0).xy;
    v_Color = texelFetch(t_Palette, int(color_params[0]), 0);

    vec3 n = normalize(a_Normal.xyz);
    v_Normal = mat3(u_Model) * n;
    v_Light = u_LightPos.xyz - world.xyz * u_LightPos.w;  
}
#endif //VS


#ifdef SHADER_FS

const float c_Emissive = 0.3, c_Ambient = 0.5, c_Diffuse = 3.0;

out vec4 Target0;

void main() {
    vec3 normal = normalize(v_Normal) * (gl_FrontFacing ? -1.0 : 1.0);
    vec3 light_dir = normalize(v_Light);
    float n_dot_l = max(0.0, dot(normal, light_dir));
    float kd = c_Ambient + c_Diffuse * n_dot_l;

    Target0 = v_Color * (c_Emissive + kd * u_LightColor);
}
#endif //FS
