// Common routines for compact encoding and decoding positioning data.

uint encode_pos(vec3 pos) {
    uvec3 pos_u = uvec3(clamp(pos + step(pos, vec3(0.0)) * 256.0, vec3(0.0), vec3(255.0)));
    return pos_u.x | (pos_u.y << 8) | (pos_u.z << 16);
}

vec3 decode_pos(uint pos_val) {
    // extract X Y Z coordinates
    uvec3 pos_u = (uvec3(pos_val) >> uvec3(0, 8, 16)) & uvec3(0xFFU);
    // convert from u8 to i8
    return vec3(pos_u) - step(vec3(128.0), vec3(pos_u)) * 256.0;
}
