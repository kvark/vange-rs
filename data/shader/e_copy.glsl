flat varying float v_Y;

#ifdef SHADER_VS

attribute vec4 a_Entry;

void main() {
    float x = (gl_VertexID == 0 ? -1.0 : 1.0);
    gl_Position = vec4(x, a_Entry.x, 0.0, 1.0);
    v_Y = a_Entry.x * 0.5 + 0.5;
}
#endif //VS


#ifdef SHADER_FS

uniform sampler2D t_Entries;

out vec4 Target0;

void main() {
    float x = 0.25 * gl_FragCoord.x;
    Target0 = textureLod(t_Entries, vec2(x, v_Y), 0.0);
}
#endif //FS
