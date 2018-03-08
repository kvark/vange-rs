//!include surface.frag

varying vec3 v_World;
varying vec3 v_PolyOrigin;
varying vec3 v_PolyNormal;

#ifdef SHADER_VS

struct Polygon {
    vec4 u_Origin;
    vec4 u_Normal;
};

uniform c_Locals {
    mat4 u_Model;
    vec4 u_TargetCenterScale;
};

uniform c_Polys {
    Polygon polygons[0x100];
};

attribute vec4 a_Pos;

void main() {
    v_World = (u_Model * a_Pos).xyz;

    Polygon poly = polygons[gl_VertexID >> 2];
    v_PolyOrigin = poly.u_Origin.xyz;
    v_PolyNormal = normalize(poly.u_Normal.xyz);

    vec2 offset = v_World.xy - u_Model[3].xy;
    vec2 out_pos = (offset + u_TargetCenterScale.xy) * u_TargetCenterScale.zw - vec2(1.0);
    gl_Position = vec4(out_pos, 0.0, 1.0);
}
#endif //VS


#ifdef SHADER_FS
//imported: Surface, get_surface

out vec4 Target0;

void main() {
    Surface suf = get_surface(v_World.xy);
    Target0 = vec4(suf.high_alt / u_TextureScale.z);
}
#endif //FS
