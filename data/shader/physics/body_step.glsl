//!include cs:body.inc cs:quat.inc

#ifdef SHADER_CS

layout(set = 0, binding = 0, std430) buffer Storage {
    Body s_Bodies[];
};

layout(set = 0, binding = 1, std140) uniform Uniforms {
    vec4 u_Delta;
};

layout(set = 0, binding = 2, std140) uniform Constants {
    vec4 u_Nature; // X = time delta0, Z = gravity
    vec2 u_DragFree;
    vec2 u_DragSpeed;
    vec2 u_DragSpring;
    vec2 u_DragAbsMin;
    vec2 u_DragAbsStop;
    vec2 u_DragColl;
};

void main() {
    uint index =
        gl_GlobalInvocationID.z * gl_WorkGroupSize.x * gl_NumWorkGroups.x * gl_WorkGroupSize.y * gl_NumWorkGroups.y +
        gl_GlobalInvocationID.y * gl_WorkGroupSize.x * gl_NumWorkGroups.x +
        gl_GlobalInvocationID.x;
    Body body = s_Bodies[index];

    float speed_correction_factor = u_Delta.x / u_Nature.x;
    vec3 vel = body.linear.xyz;
    vec3 wel = body.angular.xyz;

    vec2 drag = u_DragFree.xy * pow(u_DragSpeed, vec2(length(vel), dot(wel, wel)));

    vec4 irot = qinv(body.orientation);
    vec3 z_axis = qrot(irot, vec3(0.0, 0.0, 1.0));
    bool spring_touch = dot(body.springs, body.springs) != 0.0;
    bool wheels_touch = z_axis.z > 0.0 && spring_touch;

    if (spring_touch) {
        drag *= u_DragSpring;
    }

    vec3 v_accel = qrot(irot, vec3(0.0, 0.0, body.springs.z - u_Nature.z));
    vec3 w_accel = qrot(irot, vec3(body.springs.xy, 0.0));

    vec3 tmp = vec3(0.0, 0.0, body.scale_volume_zomc.z * body.pos_scale.w);
    w_accel -= u_Nature.z * cross(tmp, z_axis);

    vel += u_Delta.x * v_accel;
    wel += u_Delta.x * (mat3(body.jacobian_inv) * w_accel);

    vec2 drag_corrected = pow(drag, vec2(speed_correction_factor));
    vel *= drag_corrected.x;
    wel *= drag_corrected.y;

    s_Bodies[index].pos_scale.xyz = body.pos_scale.xyz + u_Delta.x * vel;
    s_Bodies[index].orientation = normalize(body.orientation + vec4(u_Delta.x * wel, 0.0));
    s_Bodies[index].linear.xyz = vel;
    s_Bodies[index].angular.xyz = wel;
    s_Bodies[index].springs.xyz = vec3(0.0);
}
#endif //CS
