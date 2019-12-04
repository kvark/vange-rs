#define MAX_WHEELS  4

struct Body {
    vec4 control; // X=steer, Y=motor
    vec4 engine; // X=rudder, Y=tracktion
    vec4 pos_scale;
    vec4 orientation;
    vec4 linear;
    vec4 angular;
    vec4 springs;
    vec4 radius_volume_zomc_scale; // X = avg radius, Y=volume, Z = Z offset of mass center, W = shape scale
    mat4 jacobian_inv;
    vec4 wheels[MAX_WHEELS]; //XYZ = position, W = steer
};
