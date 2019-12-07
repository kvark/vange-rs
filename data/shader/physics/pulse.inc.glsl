// Common math for pulse application

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
