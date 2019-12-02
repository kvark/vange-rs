//!include vs:body.inc vs:globals.inc vs:encode.inc vs:quat.inc vs:shape.inc

#ifdef SHADER_VS
//imported: Polygon, get_shape_polygon

layout(set = 0, binding = 2, std430) readonly buffer Storage {
    Body s_Bodies[];
};

layout(set = 3, binding = 0) uniform c_Locals {
    vec4 u_PosScale;
    vec4 u_Orientation;
    float u_ShapeScale;
    uint u_BodyId;
};

void main() {
    Polygon poly = get_shape_polygon();
    vec3 vertex = vec3(poly.vertex) * u_ShapeScale;
    vec3 local = qrot(u_Orientation, vertex) * u_PosScale.w + u_PosScale.xyz;

    vec4 base_pos_scale = s_Bodies[int(u_BodyId)].pos_scale;
    vec4 base_orientation = s_Bodies[int(u_BodyId)].orientation;
    vec3 world = qrot(base_orientation, local) * base_pos_scale.w + base_pos_scale.xyz;

    gl_Position = u_ViewProj * vec4(world, 1.0);
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
