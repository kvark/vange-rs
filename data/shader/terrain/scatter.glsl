//!include cs:surface.inc cs:color.inc

#ifdef SHADER_CS
//imported: Surface, get_surface, evaluate_color_id

layout(set = 0, binding = 0) uniform Globals {
    vec4 u_CameraPos;
    mat4 u_ViewProj;
    mat4 u_InvViewProj;
    vec4 u_LightPos;
    vec4 u_LightColor;
};

layout(set = 1, binding = 1) uniform c_Locals {
    uvec4 u_ScreenSize;      // XY = size
};

layout(set = 2, binding = 0, r32ui) uniform uimage2D i_Output;
layout(set = 2, binding = 1) uniform c_Scatter {
    vec4 u_TerrainOffsetScale;
};


bool is_visible(vec4 p) {
    return p.w > 0.0 && p.z > 0.0 && p.x >= -p.w && p.x <= p.w && p.y >= -p.w && p.y < p.w;
}

void add_voxel(vec2 pos, float altitude, uint type, float lit_factor) {
    vec4 screen_pos = u_ViewProj * vec4(pos, altitude, 1.0);
    if (!is_visible(screen_pos)) {
        return;
    }
    vec3 ndc = screen_pos.xyz / screen_pos.w;
    float color_id = evaluate_color_id(type, pos / u_TextureScale.xy, altitude / u_TextureScale.z, lit_factor);
    float depth = clamp(ndc.z, 0.0, 1.0);
    uint value = (uint(depth * float(0xFFFFFF)) << 8U) | uint(color_id * float(0xFF));
    ivec2 tc = min(ivec2(u_ScreenSize.xy) - 1, ivec2(round((ndc.xy * 0.5 + 0.5) * vec2(u_ScreenSize.xy))));
    imageAtomicMin(i_Output, tc, value);
}

void main() {
    vec2 pos = vec2(gl_GlobalInvocationID.xy) * u_TerrainOffsetScale.xy + u_TerrainOffsetScale.zw;
    Surface suf = get_surface(pos);
    if (true) {
        add_voxel(pos, suf.high_alt, suf.high_type, 1.0);
    }
    if (suf.delta != 0.0) {
        add_voxel(pos, suf.low_alt, suf.low_type, 0.25);
    }
}
#endif //CS
