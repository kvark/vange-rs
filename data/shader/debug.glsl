varying vec4 v_Color;


#ifdef SHADER_VS

uniform c_Locals {
    mat4 u_ModelViewProj;
};

attribute vec4 a_Pos;
attribute vec4 a_Color;

void main() {
    gl_Position = u_ModelViewProj * a_Pos;
    v_Color = a_Color;
}
#endif //VS


#ifdef SHADER_FS

out vec4 Target0;

void main() {
    Target0 = v_Color;
}
#endif //FS
