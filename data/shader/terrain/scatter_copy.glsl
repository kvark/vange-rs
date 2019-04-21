#ifdef SHADER_VS

void main() {
    vec2 pos = vec2(0.0);
    switch (gl_VertexIndex) {
        case 0: pos = vec2(1.0, -1.0); break;
        case 1: pos = vec2(1.0, 1.0); break;
        case 2: pos = vec2(-1.0, -1.0); break;
        case 3: pos = vec2(-1.0, 1.0); break;
        default: break;
    }
    gl_Position = vec4(pos, 0.0, 1.0);
}
#endif //VS


#ifdef SHADER_FS

layout(set = 0, binding = 1) uniform sampler s_PaletteSampler;
layout(set = 1, binding = 6) uniform texture1D t_Palette;
layout(set = 2, binding = 0, r32ui) uniform readonly uimage2D i_Input;

layout(location = 0) out vec4 o_Color;

void main() {
    uint value = imageLoad(i_Input, ivec2(gl_FragCoord.xy)).x;
    o_Color = texelFetch(sampler1D(t_Palette, s_PaletteSampler), int(value & 0xFFU), 0);
    gl_FragDepth = float(value >> 8U) / float(0xFFFFFF);
}
#endif //FS
