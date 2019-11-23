#ifdef SHADER_CS

layout(set = 0, binding = 1, std430) buffer Storage {
    int s_Data[];
};

void main() {
    uint index =
        gl_GlobalInvocationID.z * gl_WorkGroupSize.x * gl_NumWorkGroups.x * gl_WorkGroupSize.y * gl_NumWorkGroups.y +
        gl_GlobalInvocationID.y * gl_WorkGroupSize.x * gl_NumWorkGroups.x +
        gl_GlobalInvocationID.x;
    //Note: this is supposed to clear data for exactly one polygon
    s_Data[int(index)] = 0;
}
#endif //CS
