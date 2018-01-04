#version 150 core

const float c_Emissive = 0.3, c_Ambient = 0.5, c_Diffuse = 3.0;

uniform c_Globals {
	vec4 u_CameraPos;
	mat4 u_ViewProj;
	mat4 u_InvViewProj;
	vec4 u_LightPos;
	vec4 u_LightColor;
};


in vec4 v_Color;
in vec3 v_Normal;
in vec3 v_Light;
out vec4 Target0;


void main() {
	vec3 normal = normalize(v_Normal) * (gl_FrontFacing ? -1.0 : 1.0);
	vec3 light_dir = normalize(v_Light);
	float n_dot_l = max(0.0, dot(normal, light_dir));
	float kd = c_Ambient + c_Diffuse * n_dot_l;

	Target0 = v_Color * (c_Emissive + kd * u_LightColor);
}
