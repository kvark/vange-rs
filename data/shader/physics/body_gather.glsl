//!include cs:body.inc cs:physics/collision.inc cs:encode.inc cs:quat.inc

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
    float scale = s_Bodies[index].pos_scale.w * s_Bodies[index].physics.scale.y;
    vec4 orientation = s_Bodies[index].orientation;
    vec3 springs = vec3(0.0);

    for (uint i=range.x; i<range.y; ++i) {
        CollisionPolygon cp = s_Collisions[i];
        float depth = resolve_depth(cp.depth_soft);
        if (depth != 0.0) {
            vec3 origin = decode_pos(cp.middle);
            vec3 rg0 = qrot(orientation, origin) * scale;
            float df = min(depth * u_Constants.contact_elastic.y, u_Constants.impulse_elastic.x);
            springs += df * vec3(rg0.y, -rg0.x, 1.0);
        }
    }

    s_Bodies[index].springs.xyz += springs;
}
#endif //CS
