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
    vec4 model; // X = avg radius, Y = volume
    mat4 jacobian_inv;
    Physics physics;
    vec4 wheels[MAX_WHEELS]; //XYZ = position, W = steer
};
