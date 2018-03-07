
#ifdef SHADER_VS

struct Polygon {
    vec4 u_Origin;
    vec4 u_Normal;
};

uniform c_Locals {
    mat4 u_ModelViewProj;
};

uniform c_Polys {
    Polygon polygons[0x100];
};

attribute vec4 a_Pos;

void main() {
    gl_Position = u_ModelViewProj * a_Pos;
}
#endif //VS


#ifdef SHADER_FS

out vec4 Target0;

void main() {
    Target0 = vec4(1.0);
}
#endif //FS
