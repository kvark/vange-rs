//!include cs:body.inc

layout(set = 0, binding = 0, std430) buffer Storage {
    Body s_Bodies[];
};

layout(set = 0, binding = 2, std140) uniform Constants {
    GlobalConstants u_Constants;
};

struct Push {
    vec4 dir_id;
};

layout(set = 1, binding = 0, std430) readonly buffer Pushes {
    Push s_Pushes[];
};

#ifdef SHADER_CS

void main() {
    Push push = s_Pushes[gl_LocalInvocationIndex];
    if (push.dir_id.w < 0.0) {
        return;
    }
    int index = int(push.dir_id.w);
    float device_modulation = 1.0;
    float dt_impulse = 1.0;

    float scale = s_Bodies[index].pos_scale.w;
    float mass = u_Constants.nature.y * s_Bodies[index].model.jacobi0.w * scale * scale;
    float f = device_modulation * u_Constants.force.x * dt_impulse / pow(mass, 0.3);

    s_Bodies[index].v_linear.xyz += f * push.dir_id.xyz;
}
#endif //CS
