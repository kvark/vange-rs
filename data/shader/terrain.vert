#version 150 core

uniform c_Locals {
	vec4 u_CamPos;
	mat4 u_ViewProj;
	mat4 u_InvViewProj;
};

in ivec4 a_Pos;

void main() {
    gl_Position = u_ViewProj * vec4(a_Pos);
}
