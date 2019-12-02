//!include vs:globals.inc vs:encode.inc vs:quat.inc vs:shape.inc

#ifdef SHADER_VS
//imported: Polygon, get_shape_polygon

layout(set = 3, binding = 0) uniform c_Locals {
    vec4 u_PosScale;
    vec4 u_Orientation;
    float u_ShapeScale;
    uint u_BodyId;
};

void main() {
    Polygon poly = get_shape_polygon();
    vec3 pos = qrot(u_Orientation, vec3(poly.vertex)) * u_PosScale.w * u_ShapeScale + u_PosScale.xyz;
    gl_Position = u_ViewProj * vec4(pos, 1.0);
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
