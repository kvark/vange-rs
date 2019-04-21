#ifdef SHADER_CS

layout(set = 1, binding = 1) uniform c_Locals {
    uvec4 u_ScreenSize;      // XY = size
};

layout(set = 2, binding = 0, r32ui) uniform uimage2D i_Output;

void main() {
    uvec2 pos = gl_GlobalInvocationID.xy;
    if (pos.x < u_ScreenSize.x && pos.y < u_ScreenSize.y) {
        imageStore(i_Output, ivec2(pos), uvec4(0xFFFFFF00U));
    }
}
#endif //CS
