#version 150 core

uniform c_Locals {
	vec4 u_CamPos;
	vec4 u_ScreenSize;		// XY = size
	vec4 u_TextureScale;	// XY = size, Z = height scale, w = number of layers
	mat4 u_ViewProj;
	mat4 u_InvViewProj;
};

uniform sampler2DArray t_Height;
uniform usampler2DArray t_Meta;
uniform sampler1D t_Palette;
uniform sampler2DArray t_Table;

const float c_HorFactor = 0.5; //H_CORRECTION
const uint c_DoubleLevelMask = 1U<<6, c_ShadowMask = 1U<<7;
const uint c_TerrainShift = 3U, c_TerrainBits = 3U;
const uint c_DeltaShift = 0U, c_DeltaBits = 2U;
const uint c_NumBinarySteps = 8U, c_NumForwardSteps = 0U;

out vec4 Target0;


struct Surface {
	float low_alt, high_alt, delta;
	uint low_type, high_type;
	vec3 tex_coord;
	bool is_shadowed;
};

uint get_terrain_type(uint meta) {
	return (meta >> c_TerrainShift) & ((1U << c_TerrainBits) - 1U);
}
uint get_delta(uint meta) {
	return (meta >> c_DeltaShift) & ((1U << c_DeltaBits) - 1U);
}

Surface get_surface(vec2 pos) {
	vec3 tc = vec3(pos / u_TextureScale.xy, 0.0);
	tc.z = trunc(mod(tc.y, u_TextureScale.w));
	Surface suf;
	suf.tex_coord = tc;
	uint meta = texture(t_Meta, tc).x;
	suf.is_shadowed = (meta & c_ShadowMask) != 0U;
	suf.low_type = get_terrain_type(meta);
	if ((meta & c_DoubleLevelMask) != 0U) {
		if (mod(pos.x, 2.0) >= 1.0) {
			uint meta_low = textureOffset(t_Meta, tc, ivec2(-1, 0)).x;
			suf.high_type = suf.low_type;
			suf.low_type = get_terrain_type(meta_low);
			suf.delta = get_delta(meta_low) << c_DeltaBits + get_delta(meta);
		}else {
			uint meta_high = textureOffset(t_Meta, tc, ivec2(1, 0)).x;
			suf.tex_coord.x += 1.0 / u_TextureScale.x;
			suf.high_type = get_terrain_type(meta_high);
			suf.delta = get_delta(meta) << c_DeltaBits + get_delta(meta_high);
		}
		suf.low_alt = textureOffset(t_Height, suf.tex_coord, ivec2(-1, 0)).x * u_TextureScale.z;
		suf.high_alt = texture(t_Height, suf.tex_coord).x * u_TextureScale.z;
	}else {
		suf.low_alt = texture(t_Height, tc).x * u_TextureScale.z;
		suf.high_alt = suf.low_alt;
		suf.high_type = suf.low_type;
		suf.delta = 0.0;
	}
	return suf;
}


vec3 cast_ray_to_plane(float level, vec3 base, vec3 dir) {
	float t = (level - base.z) / dir.z;
	return t * dir + base;
}

vec3 cast_ray_to_map(vec3 base, vec3 dir) {
	vec3 a = cast_ray_to_plane(u_TextureScale.z, base, dir);
	vec3 b = cast_ray_to_plane(0.0, base, dir);
	vec3 step = (1.0 / float(c_NumForwardSteps + 1U)) * (b - a);
	bool above_surface = true;
	for (uint i=0U; i<c_NumForwardSteps; ++i) {
		vec3 c = a + step;
		Surface suf = get_surface(c.xy);
		if (c.z < suf.low_alt) {
			b = c;
			break;
		} else {
			a = c;
		}
	}
	for (uint i=0U; i<c_NumBinarySteps; ++i) {
		vec3 c = mix(a, b, 0.5);
		Surface suf = get_surface(c.xy);
		if (c.z < suf.low_alt) {
			b = c;
		} else {
			a = c;
		}
	}
	//float t = a.z > a.w + 0.1 ? (b.w - a.w - b.z + a.z) / (a.z - a.w) : 0.5;
	float t = 0.5;
	return mix(a, b, t);
}

void main() {
	vec4 sp_ndc = vec4((gl_FragCoord.xy / u_ScreenSize.xy) * 2.0 - 1.0, 0.0, 1.0);
	vec4 sp_world = u_InvViewProj * sp_ndc;
	vec3 view = normalize(sp_world.xyz / sp_world.w - u_CamPos.xyz);

	//vec3 pos = cast_ray_to_plane(0.0, u_CamPos.xyz, view);
	//pos.z = get_surface(pos.xy).low_alt;
	//Target0 = texture(t_Palette, pos.z / u_TextureScale.z);
	vec3 pos = cast_ray_to_map(u_CamPos.xyz, view);
	{
		Surface suf = get_surface(pos.xy);
		float terrain = float(suf.low_type) + 0.5;
		float diff = textureOffset(t_Height, suf.tex_coord, ivec2(1, 0)).x -
					 textureOffset(t_Height, suf.tex_coord, ivec2(-1, 0)).x;
		float light_clr = texture(t_Table, vec3(0.5 * diff + 0.5, 0.25, terrain)).x;
		float tmp = light_clr - c_HorFactor * (1.0 - pos.z / u_TextureScale.z);
		float shadow_koeff = suf.is_shadowed ? 0.25 : 0.5;
		float color_id = texture(t_Table, vec3(shadow_koeff * tmp + 0.5, 0.75, terrain)).x;
		Target0 = texture(t_Palette, color_id);
	}
	vec4 target_ndc = u_ViewProj * vec4(pos, 1.0);
	gl_FragDepth = target_ndc.z / target_ndc.w * 0.5 + 0.5;
}
