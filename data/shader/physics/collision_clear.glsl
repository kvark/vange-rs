#ifdef SHADER_CS

struct CollisionPolygon {
    uint middle;
    uint depth;
};

layout(set = 0, binding = 1, std430) buffer Storage {
    CollisionPolygon s_Collisions[];
};

void main() {
    uint index =
        gl_GlobalInvocationID.z * gl_WorkGroupSize.x * gl_NumWorkGroups.x * gl_WorkGroupSize.y * gl_NumWorkGroups.y +
        gl_GlobalInvocationID.y * gl_WorkGroupSize.x * gl_NumWorkGroups.x +
        gl_GlobalInvocationID.x;
    s_Collisions[int(index)].middle = 0;
    s_Collisions[int(index)].depth = 0;
}
#endif //CS
