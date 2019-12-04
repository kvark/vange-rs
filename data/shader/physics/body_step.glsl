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
    vec4 u_GlobalSpeed; // X = main, Y = water, Z = air, W = underground
    vec4 u_GlobalMobility; // X = mobility
    vec4 u_Car; // X = rudder step, Y = rudder max, Z = tracktion incr, W = tracktion decr
    vec4 u_ImpulseElastic; // X = restriction, Y = time scale
    vec4 u_Impulse; // X = rolling scale, Y = normal_threshold, Z = K_wheel, W = K_friction
    vec2 u_DragFree;
    vec2 u_DragSpeed;
    vec2 u_DragSpring;
    vec2 u_DragAbsMin;
    vec2 u_DragAbsStop;
    vec2 u_DragColl;
    vec2 u_Drag; // X = wheel speed, Y = drag Z
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

mat3 calc_collision_matrix_inv(vec3 r, mat3 ji) {
    vec3 a = -r.z * ji[1] + r.y * ji[2]; // 12, 3, 7
    vec3 b = r.z * ji[0] - r.x * ji[2]; // 30, 21, 25
    vec3 c = -r.y * ji[0] + r.x * ji[1]; // 48, 39, 43
    mat3 cm = mat3(
        vec3(1.0, 0.0, 0.0) + a.zxy * r.yzx - a.yzx * r.zxy,
        vec3(0.0, 1.0, 0.0) + b.zxy * r.yzx - b.yzx * r.zxy,
        vec3(0.0, 0.0, 1.0) + c.zxy * r.yzx - c.yzx * r.zxy
    );
    return inverse(cm);
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
    bool after_collision = false; //TODO

    vec3 v_accel = qrot(irot, vec3(0.0, 0.0, body.springs.z - u_Nature.z));
    vec3 w_accel = qrot(irot, vec3(body.springs.xy, 0.0));

    if (wheels_touch) {
        float speed = log(u_Drag.x) * body.physics.mobility_ship.x
            * u_GlobalSpeed.x / body.physics.speed.x;
        vel.y *= pow(1.0 + speed, speed_correction_factor);
    }

    if (wheels_touch && stand_on_wheels) {
        v_accel.y += body.physics.mobility_ship.x *
            u_GlobalMobility.x * engine.y * body.control.z;
        vec3 rudder_vec = vec3(cos(engine.x), -sin(engine.x), 0.0);
        mat3 j_inv = mat3(body.jacobian_inv);

        for (int i=0; i<MAX_WHEELS; ++i) {
            if (body.wheels[i].w != 0.0) {
                vec3 pos = body.wheels[i].xyz * body.pos_scale.w;
                vec3 vw = vel + cross(wel, pos);
                v_accel -= vw * body.control.w;

                if (!after_collision) {
                    vec3 normal = body.wheels[i].w > 0.0 ? rudder_vec : vec3(1.0, 0.0, 0.0);
                    vec3 u0 = normal * dot(vw, normal);
                    mat3 mx = calc_collision_matrix_inv(pos, j_inv);
                    vec3 pulse = -u_Impulse.z * (mx * u0);
                    vel += pulse;
                    wel += j_inv * cross(pos, pulse);
                }
            }
        }
    }

    if (spring_touch) {
        drag *= u_DragSpring;
    }

    if (spring_touch || wheels_touch) {
        vec3 tmp = vec3(0.0, 0.0, body.physics.scale.w * body.pos_scale.w);
        w_accel -= u_Nature.z * cross(tmp, z_axis);
        float vz = dot(z_axis, vel);
        if (vz < -10.0) {
            drag.x *= pow(u_Drag.y, -vz);
        }
    }

    vel += u_Delta.x * v_accel;
    wel += u_Delta.x * (mat3(body.jacobian_inv) * w_accel);

    if (stand_on_wheels && all(lessThan(mag, u_DragAbsMin))) {
        drag *= pow(u_DragColl, drag / max(mag, vec2(0.01)));
    }

    if (any(greaterThan(mag * drag, u_DragAbsStop))) {
        vec3 local_z_scaled = (body.model.x * u_Impulse.x) * z_axis;
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
