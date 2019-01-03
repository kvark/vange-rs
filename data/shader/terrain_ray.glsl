//!include fs:surface.inc fs:color.inc

uniform c_Globals {
    vec4 u_CameraPos;
    mat4 u_ViewProj;
    mat4 u_InvViewProj;
    vec4 u_LightPos;
    vec4 u_LightColor;
};


#ifdef SHADER_VS

attribute vec4 a_Pos;

void main() {
    gl_Position = u_ViewProj * a_Pos;
}
#endif //VS


#ifdef SHADER_FS
//imported: Surface, u_TextureScale, get_lod_height, get_surface, evaluate_color

uniform c_Locals {
    vec4 u_ScreenSize;      // XY = size
    uvec4 u_Params; // X = max mipmap level, Y = max iterations
};

#define TERRAIN_WATER   0U

out vec4 Target0;


vec3 cast_ray_to_plane(float level, vec3 base, vec3 dir) {
    float t = (level - base.z) / dir.z;
    return t * dir + base;
}

vec3 cast_ray(vec3 point, vec3 dir) {
    uint lod = u_Params.x;
    ivec2 ipos = ivec2(floor(point.xy)); // integer coordinate of the cell
    uint iter = 0U;
    while (iter < u_Params.y) {
        // step 1: get the LOD height and early out
        float height = get_lod_height(ipos, int(lod));
        if (point.z < height) {
            if (lod == 0U) {
                break;
            }
            lod--;
            continue;
        }
        // assumption: point.z >= height

        // step 2: figure out the closest intersection with the cell
        // it can be X axis, Y axis, or the depth
        vec2 cell_id = floor(vec2(ipos) / float(1 << lod)); // careful!
        ivec2 cell_tl = ivec2(cell_id) << lod;
        vec2 cell_offset = float(1 << lod) * step(0.0, dir.xy) - point.xy + vec2(cell_tl);
        vec3 units = vec3(cell_offset, height - point.z) / dir;
        float min_side_unit = min(units.x, units.y);

        // advance the point
        point += min(units.z, min_side_unit) * dir;
        ipos = ivec2(floor(point.xy));
        iter++;

        if (units.z < min_side_unit) {
            if (lod == 0U) {
                break;
            }
            lod--;
        } else {
            // adjust the integer position on cell boundary
            // figure out if we hit the higher LOD bound and switch to it
            float affinity;
            vec2 proximity = mod(cell_id, 2.0) - 0.5;
            if (units.x <= units.y) {
                ipos.x = cell_tl.x + (dir.x < 0.0 ? -1 : 1 << lod);
                affinity = dir.x * proximity.x;
            }
            if (units.y <= units.x) {
                ipos.y = cell_tl.y + (dir.y < 0.0 ? -1 : 1 << lod);
                affinity = dir.y * proximity.y;
            }
            if (lod < u_Params.x && affinity > 0.0) {
                lod++;
            }
        }
    }

    //Target0 = vec4(iter == u_Params.y ? 1.0 : 0.0, float(iter) / float(u_Params.y), 0.0, 1.0);
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
    Surface surface = get_surface(point.xy);
    Target0 = evaluate_color(surface.high_type, surface.tex_coord, point.z / u_TextureScale.z, 1.0);

    vec4 target_ndc = u_ViewProj * vec4(point, 1.0);
    gl_FragDepth = target_ndc.z / target_ndc.w * 0.5 + 0.5;
}
#endif //FS
