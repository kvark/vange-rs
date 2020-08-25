//!include vs:globals.inc vs:terrain/locals.inc vs:surface.inc fs:surface.inc fs:color.inc

layout(location = 0) varying vec3 v_TexCoord;
layout(location = 1) flat varying uint v_Type;

#ifdef SHADER_VS

vec2 generate_paint_pos() {
    int row_size = int(ceil(u_SampleRange.y - u_SampleRange.x));
    float rel_x = float(gl_InstanceIndex % row_size);
    float rel_y = float(gl_InstanceIndex / row_size);
    float x = u_CamOriginDir.z > 0.0 ? u_SampleRange.x + rel_x : u_SampleRange.y - rel_x;
    float y = u_CamOriginDir.w > 0.0 ? u_SampleRange.z + rel_y : u_SampleRange.w - rel_y;
    return vec2(x, y);
}

void main() {
    vec2 pos_center = generate_paint_pos();

    Surface suf = get_surface(pos_center);
    float altitude = gl_VertexIndex >= 12 ? suf.high_alt :
        gl_VertexIndex >= 8 ? suf.low_alt + suf.delta :
        gl_VertexIndex >= 4 ? suf.low_alt : 0.0;
        
    int cx = ((gl_VertexIndex + 0) & 0x3) >= 2 ? 1 : 0;
    int cy = ((gl_VertexIndex + 1) & 0x3) >= 2 ? 1 : 0;
    vec2 pos = floor(pos_center) + vec2(cx, cy);

    v_Type = gl_VertexIndex < 8 ? suf.low_type : suf.high_type;
    v_TexCoord = vec3(suf.tex_coord, altitude / u_TextureScale.z);
    gl_Position = u_ViewProj * vec4(pos, altitude, 1.0);
}
#endif //VS


#ifdef SHADER_FS
//imported: Surface, u_TextureScale, get_surface, evaluate_color

layout(location = 0) out vec4 o_Color;

void main() {
    //TODO: move most of the evaluation to the vertex shader
    o_Color = evaluate_color(v_Type, v_TexCoord.xy, v_TexCoord.z, 1.0);
}
#endif //FS
