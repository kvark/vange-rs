//!include vs:shape.inc fs:surface.inc

//layout(location = 0) flat varying vec3 v_Vector;
layout(location = 1) varying vec3 v_World;
//layout(location = 2) flat varying vec3 v_PolyNormal;
layout(location = 3) flat varying int v_TargetIndex;

layout(set = 0, binding = 0) uniform c_Globals {
    vec4 u_TargetScale;
    vec4 u_Penetration; // X=scale, Y=limit
};

#ifdef SHADER_VS
//imported: Polygon, get_shape_polygon

// Compute the exact collision vector instead of using the origin
// of the input polygon.
const float EXACT_VECTOR = 0.0;

layout(set = 3, binding = 0) uniform c_Locals {
    mat4 u_Model;
    vec4 u_ModelScale;
    uvec4 u_IndexOffset;
};

void main() {
    Polygon poly = get_shape_polygon();
    v_World = (u_Model * poly.vertex).xyz;

    //v_Vector = mix(mat3(u_Model) * poly.origin, v_World - u_Model[3].xyz, EXACT_VECTOR);
    //v_PolyNormal = poly.normal;
    v_TargetIndex = int(u_IndexOffset.x) + gl_InstanceIndex;

    //vec2 pos = poly.vertex.xy * u_ModelScale.xy * u_TargetScale.xy;
    vec3 pos = mat3(u_Model) * poly.vertex.xyz * u_TargetScale.xyz;
    pos.z = pos.z * 0.5 + 0.5; // convert Z into [0, 1]:
    gl_Position = vec4(pos, 1.0);
}
#endif //VS


#ifdef SHADER_FS
//imported: Surface, get_surface

layout(set = 0, binding = 1, std430) buffer Storage {
    //Note: using `uvec2` here fails to compile on Metal:
    //> error: address of vector element requested
    uint s_Data[];
};

const uint MAX_DEPTH = 255;
const uint DEPTH_BITS = 20;

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

    //HACK: convince Metal driver that we are actually using the buffer...
    // the atomic operations appear to be ignored otherwise
    s_Data[0] += 1;

    if (depth_raw != 0.0) {
        uint effective_depth = min(uint(depth_raw), MAX_DEPTH);
        atomicAdd(s_Data[v_TargetIndex], effective_depth + (1U<<DEPTH_BITS));
    }
}
#endif //FS
