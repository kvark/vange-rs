// Common VS routines for fetching the collision shape data.

#ifdef SHADER_VS

//layout(set = 1, location = 0) uniform textureBuffer t_Position;

layout(location = 0) in uvec4 a_Indices;
layout(location = 1) in vec4 a_Normal;
layout(location = 2) in vec4 a_OriginSquare;

layout(set = 2, binding = 0, std430) buffer Positions
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
    uint pos = r_Positions[int(index)];
    uvec3 pos_vec = (uvec3(pos) >> uvec3(0, 8, 16)) & uvec3(0xFFU);
    Polygon poly = Polygon(
        vec4(vec3(pos_vec) - 128.0, 1.0),
        a_OriginSquare.xyz,
        normalize(a_Normal.xyz),
        a_OriginSquare.w
    );
    return poly;
}

#endif
