#version 150 core

uniform c_Globals {
	vec4 u_CameraPos;
	mat4 u_ViewProj;
	mat4 u_InvViewProj;
	vec4 u_LightPos;
	vec4 u_LightColor;
};

in ivec4 a_Pos;

void main() {
    gl_Position = u_ViewProj * vec4(a_Pos);
}
