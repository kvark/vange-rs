//!include quat.vert

flat varying vec4 v_Linear;
flat varying vec4 v_Angular;


#ifdef SHADER_VS
//imported: qrot,qmul,qinv

attribute vec4 a_EntryDeltaDid;

uniform c_Globals {
    vec4 u_GlobalForce;
};

uniform sampler2D t_Entries;
uniform sampler2D t_Collisions;

void main() {
    // car constants
    float car_z_offset_of_mass_center = 0.0;
    mat3 j_inv = mat3(vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0));

    float in_y = a_EntryDeltaDid.x * 0.5 + 0.5;
    vec4 pos = textureLod(t_Entries, vec2(0.125, in_y), 0.0);
    vec4 rot = textureLod(t_Entries, vec2(0.375, in_y), 0.0);
    vec3 vel = textureLod(t_Entries, vec2(0.625, in_y), 0.0).xyz;
    vec3 wel = textureLod(t_Entries, vec2(0.875, in_y), 0.0).xyz;

    bool spring_touch = true; //TODO
    vec3 collision = spring_touch ?
        textureLod(t_Collisions, vec2(a_EntryDeltaDid.z, 0.5), 0.0).xyz :
        vec3(0.0);

    float out_x = (gl_VertexID == 0 ? 0.5 : 1.0);
    gl_Position = vec4(out_x, a_EntryDeltaDid.x, 0.0, 1.0);

    float delta = a_EntryDeltaDid.y;
    vec4 irot = qinv(rot);
    vec3 z_axis = qrot(irot, vec3(0.0, 0.0, 1.0));
    vec3 vac = qrot(irot, u_GlobalForce.xyz + vec3(0.0, 0.0, collision.z));
    vec3 wac = qrot(irot, vec3(collision.xy, 0.0));

    vec3 tmp = vec3(0.0, 0.0, car_z_offset_of_mass_center * pos.w);
    wac += u_GlobalForce.z * cross(tmp, z_axis);

    vel += delta * vac;
    wel += delta * (j_inv * wac);

    v_Linear = vec4(vel, 0.0);
    v_Angular = vec4(wel, 0.0);
}
#endif //VS


#ifdef SHADER_FS

out vec4 Target0;

void main() {
    Target0 = (gl_FragCoord.x < 1.0 ? v_Linear : v_Angular);
}
#endif //FS
