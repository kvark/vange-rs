//!include vs:body.inc vs:encode.inc vs:quat.inc vs:shape.inc fs:physics/collision.inc fs:surface.inc

//layout(location = 0) flat varying vec3 v_Vector;
layout(location = 1) varying vec3 v_World;
//layout(location = 2) flat varying vec3 v_PolyNormal;
layout(location = 3) flat varying int v_TargetIndex;
layout(location = 4) flat varying uint v_EncodedOrigin;
layout(location = 5) flat varying vec2 v_Normal;

layout(set = 0, binding = 0) uniform c_Globals {
    vec4 u_TargetScale;
    vec4 u_Penetration; // X=hard threshold
};

#ifdef SHADER_VS
//imported: Polygon, get_shape_polygon

layout(set = 0, binding = 2, std430) buffer Storage {
    Body s_Bodies[];
};

// Compute the exact collision vector instead of using the origin
// of the input polygon.
const float EXACT_VECTOR = 0.0;

layout(set = 3, binding = 0) uniform c_Locals {
    uvec2 u_Indices;
};

void main() {
    Polygon poly = get_shape_polygon();
    vec4 pos_scale = s_Bodies[u_Indices.x].pos_scale;
    vec4 orientation = s_Bodies[u_Indices.x].orientation;
    float scale = pos_scale.w * s_Bodies[u_Indices.x].physics.scale.y;
    vec3 base_pos = qrot(orientation, poly.vertex.xyz) * scale;
    v_World = base_pos + pos_scale.xyz;

    //v_Vector = mix(mat3(u_Model) * poly.origin, v_World - u_Model[3].xyz, EXACT_VECTOR);
    //v_PolyNormal = poly.normal;
    v_TargetIndex = int(u_Indices.y) + gl_InstanceIndex;
    v_EncodedOrigin = encode_pos(poly.origin);
    v_Normal = poly.normal.xy;

    vec3 pos = base_pos * u_TargetScale.xyz;
    pos.z = pos.z * 0.5 + 0.5; // convert Z into [0, 1]:
    gl_Position = vec4(pos, 1.0);
}
#endif //VS


#ifdef SHADER_FS
//imported: Surface, get_surface

layout(set = 0, binding = 1, std430) buffer Collision {
    CollisionPolygon s_Collisions[];
};

layout(location = 0) out float o_Dummy;

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

    if (depth_raw != 0.0) {
        //TODO: avoid doing this on every FS invocation
        s_Collisions[v_TargetIndex].middle = v_EncodedOrigin;
        //s_Collisions[v_TargetIndex].normal = v_Normal;

        //HACK: convince Metal driver that we are actually using the buffer...
        // the atomic operations appear to be ignored otherwise
        s_Collisions[0].depth_soft += 1;
        s_Collisions[0].depth_hard += 1;

        uint encoded = encode_depth(depth_raw);
        if (depth_raw >= u_Penetration.x) {
            atomicAdd(s_Collisions[v_TargetIndex].depth_hard, encoded);
        } else {
            atomicAdd(s_Collisions[v_TargetIndex].depth_soft, encoded);
        }
    }

    o_Dummy = depth_raw/8.0;
}
#endif //FS
