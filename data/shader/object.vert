#version 150 core

uniform c_Globals {
	vec4 u_CameraPos;
	mat4 u_ViewProj;
	mat4 u_InvViewProj;
	vec4 u_LightPos;
	vec4 u_LightColor;
};

uniform c_Locals {
	mat4 u_Model;
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
	vec4 world = u_Model * a_Pos;
	gl_Position = u_ViewProj * world;

	uvec2 color_params = texelFetch(t_ColorTable, int(a_ColorIndex), 0).xy;
	v_Color = texelFetch(t_Palette, int(color_params[0]), 0);

	vec3 n = normalize(a_Normal.xyz);
	v_Normal = mat3(u_Model) * n;
	v_Light = u_LightPos.xyz - world.xyz * u_LightPos.w;  
}
