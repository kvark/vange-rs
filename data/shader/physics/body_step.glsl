//!include cs:body.inc cs:quat.inc

#ifdef SHADER_CS

layout(set = 0, binding = 0, std430) buffer Storage {
    Body s_Bodies[];
};

layout(set = 0, binding = 1, std140) uniform Uniforms {
    vec4 u_Delta;
};

layout(set = 0, binding = 2, std140) uniform Constants {
    vec4 u_Nature; // X = time delta0, Z = gravity
    vec4 u_Car; // X = rudder step, Y = rudder max, Z = tracktion incr, W = tracktion decr
    vec2 u_ImpulseElastic; // X = restriction, Y = time scale
    vec4 u_Impulse; // X = rolling scale, Y = normal_threshold, Z = K_wheel, W = K_friction
    vec2 u_DragFree;
    vec2 u_DragSpeed;
    vec2 u_DragSpring;
    vec2 u_DragAbsMin;
    vec2 u_DragAbsStop;
    vec2 u_DragColl;
};

vec4 apply_control(vec4 engine, vec4 control) {
    const float max_tracktion = 4.0;
    if (control.x != 0.0) {
        engine.x = clamp(
            engine.x + u_Car.x * 2.0 * u_Delta.x * control.x,
            -u_Car.y,
            u_Car.y
        );
    }
    if (control.y != 0.0) {
        engine.y = clamp(
            engine.y + control.y * u_Delta.x * u_Car.z,
            -max_tracktion,
            max_tracktion
        );
    }
    if (control.z != 0.0 && engine.y != 0.0) {
        engine.y *= exp2(-u_Delta.x);
    }
    return engine;
}

void main() {
    uint index =
        gl_GlobalInvocationID.z * gl_WorkGroupSize.x * gl_NumWorkGroups.x * gl_WorkGroupSize.y * gl_NumWorkGroups.y +
        gl_GlobalInvocationID.y * gl_WorkGroupSize.x * gl_NumWorkGroups.x +
        gl_GlobalInvocationID.x;
    Body body = s_Bodies[index];

    vec4 engine = apply_control(body.engine, body.control);

    float speed_correction_factor = u_Delta.x / u_Nature.x;
    vec3 vel = body.linear.xyz;
    vec3 wel = body.angular.xyz;

    vec2 mag = vec2(length(vel), length(wel));
    vec2 drag = u_DragFree.xy * pow(u_DragSpeed, vec2(mag.x, mag.y*mag.y));

    vec4 irot = qinv(body.orientation);
    vec3 z_axis = qrot(irot, vec3(0.0, 0.0, 1.0));
    bool spring_touch = dot(body.springs, body.springs) != 0.0;
    bool wheels_touch = z_axis.z > 0.0 && spring_touch;
    bool stand_on_wheels = z_axis.z > 0.0 &&
        abs(qrot(body.orientation, vec3(1.0, 0.0, 0.0)).z) < 0.7;

    if (spring_touch) {
        drag *= u_DragSpring;
    }

    vec3 v_accel = qrot(irot, vec3(0.0, 0.0, body.springs.z - u_Nature.z));
    vec3 w_accel = qrot(irot, vec3(body.springs.xy, 0.0));

    vec3 tmp = vec3(0.0, 0.0, body.radius_volume_zomc_scale.z * body.pos_scale.w);
    w_accel -= u_Nature.z * cross(tmp, z_axis);

    vel += u_Delta.x * v_accel;
    wel += u_Delta.x * (mat3(body.jacobian_inv) * w_accel);

    if (stand_on_wheels && all(lessThan(mag, u_DragAbsMin))) {
        drag *= pow(u_DragColl, drag / max(mag, vec2(0.01)));
    }

    if (any(greaterThan(mag * drag, u_DragAbsStop))) {
        vec3 local_z_scaled = z_axis * (body.radius_volume_zomc_scale.x * u_Impulse.x);
        float r_diff_sign = 1.0; //TODO: down_minus_up.signum() as f32;
        vec3 vs = vel - r_diff_sign * cross(local_z_scaled, wel);

        vec4 vel_rot_inv = qmake(wel / max(mag.y, 0.01), -u_Delta.x * mag.y);
        vel = qrot(vel_rot_inv, vel);
        wel = qrot(vel_rot_inv, wel);
        s_Bodies[index].pos_scale.xyz = body.pos_scale.xyz + qrot(body.orientation, vs) * u_Delta.x;
        s_Bodies[index].orientation = qmul(body.orientation, qinv(vel_rot_inv));
    }

    vec2 drag_corrected = pow(drag, vec2(speed_correction_factor));
    vel *= drag_corrected.x;
    wel *= drag_corrected.y;

    s_Bodies[index].engine = engine;
    s_Bodies[index].linear.xyz = vel;
    s_Bodies[index].angular.xyz = wel;
    s_Bodies[index].springs.xyz = vec3(0.0);
}
#endif //CS
