#version 150 core

uniform c_Locals {
	mat4 u_ModelViewProj;
	mat4 u_NormalMatrix;
	vec4 u_CameraWorldPos;
	vec4 u_LightWorldPos;
	vec4 u_LightColor;
};

uniform usampler1D t_ColorTable;
uniform sampler1D t_Palette;

in ivec4 a_Pos;
in vec4 a_Normal;
in uint a_ColorIndex;

out vec4 v_Color;
out vec3 v_Normal;
out vec3 v_Light;

void main() {
	vec4 world = u_NormalMatrix * a_Pos;
	gl_Position = u_ModelViewProj * a_Pos;
	uvec2 color_params = texelFetch(t_ColorTable, int(a_ColorIndex), 0).xy;
	v_Color = texelFetch(t_Palette, int(color_params[0]), 0);
	vec3 n = normalize(a_Normal.xyz);
	v_Normal = mat3(u_NormalMatrix) * n;
	v_Light = u_LightWorldPos.xyz - world.xyz * u_LightWorldPos.w;  
}
