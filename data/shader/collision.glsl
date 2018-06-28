//!include quat.vert transform.vert shape.vert surface.frag

flat varying vec3 v_Vector;
varying vec3 v_World;
flat varying vec3 v_PolyNormal;


#ifdef SHADER_VS
//imported: Polygon, get_shape_polygon,
//imported: Transform, fetch_entry_transform, transform

// Compute the exact collision vector instead of using the origin
// of the input polygon.
const float EXACT_VECTOR = 0.0;

uniform c_Locals {
    vec4 u_BoundScaleEntry;
    vec4 u_TargetCenterScale;
};

void main() {
    Polygon poly = get_shape_polygon();
    Transform world = fetch_entry_transform(int(u_BoundScaleEntry.y));
    world.scale = u_BoundScaleEntry.x;
    v_World = transform(world, poly.vertex);

    v_Vector = mix(transform(world, vec4(poly.origin, 0.0)), v_World - world.pos, EXACT_VECTOR);
    v_PolyNormal = poly.normal;

    vec2 local_pos = poly.vertex.xy * u_BoundScaleEntry.xx;
    vec2 out_pos = (local_pos + u_TargetCenterScale.xy) * u_TargetCenterScale.zw - vec2(1.0);
    gl_Position = vec4(out_pos, 0.0, 1.0);
}
#endif //VS


#ifdef SHADER_FS
//imported: Surface, get_surface

// Each pixel of the collision grid corresponds to a level texel
// and contributes to the total momentum. The universal scale between
// individual impulses here and the rough overage computed by the
// original game is encoded in this constant.
const float SCALE = 0.01;

uniform c_Globals {
    vec4 u_Penetration; // X=scale, Y=limit
};

out vec4 Target0;

void main() {
    Surface suf = get_surface(v_World.xy);

    // see `GET_MIDDLE_HIGHT` macro in the original
    float extra_room = suf.high_alt - suf.low_alt > 130.0 ? 110.0 : 48.0;
    float middle = suf.low_alt + extra_room;
    float depth_raw = max(0.0, suf.low_alt - v_World.z);

    if (v_World.z > middle && middle < suf.high_alt) {
        depth_raw = max(0.0, suf.high_alt - v_World.z);
        if (v_World.z - middle < depth_raw) {
            depth_raw = 0.0;
        }
    }

    float depth = SCALE * min(u_Penetration.y, u_Penetration.x * depth_raw);
    Target0 = depth * vec4(v_Vector.y, -v_Vector.x, 1.0, 0.0);
}
#endif //FS
