struct Data {
    vec4 pos_scale;
    vec4 orientation;
    vec4 linear;
    vec4 angular;
    vec4 collision;
    vec4 volume_zero_zomc; // X=volume, Y=0, Z = Z offset of mass center
    mat4 jacobian_inv;
};

layout(set = 0, binding = 0, std430) buffer Storage {
    Data s_Data[];
};

layout(set = 0, binding = 1, std140) uniform Uniforms {
    vec4 u_GlobalForce;
    vec4 u_Delta;
};
