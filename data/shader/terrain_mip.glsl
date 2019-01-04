#if GL_ARB_texture_gather
#extension GL_ARB_texture_gather : enable
#endif

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

out float Target0;

void main() {
    #if GL_ARB_texture_gather
    vec4 heights = textureGather(t_Height, v_TexCoord);
    #else
    // we are at a pixel center, so a slight offset guarantees
    // a sample from a particular neighbor in 2x2 grid around the center
    float delta = 0.00001;
    vec4 heights = vec4(
        texture(t_Height, v_TexCoord + vec3(-delta, -delta, 0.0)).r,
        texture(t_Height, v_TexCoord + vec3(delta, -delta, 0.0)).r,
        texture(t_Height, v_TexCoord + vec3(delta, delta, 0.0)).r,
        texture(t_Height, v_TexCoord + vec3(-delta, delta, 0.0)).r
    );
    #endif
    Target0 = max(max(heights.x, heights.y), max(heights.z, heights.w));
}
#endif //FS
