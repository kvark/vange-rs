#version 150 core

uniform c_Locals {
	mat4 u_ModelViewProj;
	mat4 u_NormalMatrix;
	vec4 u_CameraWorldPos;
};

uniform usampler1D t_ColorTable;
uniform sampler1D t_Palette;

in ivec4 a_Pos;
in vec4 a_Normal;
in uint a_ColorIndex;

out vec4 v_Color;
out vec3 v_Normal, v_HalfNormal;

void main() {
    gl_Position = u_ModelViewProj * a_Pos;
    uvec2 color_params = texelFetch(t_ColorTable, int(a_ColorIndex), 0).xy;
    v_Color = texelFetch(t_Palette, int(color_params[0]), 0);
    vec3 n = normalize(a_Normal.xyz);
    v_Normal = (u_NormalMatrix * vec4(n, 0.0)).xyz;
    vec3 camDir = u_CameraWorldPos.xyz - (u_NormalMatrix * a_Pos).xyz;
    v_HalfNormal = normalize(v_Normal + normalize(camDir));
}
