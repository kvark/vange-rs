#version 150 core

uniform c_Locals {
	mat4 u_ModelViewProj;
	vec4 u_Color;
};

in ivec4 a_Pos;

out vec4 v_Color;

void main() {
    gl_Position = u_ModelViewProj * a_Pos;
    v_Color = u_Color;
}
