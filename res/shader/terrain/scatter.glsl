//!include cs:globals.inc cs:terrain/locals.inc cs:surface.inc cs:color.inc

#ifdef SHADER_CS
//imported: Surface, get_surface, evaluate_color_id

layout(set = 2, binding = 0, std430) buffer Storage {
    uint w_Data[];
};

bool is_visible(vec4 p) {
    return p.w > 0.0 && p.z >= 0.0 &&
        p.x >= -p.w && p.x <= p.w &&
        p.y >= -p.w && p.y <= p.w;
}

void add_voxel(vec2 pos, float altitude, uint type, float lit_factor) {
    vec4 screen_pos = u_ViewProj * vec4(pos, altitude, 1.0);
    if (!is_visible(screen_pos)) {
        return;
    }
    vec3 ndc = screen_pos.xyz / screen_pos.w;
    ndc.y *= -1.0;
    float color_id = evaluate_color_id(type, pos / u_TextureScale.xy, altitude / u_TextureScale.z, lit_factor);
    float depth = clamp(ndc.z, 0.0, 1.0);
    uint value = (uint(depth * float(0xFFFFFF)) << 8U) | uint(color_id * float(0xFF));
    uvec2 tc = clamp(uvec2(round((ndc.xy * 0.5 + 0.5) * vec2(u_ScreenSize.xy))), uvec2(0U), u_ScreenSize.xy - 1U);
    atomicMin(w_Data[tc.y * u_ScreenSize.x + tc.x], value);
}

void main() {
    vec2 source_coord = vec2(gl_GlobalInvocationID.xy) /
        vec2(gl_NumWorkGroups.xy * gl_WorkGroupSize.xy - vec2(1));
    vec2 pos = generate_scatter_pos(source_coord);

    Surface suf = get_surface(pos);
    float base = 0.0;
    float t = float(gl_GlobalInvocationID.z) / float(gl_NumWorkGroups.z * gl_WorkGroupSize.z);

    if (suf.delta != 0.0) {
        float alt = mix(suf.low_alt, 0.0, t);
        add_voxel(pos, alt, suf.low_type, 0.25);
        base = suf.low_alt + suf.delta;
    }
    if (true) {
        float alt = mix(suf.high_alt, base, t);
        add_voxel(pos, alt, suf.high_type, 1.0);
    }
}
#endif //CS
