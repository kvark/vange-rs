//!include cs:body.inc cs:encode.inc

struct CollisionPolygon {
    uint middle;
    uint depth;
};

layout(set = 0, binding = 0, std430) buffer Storage {
    Body s_Bodies[];
};

struct DragConstants {
    vec2 free;
    vec2 speed;
    vec2 spring;
    vec2 abs_min;
    vec2 abs_stop;
    vec2 coll;
    vec2 other; // X = wheel speed, Y = drag Z
};

layout(set = 0, binding = 2, std140) uniform Constants {
    vec4 u_Nature; // X = time delta0, Z = gravity
    vec4 u_GlobalSpeed; // X = main, Y = water, Z = air, W = underground
    vec4 u_GlobalMobility; // X = mobility
    vec4 u_Car; // X = rudder step, Y = rudder max, Z = traction incr, W = traction decr
    vec4 u_ImpulseElastic; // X = restriction, Y = time scale
    vec4 u_Impulse; // X = rolling scale, Y = normal_threshold, Z = K_wheel, W = K_friction
    DragConstants u_Drag;
    vec4 u_ContactElastic; // X = wheel, Y = spring, Z = xy, W = db collision
};

layout(set = 1, binding = 0, std430) readonly buffer Collision {
    CollisionPolygon s_Collisions[];
};

layout(set = 1, binding = 1, std430) readonly buffer Ranges {
    uint s_Ranges[];
};

#ifdef SHADER_CS

const uint DEPTH_BITS = 20;

void main() {
    uint index =
        gl_GlobalInvocationID.z * gl_WorkGroupSize.x * gl_NumWorkGroups.x * gl_WorkGroupSize.y * gl_NumWorkGroups.y +
        gl_GlobalInvocationID.y * gl_WorkGroupSize.x * gl_NumWorkGroups.x +
        gl_GlobalInvocationID.x;
    uvec2 range = (uvec2(s_Ranges[index]) >> uvec2(0, 16)) & 0xFFFF;

    vec3 springs = vec3(0.0);
    for (uint i=range.x; i<range.y; ++i) {
        CollisionPolygon cp = s_Collisions[i];
        vec3 origin = decode_pos(cp.middle);
        uint depth_count = cp.depth >> DEPTH_BITS;
        if (depth_count != 0U) {
            float depth = (cp.depth & ((1U << DEPTH_BITS) - 1U)) / float(depth_count);
            float df = min(depth * u_ContactElastic.y, u_ImpulseElastic.x);
            springs += vec3(origin.y * depth, -origin.x * depth, depth);
        }
    }

    s_Bodies[index].springs.xyz += springs;
}
#endif //CS
