#ifdef SHADER_CS

layout(set = 1, binding = 1) uniform c_Locals {
    uvec4 u_ScreenSize;      // XY = size
};
layout(set = 2, binding = 0, std430) buffer Storage {
    uint w_Data[];
};

void main() {
    uvec2 pos = gl_GlobalInvocationID.xy;
    if (pos.x < u_ScreenSize.x && pos.y < u_ScreenSize.y) {
        w_Data[pos.y * u_ScreenSize.x + pos.x] = 0xFFFFFF00U;
    }
}
#endif //CS
