#define MAX_WHEELS  4

struct Physics {
    vec4 scale; // size, bound, box, z offset of center
    vec4 mobility_ship; // X = mobility, YZW = water
    vec4 speed; // main, water, air, underground
};

struct Model {
    vec4 jacobi0; // W = volume
    vec4 jacobi1; // W = avg radius
    vec4 jacobi2;
};

mat3 calc_j_inv(Model m, float scale) {
    return mat3(m.jacobi0.xyz, m.jacobi1.xyz, m.jacobi2.xyz) *
        (m.jacobi0.w / (scale * scale));
}

struct Body {
    vec4 control; // X=steer, Y=motor, Z = k_turbo, W = f_brake
    vec4 engine; // X=rudder, Y=traction
    vec4 pos_scale;
    vec4 orientation;
    vec4 v_linear;
    vec4 v_angular;
    vec4 springs;
    Model model;
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
    vec2 _pad;
};

struct GlobalConstants {
    vec4 nature; // X = time delta0, Y = density, Z = gravity
    vec4 global_speed; // X = main, Y = water, Z = air, W = underground
    vec4 global_mobility; // X = mobility
    vec4 car_rudder; // X = step, Y = max, Z = decr
    vec4 car_traction; // X = incr, Y = decr
    vec4 impulse_elastic; // X = restriction, Y = time scale
    vec4 impulse_factors;
    vec4 impulse; // X = rolling scale, Y = normal_threshold, Z = K_wheel, W = K_friction
    DragConstants drag;
    vec4 contact_elastic; // X = wheel, Y = spring, Z = xy, W = db collision
    vec4 force; // X = k_distance_to_force
};
