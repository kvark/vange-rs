layout(set = 1, binding = 1) uniform c_Locals {
    uvec4 u_ScreenSize;      // XY = size
    uvec4 u_Params;
    vec4 u_CamOriginDir;	// XY = origin, ZW = dir
    vec4 u_SampleRange;		// XY = X range, ZW = y range
};

vec4 get_frag_ndc() {
    // note the Y-flip here
    return vec4(
        ((gl_FragCoord.xy / vec2(u_ScreenSize.xy)) * 2.0 - 1.0) * vec2(1.0, -1.0),
        0.0,
        1.0
    );
}

vec2 generate_scatter_pos(vec2 source_coord) {
    float y;
    if (true) {
        float y_sqrt = mix(
            sqrt(abs(u_SampleRange.z)) * sign(u_SampleRange.z),
            sqrt(abs(u_SampleRange.w)) * sign(u_SampleRange.w),
            source_coord.y
        );
        y = y_sqrt * y_sqrt * sign(y_sqrt);
    } else {
        y = mix(u_SampleRange.z, u_SampleRange.w, source_coord.y);
    }

    float x_limit = mix(
        u_SampleRange.x, u_SampleRange.y,
        (y - u_SampleRange.z) / (u_SampleRange.w - u_SampleRange.z)
    );
    float x = mix(-x_limit, x_limit, source_coord.x);

    return u_CamOriginDir.xy + u_CamOriginDir.zw * y +
        vec2(u_CamOriginDir.w, -u_CamOriginDir.z) * x;
}
