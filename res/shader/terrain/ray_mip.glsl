//!include vs:globals.inc fs:globals.inc fs:terrain/locals.inc fs:surface.inc fs:shadow.inc fs:color.inc
//!specialization COLOR

#ifdef SHADER_VS

layout(location = 0) attribute ivec4 a_Pos;

void main() {
    // orhto projections don't like infinite values
    gl_Position = u_ViewProj[2][3] == 0.0 ?
        // the expected geometry is 4 trianges meeting in the center
        vec4(a_Pos.xy, 0.0, 0.5) :
        u_ViewProj * vec4(a_Pos);
}
#endif //VS


#ifdef SHADER_FS
//imported: Surface, u_TextureScale, get_lod_height, get_surface, evaluate_color

const float c_DepthBias = COLOR != 0 ? 0.0 : 0.01;
const float c_Step = 0.6;

#if COLOR
#define TERRAIN_WATER   0U
layout(location = 0) out vec4 o_Color;
#endif

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
        num_jumps--;

        if (units.z < min_side_unit) {
            lod--;
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

    #if COLOR
    // debug output here
    if (u_Params.w != 0U) {
        o_Color = vec4(
            (num_jumps == 0U ? 0.5 : 0.0) + (num_steps == 0U ? 0.5 : 0.0),
            1.0 - float(num_jumps) / float(u_Params.y),
            1.0 - float(num_steps) / float(u_Params.z),
            1.0);
    }
    #endif //COLOR
    return point;
}

void main() {
    vec3 sp_near_world = get_frag_world(0.0);
    vec3 sp_far_world = get_frag_world(1.0);
    vec3 view = normalize(sp_far_world - sp_near_world);
    vec3 point = cast_ray(sp_near_world, view);
    //vec3 point = cast_ray_to_plane(0.0, sp_near_world, view);

    #if COLOR
    if (u_Params.w == 0U) {
        float lit_factor = fetch_shadow(point);
        Surface surface = get_surface(point.xy);
        uint type = point.z <= surface.low_alt ? surface.low_type : surface.high_type;
        o_Color = evaluate_color(type, surface.tex_coord, point.z / u_TextureScale.z, lit_factor);
    }
    #endif //COLOR

    vec4 target_ndc = u_ViewProj * vec4(point, 1.0);
    gl_FragDepth = target_ndc.z / target_ndc.w + c_DepthBias;
}
#endif //FS
