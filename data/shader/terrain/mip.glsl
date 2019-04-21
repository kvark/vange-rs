varying vec3 v_TexCoord;

#ifdef SHADER_VS

attribute vec4 a_Pos;

void main() {
    v_TexCoord = a_Pos.xyz;
    gl_Position = vec4(a_Pos.xy * 2.0 - 1.0, 0.0, 1.0);
}
#endif //VS


#ifdef SHADER_FS

uniform sampler2DArray t_Height;

uniform c_Surface {
    vec4 u_TextureScale;    // XY = source size, Z = source mipmap level, W = 1
};


out float Target0;

void main() {
    ivec3 tc = ivec3(u_TextureScale.xyw * v_TexCoord);
    int lod = int(u_TextureScale.z);
    vec4 heights = vec4(
        texelFetch(t_Height, tc - ivec3(0, 0, 0), lod).x,
        texelFetch(t_Height, tc - ivec3(0, 1, 0), lod).x,
        texelFetch(t_Height, tc - ivec3(1, 0, 0), lod).x,
        texelFetch(t_Height, tc - ivec3(1, 1, 0), lod).x
    );
    Target0 = max(max(heights.x, heights.y), max(heights.z, heights.w));
}
#endif //FS
