flat varying vec4 v_Linear;
flat varying vec4 v_Angular;


#ifdef SHADER_VS

attribute vec4 a_EntryDeltaDid;

uniform c_Globals {
    vec4 u_GlobalForce;
};

uniform sampler2D t_Entries;
uniform sampler2D t_Collisions;

void main() {
    float in_y = a_EntryDeltaDid.x * 0.5 + 0.5;
    vec4 pos = textureLod(t_Entries, vec2(0.125, in_y), 0.0);
    vec4 rot = textureLod(t_Entries, vec2(0.375, in_y), 0.0);
    vec4 vel = textureLod(t_Entries, vec2(0.625, in_y), 0.0);
    vec4 wel = textureLod(t_Entries, vec2(0.875, in_y), 0.0);
    vec3 collision = textureLod(t_Collisions, vec2(a_EntryDeltaDid.z, 0.5), 0.0).xyz;

    float out_x = (gl_VertexID == 0 ? 0.5 : 1.0);
    gl_Position = vec4(out_x, a_EntryDeltaDid.x, 0.0, 1.0);

    float delta = a_EntryDeltaDid.y;
}
#endif //VS


#ifdef SHADER_FS

out vec4 Target0;

void main() {
    Target0 = (gl_FragCoord.x < 1.0 ? v_Linear : v_Angular);
}
#endif //FS
