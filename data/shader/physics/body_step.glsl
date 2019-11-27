//!include cs:body.inc cs:quat.inc

#ifdef SHADER_CS

void main() {
    uint index =
        gl_GlobalInvocationID.z * gl_WorkGroupSize.x * gl_NumWorkGroups.x * gl_WorkGroupSize.y * gl_NumWorkGroups.y +
        gl_GlobalInvocationID.y * gl_WorkGroupSize.x * gl_NumWorkGroups.x +
        gl_GlobalInvocationID.x;

    float delta = 0.1;
    vec3 collision = vec3(0.0);

    vec4 pos_scale = sData[index].pos_scale;
    vec4 rot = sData[index].rot;

    vec4 irot = qinv(rot);
    vec3 z_axis = qrot(irot, vec3(0.0, 0.0, 1.0));
    vec3 vac = qrot(irot, u_GlobalForce.xyz + vec3(0.0, 0.0, collision.z));
    vec3 wac = qrot(irot, vec3(collision.xy, 0.0));

    Car car = s_Cars[sData[index].car_id.x];
    vec3 tmp = vec3(0.0, 0.0, car.zomc.x * pos_scale.w);
    wac += u_GlobalForce.z * cross(tmp, z_axis);

    vel += delta * vac;
    wel += delta * (car.jacobian_inv * wac);

    sData[index].pos_scale.xyz = pos_scale.xyz + delta * vel;
    sData[index].rot = normalize(rot + vec4(delta * wel, 0.0));
    sData[index].v_linear = vec4(vel, 0.0);
    sData[index].v_angular = vec4(wel, 0.0);
}
#endif //CS
