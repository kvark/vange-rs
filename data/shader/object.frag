#version 150 core

const float c_Emissive = 0.1, c_Ambient = 0.1, c_Diffuse = 1.0;

uniform c_Locals {
	mat4 u_ModelViewProj;
	mat4 u_NormalMatrix;
	vec4 u_CameraWorldPos;
	vec4 u_LightWorldPos;
	vec4 u_LightColor;
};

in vec4 v_Color;
in vec3 v_Normal;
in vec3 v_Light;
out vec4 Target0;


void main() {
	vec3 normal = normalize(v_Normal) * (gl_FrontFacing ? 1.0 : -1.0);
	vec3 light_dir = normalize(v_Light);
	float n_dot_l = max(0.0, dot(normal, light_dir));
	float kd = c_Ambient + c_Diffuse * n_dot_l;
	Target0 = v_Color * (c_Emissive + kd * u_LightColor);
}
