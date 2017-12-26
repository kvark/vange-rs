#version 150 core

const vec3 c_LightDir = normalize(vec3(1.0, 0.0, -2.0));
const vec4 c_LightColor = vec4(1.0, 1.0, 1.0, 1.0);
const float c_Emissive = 0.1, c_Ambient = 0.1, c_Diffuse = 1.0, c_Specular = 0.2, c_SpecularPower = 10.0;

in vec3 v_Normal, v_Camera;
in vec4 v_Color;
out vec4 Target0;


void main() {
	vec3 normal = normalize(v_Normal) * (gl_FrontFacing ? 1.0 : -1.0);
	float ddot = max(0.0, dot(normal, c_LightDir));
	float kd = c_Ambient + c_Diffuse * ddot;
	vec3 reflected = reflect(c_LightDir, normal);
	float sdot = max(dot(reflected, v_Camera), 0.01);
	float ks = c_Specular * pow(sdot, c_SpecularPower);
	Target0 = v_Color * c_Emissive + c_LightColor * (kd * v_Color + ks);
}
