#version 150 core

uniform c_Locals {
	vec4 u_CamPos;
	mat4 u_ViewProj;
	mat4 u_InvViewProj;
};

uniform sampler2DArray t_Height;
uniform usampler2DArray t_Meta;

const vec4 c_ScreenSize = vec4(800.0, 540.0, 0.0, 0.0);
const vec4 c_Scale = vec4(1.0/100.0, 1.0/200.0, 0.1, 4.0);

out vec4 Target0;

vec2 cast_ray(float level, vec3 base, vec3 dir) {
	float t = (level - base.z) / dir.z;
	return base.xy + t * dir.xy;
}

float get_latitude(vec2 pos) {
	return texture(t_Height, vec3(pos*c_Scale.xy, 0.0)).x;
}

void main() {
	vec4 sp_ndc = vec4((gl_FragCoord.xy / c_ScreenSize.xy) * 2.0 - 1.0, 0.0, 1.0);
	vec4 sp_world = u_InvViewProj * sp_ndc;
	vec3 view = normalize(sp_world.xyz / sp_world.w - u_CamPos.xyz);

	vec2 gpos = cast_ray(0.0, u_CamPos.xyz, view);
	float lat = get_latitude(gpos);
	Target0 = vec4(lat, lat, lat, 1.0);
}
