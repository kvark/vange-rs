#define MAX_WHEELS  4

struct Physics {
    vec4 scale; // size, bound, box, z offset of center
    vec4 mobility_ship; // X = mobility, YZW = water
    vec4 speed; // main, water, air, underground
};

struct Body {
    vec4 control; // X=steer, Y=motor, Z = k_turbo, W = f_brake
    vec4 engine; // X=rudder, Y=traction
    vec4 pos_scale;
    vec4 orientation;
    vec4 linear;
    vec4 angular;
    vec4 springs;
    vec4 model; // X = avg radius
    mat4 jacobian_inv;
    Physics physics;
    vec4 wheels[MAX_WHEELS]; //XYZ = position, W = steer
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

struct GlobalConstants {
    vec4 nature; // X = time delta0, Z = gravity
    vec4 global_speed; // X = main, Y = water, Z = air, W = underground
    vec4 global_mobility; // X = mobility
    vec4 car_rudder; // X = step, Y = max, Z = decr
    vec4 car_traction; // X = incr, Y = decr
    vec4 impulse_elastic; // X = restriction, Y = time scale
    vec4 impulse; // X = rolling scale, Y = normal_threshold, Z = K_wheel, W = K_friction
    DragConstants drag;
    vec4 contact_elastic; // X = wheel, Y = spring, Z = xy, W = db collision
};
