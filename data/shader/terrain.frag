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
const float c_ReflectionVariance = 0.5, c_ReflectionPower = 0.2;
const uint c_DoubleLevelMask = 1U<<6, c_ShadowMask = 1U<<7;
const uint c_TerrainShift = 3U, c_TerrainBits = 3U;
const uint c_DeltaShift = 0U, c_DeltaBits = 2U;

#define TERRAIN_WATER	0U

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
		//TODO: we need either low or high for the most part
		// so this can be more efficient with a boolean param
		uint delta;
		if (mod(pos.x, 2.0) >= 1.0) {
			uint meta_low = textureOffset(t_Meta, tc, ivec2(-1, 0)).x;
			suf.high_type = suf.low_type;
			suf.low_type = get_terrain_type(meta_low);
			delta = get_delta(meta_low) << c_DeltaBits + get_delta(meta);
		}else {
			uint meta_high = textureOffset(t_Meta, tc, ivec2(1, 0)).x;
			suf.tex_coord.x += 1.0 / u_TextureScale.x;
			suf.high_type = get_terrain_type(meta_high);
			delta = get_delta(meta) << c_DeltaBits + get_delta(meta_high);
		}
		suf.low_alt = textureOffset(t_Height, suf.tex_coord, ivec2(-1, 0)).x * u_TextureScale.z;
		suf.high_alt = texture(t_Height, suf.tex_coord).x * u_TextureScale.z;
		suf.delta = float(delta) / 192.0 * u_TextureScale.z;
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

Surface cast_ray_impl(
	inout vec3 a, inout vec3 b,
	bool high, int num_forward, int num_binary
) {
	vec3 step = (1.0 / float(num_forward + 1)) * (b - a);
	Surface result;

	for (int i=0; i<num_forward; ++i) {
		vec3 c = a + step;
		Surface suf = get_surface(c.xy);
		float height = mix(suf.low_alt, suf.high_alt, high);
		if (c.z < height) {
			result = suf;
			b = c;
			break;
		} else {
			a = c;
		}
	}

	for (int i=0; i<num_binary; ++i) {
		vec3 c = mix(a, b, 0.5);
		Surface suf = get_surface(c.xy);
		float height = mix(suf.low_alt, suf.high_alt, high);
		if (c.z < height) {
			result = suf;
			b = c;
		} else {
			a = c;
		}
	}

	return result;
}

struct CastPoint {
	vec3 pos;
	uint type;
	vec3 tex_coord;
	bool is_underground;
	bool is_shadowed;
};

CastPoint cast_ray_to_map(vec3 base, vec3 dir) {
	vec3 a = cast_ray_to_plane(u_TextureScale.z, base, dir);
	vec3 c = cast_ray_to_plane(0.0, base, dir);
	vec3 b = c;
	Surface suf = cast_ray_impl(a, b, true, 10, 5);
	CastPoint result;
	result.type = suf.high_type;
	result.is_underground = false;

	if (suf.low_alt <= b.z && b.z < suf.low_alt + suf.delta) {
		// continue the cast underground
		a = b; b = c;
		suf = cast_ray_impl(a, b, false, 6, 4);
		result.type = suf.low_type;
		result.is_underground = true;
	}

	//float t = a.z > a.w + 0.1 ? (b.w - a.w - b.z + a.z) / (a.z - a.w) : 0.5;
	result.pos = b;
	result.tex_coord = suf.tex_coord;
	result.is_shadowed = suf.is_shadowed;
	return result;
}

vec4 evaluate_color(CastPoint pt) {
	float terrain = float(pt.type) + 0.5;
	float diff = textureOffset(t_Height, pt.tex_coord, ivec2(1, 0)).x -
				 textureOffset(t_Height, pt.tex_coord, ivec2(-1, 0)).x;
	float light_clr = texture(t_Table, vec3(0.5 * diff + 0.5, 0.25, terrain)).x;
	float tmp = light_clr - c_HorFactor * (1.0 - pt.pos.z / u_TextureScale.z);
	float shadow_koeff = pt.is_shadowed ? 0.25 : 0.5;
	float color_id = texture(t_Table, vec3(shadow_koeff * tmp + 0.5, 0.75, terrain)).x;
	return texture(t_Palette, color_id);
}

void main() {
	vec4 sp_ndc = vec4((gl_FragCoord.xy / u_ScreenSize.xy) * 2.0 - 1.0, -1.0, 1.0);
	vec4 sp_world = u_InvViewProj * sp_ndc;
	vec4 sp_zero = u_InvViewProj * vec4(0.0, 0.0, -1.0, 1.0);
	vec3 near_plane = sp_world.xyz / sp_world.w;
	vec3 view_base = u_ViewProj[2][3] == 0.0 ? sp_zero.xyz/sp_zero.w : near_plane;
	vec3 view = normalize(view_base - u_CamPos.xyz);

	CastPoint pt = cast_ray_to_map(near_plane, view);
	vec4 frag_color = evaluate_color(pt);
	if (pt.type == TERRAIN_WATER) {
		vec3 a = pt.pos;
		vec2 variance = mod(a.xy, c_ReflectionVariance);
		vec3 reflected = normalize(view * vec3(1.0 + variance, -1.0));
		vec3 outside = cast_ray_to_plane(u_TextureScale.z, a, reflected);
		vec3 b = outside;
		Surface suf = cast_ray_impl(a, b, true, 4, 4);
		if (b != outside) {
			CastPoint other;
			other.pos = b;
			other.type = suf.high_type;
			other.tex_coord = suf.tex_coord;
			other.is_shadowed = suf.is_shadowed;
			frag_color += c_ReflectionPower * evaluate_color(other);
		}
	}
	Target0 = frag_color;

	vec4 target_ndc = u_ViewProj * vec4(pt.pos, 1.0);
	gl_FragDepth = target_ndc.z / target_ndc.w * 0.5 + 0.5;
}
