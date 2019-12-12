// Common VS routines for fetching the collision shape data.
// requires "encode.inc"

#ifdef SHADER_VS

//layout(set = 1, location = 0) uniform textureBuffer t_Position;

layout(location = 0) in uvec4 a_Indices;
layout(location = 1) in vec4 a_Normal;
layout(location = 2) in vec4 a_OriginSquare;

layout(set = 2, binding = 0, std430) readonly buffer Positions
{
    uint r_Positions[];
};

struct Polygon {
    vec4 vertex;
    vec3 origin;
    vec3 normal;
    float square;
};

Polygon get_shape_polygon() {
    uint index = a_Indices.xywz[gl_VertexIndex];
    vec3 pos = decode_pos(r_Positions[int(index)]);
    Polygon poly = Polygon(
        vec4(pos, 1.0),
        a_OriginSquare.xyz,
        normalize(a_Normal.xyz),
        a_OriginSquare.w
    );
    return poly;
}

#endif
