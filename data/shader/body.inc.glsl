#define MAX_WHEELS  4

struct Physics {
    vec4 scale; // size, bound, box, z offset of center
    float mobility;
    vec4 speed; // main, water, air, underground
    //vec4 ship;
};

struct Body {
    vec4 control; // X=steer, Y=motor
    vec4 engine; // X=rudder, Y=traction
    vec4 pos_scale;
    vec4 orientation;
    vec4 linear;
    vec4 angular;
    vec4 springs;
    vec2 model; // X = avg radius, Y = volume
    mat4 jacobian_inv;
    Physics physics;
    vec4 wheels[MAX_WHEELS]; //XYZ = position, W = steer
};
