//!include vs:shape.inc

#ifdef SHADER_VS
//imported: Polygon, get_shape_polygon

layout(set = 0, binding = 0) uniform c_Globals {
    vec4 u_CameraPos;
    mat4 u_ViewProj;
    mat4 u_InvViewProj;
    vec4 u_LightPos;
    vec4 u_LightColor;
};

layout(set = 3, binding = 0) uniform c_Locals {
    mat4 u_Model;
    vec4 u_ShapeScale;
};

void main() {
    Polygon poly = get_shape_polygon();
    vec4 pos = vec4(u_ShapeScale.xxx, 1.0) * (u_Model * poly.vertex);
    gl_Position = u_ViewProj * pos;
}
#endif //VS


#ifdef SHADER_FS

layout(set = 1, binding = 0) uniform c_Debug {
    vec4 u_Color;
};

layout(location = 0) out vec4 o_Color;

void main() {
    o_Color = u_Color;
}
#endif //FS
