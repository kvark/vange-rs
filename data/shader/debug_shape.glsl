//!include vs:shape.inc

layout(set = 0, binding = 0) uniform c_Globals {
    vec4 u_CameraPos;
    mat4 u_ViewProj;
    mat4 u_InvViewProj;
    vec4 u_LightPos;
    vec4 u_LightColor;
};

#ifdef SHADER_VS
//imported: Polygon, get_shape_polygon

void main() {
	Polygon poly = get_shape_polygon();
    gl_Position = u_ViewProj * poly.vertex;
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
