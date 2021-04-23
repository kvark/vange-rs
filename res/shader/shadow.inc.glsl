/// Shadow sampling.

#ifdef SHADER_FS

layout(set = 0, binding = 3) uniform texture2D u_ShadowTexture;
layout(set = 0, binding = 4) uniform samplerShadow u_ShadowSampler;

const float c_Ambient = 0.25;

float fetch_shadow(vec3 pos) {
    const vec2 flip_correction = vec2(1.0, -1.0);

    vec4 homogeneous_coords = u_LightViewProj * vec4(pos, 1.0);
    if (homogeneous_coords.w <= 0.0) {
        return 0.0;
    }
    vec3 light_local = vec3(
        0.5 * (homogeneous_coords.xy * flip_correction/homogeneous_coords.w + 1.0),
        homogeneous_coords.z / homogeneous_coords.w
    );
    float shadow = textureLod(
        sampler2DShadow(u_ShadowTexture, u_ShadowSampler),
        light_local.xyz,
        0.0
    );
    return mix(c_Ambient, 1.0, shadow);
}

#endif //FS
