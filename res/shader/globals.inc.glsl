layout(set = 0, binding = 0) uniform Globals {
    vec4 u_CameraPos;
    mat4 u_ViewProj;
    mat4 u_InvViewProj;
    mat4 u_LightViewProj;
    vec4 u_LightPos;
    vec4 u_LightColor; // not used
};
