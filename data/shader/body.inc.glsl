struct Data {
    vec4 pos_scale;
    vec4 rot;
    vec4 linear;
    vec4 angular;
    uvec4 car_id;
};

layout(set = 0, binding = 0, std430) buffer Storage {
    Data s_Data[];
};

layout(set = 0, binding = 1, std430) uniform Uniforms {
    vec4 u_GlobalForce;
};

struct Car {
    vec4 zomc; // Z offset of mass center
    mat4 jacobian_inv;
};

layout(set = 0, binding = 2, std430) readonly buffer Cars {
    Car s_Cars[];
};
