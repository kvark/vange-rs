layout(set = 1, binding = 1) uniform c_Locals {
    uvec4 u_ScreenSize;      // XY = size
    uvec4 u_Params;
    vec4 u_CamOriginDir;	// XY = origin, ZW = dir
    vec4 u_SampleRange;		// XY = X range, ZW = y range
};

#ifdef SHADER_FS
vec4 get_frag_ndc(float z) {
    // note the Y-flip here
    return vec4(
        ((gl_FragCoord.xy / vec2(u_ScreenSize.xy)) * 2.0 - 1.0) * vec2(1.0, -1.0),
        z,
        1.0
    );
}

vec3 get_frag_world(float z) {
    vec4 ndc = get_frag_ndc(z);
    vec4 homogeneous = u_InvViewProj * ndc;
    return homogeneous.xyz / homogeneous.w;
}
#endif //FS
