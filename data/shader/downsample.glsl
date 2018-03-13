flat varying ivec4 v_SourceRect;
flat varying ivec4 v_DestRect;


#ifdef SHADER_VS

attribute uvec4 a_SourceRect;
attribute uvec4 a_DestRect;

uniform vec2 u_DestSize;

void main() {
    vec2 pos;
    switch (gl_VertexID) {
        case 1: pos = vec2(1.0, 0.0); break;
        case 2: pos = vec2(0.0, 1.0); break;
        case 3: pos = vec2(1.0, 1.0); break;
        default: pos = vec2(0.0);
    }
    vec2 fpos = (vec2(a_DestRect.xy) + pos * vec2(a_DestRect.zw)) / u_DestSize;
    gl_Position = vec4(fpos * 2.0 - 1.0, 0.0, 1.0);

    v_SourceRect = ivec4(a_SourceRect);
    v_DestRect = ivec4(a_DestRect);
}
#endif //VS


#ifdef SHADER_FS

uniform sampler2D t_Source;

out vec4 Target0;

void main() {
    // the absolute coordinates of the base texel of 2x2 grid
    ivec2 frag_offset = ivec2(gl_FragCoord.xy) - v_DestRect.xy;
    ivec2 c00 = frag_offset * (v_SourceRect.zw + 1) / v_DestRect.zw + v_SourceRect.xy;
    vec4 t00 = texelFetch(t_Source, c00, 0);

    if (v_SourceRect.z <= v_DestRect.z) {
        // handle debug up-scaling (rough, can be improved)
        Target0 = t00;
    } else {
        // the offset is 0 on the edge of the source rectangle, duplicating the texels
        vec2 mask = min(vec2(1.0), vec2(v_SourceRect.xy + v_SourceRect.zw - c00 - 1));

        // all 4 texels of the grid to downsample
        vec4 t10 = texelFetchOffset(t_Source, c00, 0, ivec2(1, 0));
        vec4 t01 = texelFetchOffset(t_Source, c00, 0, ivec2(0, 1));
        vec4 t11 = texelFetchOffset(t_Source, c00, 0, ivec2(1, 1));

        Target0 = t00 + mask.x * t10 + mask.y * t01 + mask.x * mask.y * t11;
    }
}
#endif //FS
