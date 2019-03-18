//!include fs:surface.inc fs:color.inc

layout(set = 0, binding = 0) uniform Globals {
    vec4 u_CameraPos;
    mat4 u_ViewProj;
    mat4 u_InvViewProj;
    vec4 u_LightPos;
    vec4 u_LightColor;
};

#ifdef SHADER_VS

layout(location = 0) attribute ivec4 a_Pos;

void main() {
    gl_Position = u_ViewProj * a_Pos;
}
#endif //VS


#ifdef SHADER_FS
//imported: Surface, u_TextureScale, get_lod_height, get_surface, evaluate_color

layout(set = 1, binding = 1) uniform Locals {
    vec4 u_ScreenSize;      // XY = size
    uvec4 u_Params; // X = max mipmap level, Y = max iterations
};

#define TERRAIN_WATER   0U

layout(location = 0) out vec4 o_Color;

vec3 cast_ray_to_plane(float level, vec3 base, vec3 dir) {
    float t = (level - base.z) / dir.z;
    return t * dir + base;
}

// Algorithm is based on "http://www.tevs.eu/project_i3d08.html"
//"Maximum Mipmaps for Fast, Accurate, and Scalable Dynamic Height Field Rendering"
vec3 cast_ray(vec3 point, vec3 dir) {
    uint lod = u_Params.x;
    ivec2 ipos = ivec2(floor(point.xy)); // integer coordinate of the cell
    uint num_jumps = u_Params.y, num_steps = u_Params.z;
    while (num_jumps != 0U && num_steps != 0U) {
        // step 0: at lowest LOD, just advance
        if (lod == 0U) {
            Surface surface = get_surface(point.xy);
            if (point.z < surface.low_alt || (point.z < surface.high_alt && point.z >= surface.low_alt + surface.delta)) {
                break;
            }
            if (surface.low_alt == surface.high_alt) {
                lod++; //try to escape the low level and LOD
            }
            point += c_Step * dir;
            ipos = ivec2(floor(point.xy));
            num_steps--;
            continue;
        }

        // step 1: get the LOD height and early out
        float height = get_lod_height(ipos, int(lod));
        if (point.z < height) {
            lod--;
            continue;
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
            // adjust the integer position on cell boundary
            // figure out if we hit the higher LOD bound and switch to it
            float affinity;
            vec2 proximity = mod(cell_id, 2.0) - 0.5;
            if (units.x <= units.y) {
                ipos.x = dir.x < 0.0 ? cell_tl.x - 1 : cell_tl.x + (1 << lod);
                affinity = dir.x * proximity.x;
            }
            if (units.y <= units.x) {
                ipos.y = dir.y < 0.0 ? cell_tl.y - 1 : cell_tl.y + (1 << lod);
                affinity = dir.y * proximity.y;
            }
            if (lod < u_Params.x && affinity > 0.0) {
                lod++;
            }
        }
    }

    // debug output here
    if (u_Params.w != 0U) {
        Target0 = vec4(
            (num_jumps == 0U ? 0.5 : 0.0) + (num_steps == 0U ? 0.5 : 0.0),
            1.0 - float(num_jumps) / float(u_Params.y),
            1.0 - float(num_steps) / float(u_Params.z),
            1.0);
    }
    return point;
}


void main() {
    vec4 sp_ndc = vec4(
        (gl_FragCoord.xy / u_ScreenSize.xy) * 2.0 - 1.0,
        -1.0,
        1.0
    );
    vec4 sp_world = u_InvViewProj * sp_ndc;
    vec4 sp_zero = u_InvViewProj * vec4(0.0, 0.0, -1.0, 1.0);
    vec3 near_plane = sp_world.xyz / sp_world.w;
    vec3 view_base =
        u_ViewProj[2][3] == 0.0 ? sp_zero.xyz/sp_zero.w : near_plane;
    vec3 view = normalize(view_base - u_CameraPos.xyz);

    vec3 point = cast_ray(view_base, view);
    //vec3 point = cast_ray_to_plane(0.0, near_plane, view);

    if (u_Params.w == 0U) {
        Surface surface = get_surface(point.xy);
        uint type = point.z <= surface.low_alt ? surface.low_type : surface.high_type;
        float lit_factor = point.z <= surface.low_alt && surface.delta != 0.0 ? 0.25 : 1.0;
        Target0 = evaluate_color(type, surface.tex_coord, point.z / u_TextureScale.z, lit_factor);
    }
    o_Color = frag_color;

    vec4 target_ndc = u_ViewProj * vec4(point, 1.0);
    gl_FragDepth = target_ndc.z / target_ndc.w * 0.5 + 0.5;
}
#endif //FS
