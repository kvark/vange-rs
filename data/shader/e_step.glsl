flat varying vec4 v_EntryDelta;


#ifdef SHADER_VS

attribute vec4 a_EntryDelta;

void main() {
    float x = (gl_VertexID == 0 ? 0.5 : 1.0);
    gl_Position = vec4(x, a_EntryDelta.x, 0.0, 1.0);
    v_EntryDelta = a_EntryDelta;
}
#endif //VS


#ifdef SHADER_FS

uniform sampler2D t_Velocities;

out vec4 Target0;

void main() {
    float x = 0.5 * gl_FragCoord.x;
    float y = v_EntryDelta.x * 0.5 + 0.5;
    vec4 data = textureLod(t_Velocities, vec2(x, y), 0.0);
    Target0 = v_EntryDelta.y * data;
}
#endif //FS
