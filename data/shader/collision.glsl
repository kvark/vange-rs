//!include surface.frag

flat varying vec3 v_Rg0;
varying vec3 v_World;
varying vec3 v_PolyNormal;


#ifdef SHADER_VS

uniform c_Locals {
    mat4 u_Model;
    vec4 u_TargetCenterScale;
};

struct Polygon {
    vec4 u_Origin;
    vec4 u_Normal;
};

uniform c_Polys {
    Polygon polygons[0x100];
};

attribute vec4 a_Pos;

void main() {
    v_World = (u_Model * a_Pos).xyz;

    Polygon poly = polygons[gl_VertexID >> 2];
    v_Rg0 = mat3(u_Model) * poly.u_Origin.xyz;
    v_PolyNormal = normalize(poly.u_Normal.xyz);

    vec2 offset = v_World.xy - u_Model[3].xy;
    vec2 out_pos = (offset + u_TargetCenterScale.xy) * u_TargetCenterScale.zw - vec2(1.0);
    gl_Position = vec4(out_pos, 0.0, 1.0);
}
#endif //VS


#ifdef SHADER_FS
//imported: Surface, get_surface

uniform c_Globals {
    vec4 u_Penetration; // X=scale, Y=limit
};

out vec4 Target0;

void main() {
    Surface suf = get_surface(v_World.xy);
    float depth_raw = max(0.0, suf.high_alt - v_World.z);
    float depth = min(u_Penetration.y, u_Penetration.x * depth_raw);

    Target0 = depth * vec4(v_Rg0.y, -v_Rg0.x, 1.0, 0.0);
}
#endif //FS
