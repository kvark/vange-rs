//!include vs:globals.inc vs:surface.inc fs:surface.inc fs:color.inc

layout(location = 0) varying vec4 v_Pos;

#ifdef SHADER_VS

layout(location = 0) attribute ivec4 a_Pos;

void main() {
    v_Pos = vec4(a_Pos.xy * u_TextureScale.xy, u_TextureScale.z - float(gl_InstanceIndex + 1), 1.0);
    gl_Position = u_ViewProj * v_Pos;
}
#endif //VS


#ifdef SHADER_FS
//imported: Surface, u_TextureScale, get_surface, evaluate_color

layout(location = 0) out vec4 o_Color;

void main() {
    Surface surface = get_surface(v_Pos.xy);
    uint type = 0U;
    if (v_Pos.z <= surface.low_alt) {
        type = surface.low_type;
    } else if (v_Pos.z >= surface.low_alt + surface.delta && v_Pos.z <= surface.high_alt) {
        type = surface.high_type;
    } else {
        discard;
    };

    float lit_factor = v_Pos.z <= surface.low_alt && surface.delta != 0.0 ? 0.25 : 1.0;
    o_Color = evaluate_color(type, surface.tex_coord, v_Pos.z / u_TextureScale.z, lit_factor);
}
#endif //FS
