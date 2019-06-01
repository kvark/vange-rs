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
layout(set = 2, binding = 0, std430) buffer Storage
{
    uint w_Data[];
};
layout(set = 2, binding = 1) uniform c_Scatter {
    vec2 u_CamOrigin;
    vec2 u_CamDir;
    vec2 u_RangeY;
    vec2 u_RangeX;
};


const uint WORK_GROUP_SIZE = 16;

vec2 generate_pos() {
    float y_sqrt = mix(
        sqrt(abs(u_RangeY.x)) * sign(u_RangeY.x),
        sqrt(abs(u_RangeY.y)) * sign(u_RangeY.y),
        float(gl_GlobalInvocationID.y) / float(gl_NumWorkGroups.y * WORK_GROUP_SIZE - 1)
    );
    float y = y_sqrt * y_sqrt * sign(y_sqrt);
    float x_limit = mix(
        u_RangeX.x, u_RangeX.y,
        (y - u_RangeY.x) / (u_RangeY.y - u_RangeY.x)
    );
    float x = mix(
        -x_limit, x_limit,
        float(gl_GlobalInvocationID.x) / float(gl_NumWorkGroups.x * WORK_GROUP_SIZE - 1)
    );
    return u_CamOrigin + u_CamDir * y + vec2(u_CamDir.y, -u_CamDir.x) * x;
}

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
    uvec2 tc = min(u_ScreenSize.xy - 1U, uvec2(round((ndc.xy * 0.5 + 0.5) * vec2(u_ScreenSize.xy))));
    atomicMin(w_Data[tc.y * u_ScreenSize.x + tc.x], value);
}

void main() {
    vec2 pos = generate_pos();
    Surface suf = get_surface(pos);
    if (true) {
        add_voxel(pos, suf.high_alt, suf.high_type, 1.0);
    }
    if (suf.delta != 0.0) {
        add_voxel(pos, suf.low_alt, suf.low_type, 0.25);
        float mid = mix(suf.low_alt + suf.delta, suf.high_alt, 0.5);
        add_voxel(pos, mid, suf.high_type, 1.0);
    } else {
        add_voxel(pos, 0.4 * suf.high_alt, suf.high_type, 1.0);
        add_voxel(pos, 0.8 * suf.high_alt, suf.high_type, 1.0);
    }
}
#endif //CS
