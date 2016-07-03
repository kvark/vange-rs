#version 150 core

uniform c_Locals {
	mat4 u_ModelViewProj;
	mat4 u_NormalMatrix;
};

uniform usampler1D t_ColorTable;
uniform sampler1D t_Palette;

in vec4 a_Pos;
in vec4 a_Normal;
in uint a_ColorIndex;

out vec4 v_Color;

void main() {
    gl_Position = u_ModelViewProj * a_Pos;
    uvec2 color_params = texelFetch(t_ColorTable, int(a_ColorIndex), 0).xy;
    v_Color = texelFetch(t_Palette, int(color_params[0]), 0);
}