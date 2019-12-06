/// Common quaternion operations.

/// Create a quaternion from axis and angle.
vec4 qmake(vec3 axis, float angle) {
    return vec4(axis * sin(angle), cos(angle));
}

/// Rotate a vector.
vec3 qrot(vec4 q, vec3 v)   {
    return v + 2.0*cross(q.xyz, cross(q.xyz,v) + q.w*v);
}

/// Combine quaternions.
vec4 qmul(vec4 a, vec4 b)   {
    return vec4(cross(a.xyz,b.xyz) + a.xyz*b.w + b.xyz*a.w, a.w*b.w - dot(a.xyz,b.xyz));
}

/// Invert a quaternion.
vec4 qinv(vec4 q)   {
    return vec4(-q.xyz,q.w);
}
