#version 150 core

uniform c_Locals {
	mat4 u_ModelViewProj;
};

in vec4 a_Pos;
in vec4 a_Color;

out vec4 v_Color;

void main() {
    gl_Position = u_ModelViewProj * a_Pos;
    v_Color = a_Color;
}
