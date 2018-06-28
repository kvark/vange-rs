//!include quat.vert transform.vert

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
//imports: Transform, fetch_entry_transform, transform, qrot

uniform c_Locals {
    uvec4 u_Entry;
    vec4 u_PosScale;
    vec4 u_Rot;
};

uniform usampler1D t_ColorTable;
uniform sampler1D t_Palette;

attribute ivec4 a_Pos;
attribute vec4 a_Normal;
attribute uint a_ColorIndex;

void main() {
    Transform base = fetch_entry_transform(int(u_Entry.x));
    Transform local = Transform(u_PosScale.xyz, u_PosScale.w, u_Rot);

    vec3 world = transform(local, vec4(transform(base, a_Pos), 1.0));
    gl_Position = u_ViewProj * vec4(world, 1.0);

    uvec2 color_params = texelFetch(t_ColorTable, int(a_ColorIndex), 0).xy;
    v_Color = texelFetch(t_Palette, int(color_params[0]), 0);

    vec3 n = normalize(a_Normal.xyz);
    v_Normal = qrot(local.rot, qrot(base.rot, n));
    v_Light = u_LightPos.xyz - world * u_LightPos.w;  
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
