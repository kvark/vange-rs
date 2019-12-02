//!include cs:body.inc cs:encode.inc

struct CollisionPolygon {
    uint middle;
    uint depth;
};

layout(set = 0, binding = 0, std430) buffer Storage {
    Data s_Data[];
};

layout(set = 1, binding = 0, std430) readonly buffer Collision {
    CollisionPolygon s_Collisions[];
};

layout(set = 1, binding = 1, std430) readonly buffer Ranges {
    uint s_Ranges[];
};

#ifdef SHADER_CS

const uint DEPTH_BITS = 20;

void main() {
    uint index =
        gl_GlobalInvocationID.z * gl_WorkGroupSize.x * gl_NumWorkGroups.x * gl_WorkGroupSize.y * gl_NumWorkGroups.y +
        gl_GlobalInvocationID.y * gl_WorkGroupSize.x * gl_NumWorkGroups.x +
        gl_GlobalInvocationID.x;
    uvec2 range = (uvec2(s_Ranges[index]) >> uvec2(0, 16)) & 0xFFFF;

    vec3 springs = vec3(0.0);
    for (uint i=range.x; i<range.y; ++i) {
        CollisionPolygon cp = s_Collisions[i];
        vec3 origin = decode_pos(cp.middle);
        uint depth_count = cp.depth >> DEPTH_BITS;
        if (depth_count != 0U) {
            float depth = (cp.depth & ((1U << DEPTH_BITS) - 1U)) / float(depth_count);
            springs += vec3(origin.y * depth, -origin.x * depth, depth);
        }
    }

    s_Data[index].springs.xyz += springs;
}
#endif //CS