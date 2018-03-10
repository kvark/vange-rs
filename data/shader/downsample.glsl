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
    ivec2 c00 = (ivec2(gl_FragCoord.xy) - v_DestRect.xy) * 2 + v_SourceRect.xy;
    // the offset is 0 on the edge of the source rectangle, duplicating the texels
    ivec2 off = min(ivec2(1), v_SourceRect.xy + v_SourceRect.zw - c00 - 1);

    // all 4 texels of the grid to downsample
    vec4 t0 = texelFetch(t_Source, c00, 0);
    vec4 t1 = texelFetch(t_Source, c00 + ivec2(0, off.y), 0);
    vec4 t2 = texelFetch(t_Source, c00 + ivec2(off.x, 0), 0);
    vec4 t3 = texelFetch(t_Source, c00 + off, 0);

    // simple bitonic sort according to the polygon ID
    if (t0.w > t2.w) {
        vec4 t = t0; t0 = t2; t2 = t;
    }
    if (t1.w > t3.w) {
        vec4 t = t1; t1 = t3; t3 = t;
    }
    if (t0.w > t1.w) {
        vec4 t = t0; t0 = t1; t1 = t;
    }
    if (t2.w > t3.w) {
        vec4 t = t2; t2 = t3; t3 = t;
    }
    if (t1.w > t2.w) {
        vec4 t = t1; t1 = t2; t2 = t;
    }

    // now average across the same polygon ID
    //TODO: this is not ideal, merging polygon ID is difficult
    if (t0.w == t1.w) {
        if (t1.w == t2.w) {
            if (t2.w == t3.w) {
                Target0 = vec4((t0.xyz + t1.xyz + t2.xyz + t3.xyz) / 4.0, t0.w);
            } else {
                Target0 = vec4((t0.xyz + t1.xyz + t2.xyz) / 3.0 + t3.xyz, t0.w);
            }
        } else {
            if (t2.w == t3.w) {
                Target0 = vec4((t0.xyz + t1.xyz) / 2.0 + (t2.xyz + t3.xyz) / 2.0, t0.w);
            } else {
                Target0 = vec4((t0.xyz + t1.xyz) / 2.0 + t2.xyz + t3.xyz, t0.w);
            }
        }
    } else {
        if (t1.w == t2.w) {
            if (t2.w == t3.w) {
                Target0 = vec4(t0.xyz + (t1.xyz + t2.xyz + t3.xyz) / 3.0, t1.w);
            } else {
                Target0 = vec4(t0.xyz + (t1.xyz + t2.xyz) / 2.0 + t3.xyz, t1.w);
            }
        } else {
            if (t2.w == t3.w) {
                Target0 = vec4(t0.xyz + t1.xyz + (t2.xyz + t3.xyz) / 2.0, t2.w);
            } else {
                Target0 = vec4(t0.xyz + t1.xyz + t2.xyz + t3.xyz, t0.w);
            }
        }
    }
}
#endif //FS
