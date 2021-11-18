struct Physics {
    scale: vec4<f32>; // size, bound, box, z offset of center
    mobility_ship: vec4<f32>; // X = mobility, YZW = water
    speed: vec4<f32>; // main, water, air, underground
};

struct Model {
    jacobi0: vec4<f32>; // W = volume
    jacobi1: vec4<f32>; // W = avg radius
    jacobi2: vec4<f32>;
};

fn calc_j_inv(m: Model, scale: f32) -> mat3x3<f32> {
    return mat3x3<f32>(m.jacobi0.xyz, m.jacobi1.xyz, m.jacobi2.xyz) *
        (m.jacobi0.w / (scale * scale));
}

struct Body {
    control: vec4<f32>; // X=steer, Y=motor, Z = k_turbo, W = f_brake
    engine: vec4<f32>; // X=rudder, Y=traction
    pos_scale: vec4<f32>;
    orientation: vec4<f32>;
    v_linear: vec4<f32>;
    v_angular: vec4<f32>;
    springs: vec4<f32>;
    model: Model;
    physics: Physics;
    wheels: array<vec4<f32>, 4>; //XYZ = position, W = steer
};

struct DragConstants {
    free: vec2<f32>;
    speed: vec2<f32>;
    spring: vec2<f32>;
    abs_min: vec2<f32>;
    abs_stop: vec2<f32>;
    coll: vec2<f32>;
    other: vec2<f32>; // X = wheel speed, Y = drag Z
    padding: vec2<f32>;
};

struct GlobalConstants {
    nature: vec4<f32>; // X = time delta0, Y = density, Z = gravity
    global_speed: vec4<f32>; // X = main, Y = water, Z = air, W = underground
    global_mobility: vec4<f32>; // X = mobility
    car_rudder: vec4<f32>; // X = step, Y = max, Z = decr
    car_traction: vec4<f32>; // X = incr, Y = decr
    impulse_elastic: vec4<f32>; // X = restriction, Y = time scale
    impulse_factors: vec4<f32>;
    impulse: vec4<f32>; // X = rolling scale, Y = normal_threshold, Z = K_wheel, W = K_friction
    drag: DragConstants;
    contact_elastic: vec4<f32>; // X = wheel, Y = spring, Z = xy, W = db collision
    force: vec4<f32>; // X = k_distance_to_force
};
