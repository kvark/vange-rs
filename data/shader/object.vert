#version 150 core

uniform c_Locals {
	mat4 u_ModelViewProj;
	mat4 u_NormalMatrix;
};

in vec4 a_Pos;
in vec4 a_Normal;
out vec3 v_Normal;

void main() {
    gl_Position = u_ModelViewProj * a_Pos;
    v_Normal = (u_NormalMatrix * a_Normal).xyz;
}
