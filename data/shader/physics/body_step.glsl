//!include cs:body.inc cs:physics/pulse.inc cs:quat.inc

#ifdef SHADER_CS

const float EPSILON = 1e-10;
const float MAX_TRACTION = 4.0;

layout(set = 0, binding = 0, std430) buffer Storage {
    Body s_Bodies[];
};

layout(set = 0, binding = 1, std140) uniform Uniforms {
    vec4 u_Delta;
};

layout(set = 0, binding = 2, std140) uniform Constants {
    GlobalConstants u_Constants;
};

vec4 apply_control(vec4 engine, vec4 control) {
    if (control.x != 0.0) {
        engine.x = clamp(
            engine.x + control.x * 2.0 * u_Delta.x * u_Constants.car_rudder.x,
            -u_Constants.car_rudder.y,
            u_Constants.car_rudder.y
        );
    }
    if (control.y != 0.0) {
        engine.y = clamp(
            engine.y + control.y * u_Delta.x * u_Constants.car_traction.x,
            -MAX_TRACTION,
            MAX_TRACTION
        );
    }
    if (control.w != 0.0 && engine.y != 0.0) {
        engine.y *= exp2(-u_Delta.x);
    }
    return engine;
}

vec4 slow_down(vec4 engine, float velocity, bool wheels_touch) {
    // unsteer
    if (engine.x != 0.0 && wheels_touch) {
        float change = engine.x * velocity * u_Delta.x * u_Constants.car_rudder.z;
        engine.x -= sign(engine.x) * abs(change);
    }
    // slow traction
    float old = engine.y;
    engine.y = clamp(
        old - sign(old) * u_Delta.x * u_Constants.car_traction.y,
        -MAX_TRACTION,
        MAX_TRACTION
    );
    if (old * engine.y < 0.0) {
        engine.y = 0.0;
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

    float speed_correction_factor = u_Delta.x / u_Constants.nature.x;
    vec3 vel = body.v_linear.xyz;
    vec3 wel = body.v_angular.xyz;

    vec2 drag = u_Constants.drag.free.xy *
        pow(u_Constants.drag.speed, vec2(length(vel), dot(wel, wel)));

    vec4 irot = qinv(body.orientation);
    vec3 z_axis = qrot(irot, vec3(0.0, 0.0, 1.0));
    bool spring_touch = dot(body.springs, body.springs) != 0.0;
    bool wheels_touch = z_axis.z > 0.0 && spring_touch;
    bool stand_on_wheels = z_axis.z > 0.0 &&
        abs(qrot(body.orientation, vec3(1.0, 0.0, 0.0)).z) < 0.7;
    bool after_collision = false; //TODO

    vec3 v_accel = qrot(irot, vec3(0.0, 0.0, body.springs.z - u_Constants.nature.z));
    vec3 w_accel = qrot(irot, vec3(body.springs.xy, 0.0));
    mat3 j_inv = calc_j_inv(body.model, body.pos_scale.w);

    if (wheels_touch) {
        float speed = log(u_Constants.drag.other.x) * body.physics.mobility_ship.x
            * u_Constants.global_speed.x / body.physics.speed.x;
        vel.y *= pow(1.0 + speed, speed_correction_factor);
    }

    if (wheels_touch && stand_on_wheels) {
        v_accel.y += body.physics.mobility_ship.x *
            u_Constants.global_mobility.x * engine.y * body.control.z;
        vec3 rudder_vec = vec3(cos(engine.x), -sin(engine.x), 0.0);

        for (int i=0; i<MAX_WHEELS; ++i) {
            if (body.wheels[i].w != 0.0) {
                vec3 pos = body.wheels[i].xyz * body.pos_scale.w;
                vec3 vw = vel + cross(wel, pos);
                v_accel -= vw * body.control.w;

                if (!after_collision) {
                    vec3 normal = body.wheels[i].w > 0.0 ? rudder_vec : vec3(1.0, 0.0, 0.0);
                    vec3 u0 = normal * dot(vw, normal);
                    mat3 mx = calc_collision_matrix_inv(pos, j_inv);
                    vec3 pulse = -u_Constants.impulse.z * (mx * u0);
                    vel += pulse;
                    wel += j_inv * cross(pos, pulse);
                }
            }
        }
    }

    if (spring_touch) {
        drag *= u_Constants.drag.spring;
    }

    if (spring_touch || wheels_touch) {
        vec3 tmp = vec3(0.0, 0.0, body.physics.scale.w * body.pos_scale.w);
        w_accel -= u_Constants.nature.z * cross(tmp, z_axis);
        float vz = dot(z_axis, vel);
        if (vz < -10.0) {
            drag.x *= pow(u_Constants.drag.other.y, -vz);
        }
    }

    vel += u_Delta.x * v_accel;
    wel += u_Delta.x * (j_inv * w_accel);
    vec2 mag = vec2(length(vel), length(wel));

    // Static friction
    if ((wheels_touch || spring_touch) && all(lessThan(mag, u_Constants.drag.abs_min))) {
        drag *= pow(u_Constants.drag.coll, u_Constants.drag.abs_min / (mag + EPSILON));
    }

    if (any(greaterThan(mag * drag, u_Constants.drag.abs_stop))) {
        vec3 local_z_scaled = (body.model.jacobi1.w * u_Constants.impulse.x) * z_axis;
        float r_diff_sign = sign(z_axis.z);
        vec3 vs = vel - r_diff_sign * cross(local_z_scaled, wel);

        vec4 vel_rot_inv = qmake(wel / (mag.y + EPSILON), -u_Delta.x * mag.y);
        vel = qrot(vel_rot_inv, vel);
        wel = qrot(vel_rot_inv, wel);
        s_Bodies[index].pos_scale.xyz = body.pos_scale.xyz + qrot(body.orientation, vs) * u_Delta.x;
        s_Bodies[index].orientation = normalize(qmul(body.orientation, qinv(vel_rot_inv)));
    }

    vec2 drag_corrected = pow(drag, vec2(speed_correction_factor));
    vel *= drag_corrected.x;
    wel *= drag_corrected.y;

    s_Bodies[index].engine = slow_down(engine, vel.y, wheels_touch);
    s_Bodies[index].v_linear.xyz = vel;
    s_Bodies[index].v_angular.xyz = wel;
    s_Bodies[index].springs.xyz = vec3(0.0);
}
#endif //CS
