layout(set = 1, binding = 1) uniform c_Locals {
    uvec4 u_ScreenSize;      // XY = size
    uvec4 u_Params;
    vec4 u_CamOriginDir;	// XY = origin, ZW = dir
    vec4 u_SampleRange;		// XY = X range, ZW = y range
    vec4 u_FogColor;
    vec4 u_FogParams;       // X=near, Y = far
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

vec4 apply_fog(vec4 terrain_color, vec2 world_pos) {
    float cam_distance = clamp(length(world_pos - u_CamOriginDir.xy), u_FogParams.x, u_FogParams.y);
    float fog_amount = smoothstep(u_FogParams.x, u_FogParams.y, cam_distance);
    return mix(terrain_color, u_FogColor, fog_amount);
}
#endif //FS
