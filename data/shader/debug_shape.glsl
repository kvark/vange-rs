//!include shape.vert

varying vec4 v_Color;

uniform c_Locals {
    mat4 u_ModelViewProj;
    vec4 u_Color;
};


#ifdef SHADER_VS
//imported: Polygon, get_shape_polygon

void main() {
	Polygon poly = get_shape_polygon();
    gl_Position = u_ModelViewProj * poly.vertex;
}
#endif //VS


#ifdef SHADER_FS

out vec4 Target0;

void main() {
    Target0 = u_Color;
}
#endif //FS
