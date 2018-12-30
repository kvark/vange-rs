#extension GL_ARB_texture_gather : enable

varying vec2 v_TexCoord;

#ifdef SHADER_VS

attribute vec4 a_Pos;

void main() {
    v_TexCoord = a_Pos.xy;
    gl_Position = vec4(a_Pos.xy * 2.0 - 1.0, 0.0, 1.0);
}
#endif //VS


#ifdef SHADER_FS

uniform sampler2D t_HeightLayer;

out float Target0;

void main() {
    vec4 heights = textureGather(t_HeightLayer, v_TexCoord);
    Target0 = max(max(heights.x, heights.y), max(heights.z, heights.w));
}
#endif //FS
