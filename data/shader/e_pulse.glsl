flat varying vec4 v_Linear;
flat varying vec4 v_Angular;


#ifdef SHADER_VS

attribute vec4 a_Linear;
attribute vec4 a_Angular;
attribute vec4 a_Entry;

void main() {
    float x = (gl_VertexID == 0 ? 0.0 : 1.0);
    gl_Position = vec4(x, a_Entry.x, 0.0, 1.0);
    v_Linear = a_Linear;
    v_Angular = a_Angular;
}
#endif //VS


#ifdef SHADER_FS

out vec4 Target0;

void main() {
    Target0 = (gl_FragCoord.x < 3.0 ? v_Linear : v_Angular);
}
#endif //FS
