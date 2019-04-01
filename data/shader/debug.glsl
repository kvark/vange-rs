layout(set = 0, binding = 0) uniform c_Globals {
    vec4 u_CameraPos;
    mat4 u_ViewProj;
    mat4 u_InvViewProj;
    vec4 u_LightPos;
    vec4 u_LightColor;
};

layout(location = 0) varying vec4 v_Color;

#ifdef SHADER_VS

layout(set = 1, binding = 0) uniform c_Debug {
    vec4 u_Color;
};

layout(location = 0) in vec4 a_Pos;
layout(location = 1) in vec4 a_Color;

void main() {
    gl_Position = u_ViewProj * a_Pos;
    v_Color = a_Color;
}
#endif //VS


#ifdef SHADER_FS

layout(location = 0) out vec4 o_Color;

void main() {
    o_Color = v_Color;
}
#endif //FS
