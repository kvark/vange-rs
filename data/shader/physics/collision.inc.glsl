// Common definitions for GPU-based terrain collisions.

struct CollisionPolygon {
    uint middle;
    uint depth_soft;
    uint depth_hard;
    //vec2 normal;
};

const uint DEPTH_MAX = 255;
const uint DEPTH_BITS = 20;

CollisionPolygon empty_collision() {
    CollisionPolygon cp;
    cp.middle = 0;
    //cp.normal = vec2(0.0);
    cp.depth_soft = 0;
    cp.depth_hard = 0;
    return cp;
}

uint encode_depth(float depth) {
    return min(uint(depth), DEPTH_MAX) + (1U<<DEPTH_BITS);
}

float resolve_depth(uint depth) {
    uint count = depth >> DEPTH_BITS;
    return count != 0U ? (depth & ((1U << DEPTH_BITS) - 1U)) / float(count) : 0.0;
}
