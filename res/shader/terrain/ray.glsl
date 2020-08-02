//!include vs:globals.inc fs:globals.inc fs:terrain/locals.inc fs:surface.inc fs:color.inc
//!specialization COLOR

#ifdef SHADER_VS

layout(location = 0) attribute ivec4 a_Pos;

void main() {
    gl_Position = u_ViewProj * vec4(a_Pos);
}
#endif //VS


#ifdef SHADER_FS
//imported: Surface, u_TextureScale, get_surface, evaluate_color

#if COLOR
const float
    c_ReflectionVariance = 0.5,
    c_ReflectionPower = 0.2;

#define TERRAIN_WATER   0U

layout(location = 0) out vec4 o_Color;
#endif //COLOR

vec3 cast_ray_to_plane(float level, vec3 base, vec3 dir) {
    float t = (level - base.z) / dir.z;
    return t * dir + base;
}

Surface cast_ray_impl(
    inout vec3 a, inout vec3 b,
    bool high, int num_forward, int num_binary
) {
    vec3 step = (1.0 / float(num_forward + 1)) * (b - a);

    for (int i = 0; i < num_forward; ++i) {
        vec3 c = a + step;
        Surface suf = get_surface(c.xy);

        if (c.z > suf.high_alt) {
            high = true; // re-appear on the surface
            a = c;
        } else {
            float height = mix(suf.low_alt, suf.high_alt, high);
            if (c.z <= height) {
                b = c;
                break;
            } else {
                a = c;
            }
        }
    }

    Surface result = get_surface(b.xy);

    for (int i = 0; i < num_binary; ++i) {
        vec3 c = mix(a, b, 0.5);
        Surface suf = get_surface(c.xy);

        float height = mix(suf.low_alt, suf.high_alt, high);
        if (c.z <= height) {
            b = c;
            result = suf;
        } else {
            a = c;
        }
    }

    return result;
}

struct CastPoint {
    vec3 pos;
    uint type;
    vec2 tex_coord;
    bool is_underground;
    //bool is_shadowed;
};

CastPoint cast_ray_to_map(vec3 base, vec3 dir) {
    CastPoint result;

    vec3 a = base.z <= u_TextureScale.z ? base :
        cast_ray_to_plane(u_TextureScale.z, base, dir);
    vec3 c = cast_ray_to_plane(0.0, base, dir);
    vec3 b = c;

    Surface suf = cast_ray_impl(a, b, true, 8, 4);
    result.type = suf.high_type;
    result.is_underground = false;

    if (suf.delta != 0.0 && b.z < suf.low_alt + suf.delta) {
        // continue the cast underground, but reserve
        // the right to re-appear above the surface.
        a = b; b = c;
        suf = cast_ray_impl(a, b, false, 6, 3);
        if (b.z >= suf.low_alt + suf.delta) {
            result.type = suf.high_type;
        } else {
            result.type = suf.low_type;
            // underground is better indicated by a real shadow
            //result.is_underground = true;
        }
    }

    result.pos = b;
    result.tex_coord = suf.tex_coord;
    //result.is_shadowed = suf.is_shadowed;

    return result;
}

#if COLOR
vec4 color_point(CastPoint pt, float lit_factor) {
    return evaluate_color(pt.type, pt.tex_coord, pt.pos.z / u_TextureScale.z, lit_factor);
}
#endif

void main() {
    vec4 sp_ndc = get_frag_ndc();
    vec4 sp_world = u_InvViewProj * sp_ndc;
    vec4 sp_zero = u_InvViewProj * vec4(0.0, 0.0, -1.0, 1.0);
    vec3 near_plane = sp_world.xyz / sp_world.w;
    vec3 view_base =
        u_ViewProj[2][3] == 0.0 ? sp_zero.xyz/sp_zero.w : near_plane;
    vec3 view = normalize(view_base - u_CameraPos.xyz);

    CastPoint pt = cast_ray_to_map(near_plane, view);

    #if COLOR
    float lit_factor;
    if (pt.is_underground) {
        lit_factor = 0.25;
    } else {
        vec3 light_vec = normalize(u_LightPos.xyz - pt.pos * u_LightPos.w);
        vec3 a = pt.pos;
        vec3 outside = cast_ray_to_plane(u_TextureScale.z, a, light_vec);
        vec3 b = outside;

        Surface suf = cast_ray_impl(a, b, true, 4, 4);
        if (suf.delta != 0.0 && b.z < suf.low_alt + suf.delta) {
            // continue casting overground
            a = b; b = outside;
            cast_ray_impl(a, b, true, 3, 3);
        }
        lit_factor = b == outside ? 1.0 : 0.5;
    }

    vec4 frag_color = color_point(pt, lit_factor);

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
            //other.is_shadowed = suf.is_shadowed;
            vec4 ref_color = color_point(other, 0.8);
            frag_color += c_ReflectionPower * ref_color;
        }
    }
    o_Color = frag_color;
    #endif //COLOR

    vec4 target_ndc = u_ViewProj * vec4(pt.pos, 1.0);
    gl_FragDepth = target_ndc.z / target_ndc.w;
}
#endif //FS
