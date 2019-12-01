//!include cs:body.inc cs:quat.inc

#ifdef SHADER_CS

layout(set = 0, binding = 0, std430) buffer Storage {
    Data s_Data[];
};

layout(set = 0, binding = 1, std140) uniform Uniforms {
    vec4 u_GlobalForce;
    vec4 u_Delta;
};

void main() {
    uint index =
        gl_GlobalInvocationID.z * gl_WorkGroupSize.x * gl_NumWorkGroups.x * gl_WorkGroupSize.y * gl_NumWorkGroups.y +
        gl_GlobalInvocationID.y * gl_WorkGroupSize.x * gl_NumWorkGroups.x +
        gl_GlobalInvocationID.x;
    Data data = s_Data[index];

    vec4 irot = qinv(data.orientation);
    vec3 z_axis = qrot(irot, vec3(0.0, 0.0, 1.0));
    vec3 vac = qrot(irot, u_GlobalForce.xyz + vec3(0.0, 0.0, data.springs.z));
    vec3 wac = qrot(irot, vec3(data.springs.xy, 0.0));

    vec3 tmp = vec3(0.0, 0.0, data.scale_volume_zomc.z * data.pos_scale.w);
    wac += u_GlobalForce.z * cross(tmp, z_axis);

    vec3 vel = data.linear.xyz + u_Delta.x * vac;
    vec3 wel = data.angular.xyz + u_Delta.x * (mat3(data.jacobian_inv) * wac);

    s_Data[index].pos_scale.xyz = data.pos_scale.xyz + u_Delta.x * vel;
    s_Data[index].orientation = normalize(data.orientation + vec4(u_Delta.x * wel, 0.0));
    s_Data[index].linear.xyz = vel;
    s_Data[index].angular.xyz = wel;
    s_Data[index].springs.xyz = vec3(0.0);
}
#endif //CS
