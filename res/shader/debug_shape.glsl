//!include vs:body.inc vs:globals.inc vs:encode.inc vs:quat.inc vs:shape.inc

#ifdef SHADER_VS
//imported: Polygon, get_shape_polygon

layout(set = 0, binding = 2, std430) readonly buffer Storage {
    Body s_Bodies[];
};

layout(location = 3) attribute vec4 a_PosScale;
layout(location = 4) attribute vec4 a_Orientation;
layout(location = 5) attribute float a_ShapeScale;
layout(location = 6) attribute uvec2 a_BodyAndColorId;

void main() {
    Polygon poly = get_shape_polygon();
    vec3 vertex = vec3(poly.vertex) * a_ShapeScale;
    vec3 local = qrot(a_Orientation, vertex) * a_PosScale.w + a_PosScale.xyz;

    int body_id = int(a_BodyAndColorId.x);
    vec4 base_pos_scale = s_Bodies[body_id].pos_scale;
    vec4 base_orientation = s_Bodies[body_id].orientation;
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
