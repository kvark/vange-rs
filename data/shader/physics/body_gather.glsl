//!include cs:body.inc cs:physics/collision.inc cs:physics/pulse.inc cs:encode.inc cs:quat.inc

layout(set = 0, binding = 0, std430) buffer Storage {
    Body s_Bodies[];
};

layout(set = 0, binding = 2, std140) uniform Constants {
    GlobalConstants u_Constants;
};

layout(set = 1, binding = 0, std430) readonly buffer Collision {
    CollisionPolygon s_Collisions[];
};

layout(set = 1, binding = 1, std430) readonly buffer Ranges {
    uint s_Ranges[];
};

#ifdef SHADER_CS

void main() {
    uint index =
        gl_GlobalInvocationID.z * gl_WorkGroupSize.x * gl_NumWorkGroups.x * gl_WorkGroupSize.y * gl_NumWorkGroups.y +
        gl_GlobalInvocationID.y * gl_WorkGroupSize.x * gl_NumWorkGroups.x +
        gl_GlobalInvocationID.x;

    uvec2 range = (uvec2(s_Ranges[index]) >> uvec2(0, 16)) & 0xFFFF;
    Body body = s_Bodies[index];
    float scale = body.pos_scale.w * body.physics.scale.y;
    vec3 springs = vec3(0.0);

    vec4 irot = qinv(body.orientation);
    vec3 z_axis = qrot(irot, vec3(0.0, 0.0, 1.0));
    mat3 j_inv = calc_j_inv(body.model, body.pos_scale.w);
    vec3 vel = body.v_linear.xyz;
    vec3 wel = body.v_angular.xyz;
    bool stand_on_wheels = true; //TEMP!
    float modulation = 1.0;
    float k_friction = u_Constants.impulse.w;

    for (uint i=range.x; i<range.y; ++i) {
        CollisionPolygon cp = s_Collisions[i];
        float depth = resolve_depth(cp.depth_soft);
        if (depth != 0.0) {
            vec3 r0 = decode_pos(cp.middle) * scale;
            vec3 rg0 = qrot(body.orientation, r0);
            //vec3 r1 = qrot(irot, vec3(r0.xy, rg0.z));
            vec3 r1 = r0;
            vec3 u0 = vel + cross(wel, r1);

            if (dot(u0, z_axis) < 0.0) {
                if (stand_on_wheels) { // ignore XY
                    u0.xy = vec2(0.0);
                } else {
                    //vec3 normal = vec3(cp.normal, sqrt(1.0 - dot(cp.normal, cp.normal)));
                    //float kn = dot(u0, normal) * (1.0 - k_friction);
                    //u0 = u0 * k_friction + normal * kn;
                }
                mat3 cmi = calc_collision_matrix_inv(r0, j_inv);
                vec3 pulse = (cmi * u0) * (-u_Constants.impulse_factors.y * modulation);
                vel += pulse;
                wel += j_inv * cross(r0, pulse);
            }

            float df = min(depth * u_Constants.contact_elastic.y, u_Constants.impulse_elastic.x);
            springs += df * vec3(rg0.y, -rg0.x, 1.0);
        }
    }

    s_Bodies[index].v_linear.xyz = vel;
    s_Bodies[index].v_angular.xyz = wel;
    s_Bodies[index].springs.xyz += springs;
}
#endif //CS
