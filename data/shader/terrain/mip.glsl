#ifdef SHADER_VS

layout(location = 0) in vec2 a_Pos;

void main() {
    gl_Position = vec4(2.0 * a_Pos - 1.0, 0.0, 1.0);
}
#endif //VS


#ifdef SHADER_FS

layout(set = 0, binding = 0) uniform sampler s_Height;
layout(set = 0, binding = 1) uniform texture2D t_Height;

layout(location = 0) out float o_Height;

void main() {
    ivec2 tc = ivec2(gl_FragCoord.xy * 2.0);
    vec4 heights = vec4(
        texelFetch(sampler2D(t_Height, s_Height), tc - ivec2(0, 0), 0).x,
        texelFetch(sampler2D(t_Height, s_Height), tc - ivec2(0, 1), 0).x,
        texelFetch(sampler2D(t_Height, s_Height), tc - ivec2(1, 0), 0).x,
        texelFetch(sampler2D(t_Height, s_Height), tc - ivec2(1, 1), 0).x
    );
    o_Height = max(max(heights.x, heights.y), max(heights.z, heights.w));
}
#endif //FS
