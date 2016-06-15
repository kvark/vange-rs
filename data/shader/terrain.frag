#version 150 core

uniform c_Locals {
	vec4 u_CamPos;
	mat4 u_ViewProj;
	mat4 u_InvViewProj;
};

uniform sampler2DArray t_Height;
uniform usampler2DArray t_Meta;
uniform sampler1D t_Palette;

const uint c_DoubleLevelMask = 1U<<6;
const vec4 c_ScreenSize = vec4(800.0, 540.0, 0.0, 0.0);
const vec4 c_TextureScale = vec4(2048.0, 4096.0, 50.0, 4.0);
const uint c_NumBinarySteps = 8U, c_NumForwardSteps = 0U;

out vec4 Target0;

vec3 cast_ray_to_plane(float level, vec3 base, vec3 dir) {
	float t = (level - base.z) / dir.z;
	return t * dir + base;
}

float get_latitude(vec2 pos) {
	vec2 ndc = pos / c_TextureScale.xy;
	float slice = trunc(mod(ndc.y, c_TextureScale.w));
	if (mod(pos.x, 2.0) >= 1.0) {
		vec2 ndc_prev = ndc - vec2(1.0/c_TextureScale.x, 0.0);
		uint meta_prev = texture(t_Meta, vec3(ndc_prev, slice)).x;
		if ((meta_prev & c_DoubleLevelMask) != 0U) {
			ndc = ndc_prev;
		}
	}
	return texture(t_Height, vec3(ndc, slice)).x * c_TextureScale.z;
}

vec4 cast_ray_with_latitude(float level, vec3 base, vec3 dir) {
	vec3 pos = cast_ray_to_plane(level, base, dir);
	float height = get_latitude(pos.xy);
	return vec4(pos, height);
}

vec3 cast_ray_to_map(vec3 base, vec3 dir) {
	vec4 a = cast_ray_with_latitude(c_TextureScale.z, base, dir);
	vec4 b = cast_ray_with_latitude(0.0, base, dir);
	vec4 step = (1.0 / float(c_NumForwardSteps + 1U)) * (b - a);
	for (uint i=0U; i<c_NumForwardSteps; ++i) {
		vec4 c = a + step;
		c.w = get_latitude(c.xy);
		if (c.z < c.w) {
			b = c;
			break;
		}else {
			a = c;
		}
	}
	for (uint i=0U; i<c_NumBinarySteps; ++i) {
		vec4 c = 0.5 * (a + b);
		c.w = get_latitude(c.xy);
		if (c.z < c.w) {
			b = c;
		}else {
			a = c;
		}
	}
	//float t = a.z > a.w + 0.1 ? (b.w - a.w - b.z + a.z) / (a.z - a.w) : 0.5;
	float t = 0.5;
	return mix(a.xyz, b.xyz, t);
}

void main() {
	vec4 sp_ndc = vec4((gl_FragCoord.xy / c_ScreenSize.xy) * 2.0 - 1.0, 0.0, 1.0);
	vec4 sp_world = u_InvViewProj * sp_ndc;
	vec3 view = normalize(sp_world.xyz / sp_world.w - u_CamPos.xyz);

	//vec3 pos = cast_ray_with_latitude(0.0, u_CamPos.xyz, view).xyw;
	vec3 pos = cast_ray_to_map(u_CamPos.xyz, view);
	Target0 = texture(t_Palette, pos.z / c_TextureScale.z);
}
