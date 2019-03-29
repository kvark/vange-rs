// Common VS routines for fetching the collision shape data.

#ifdef SHADER_VS

//layout(set = 1, location = 0) uniform textureBuffer t_Position;

layout(location = 0) in uvec4 a_Indices;
layout(location = 1) in vec4 a_Normal;
layout(location = 2) in vec4 a_OriginSquare;

struct Polygon {
	vec4 vertex;
	vec3 origin;
	vec3 normal;
	float square;
};

Polygon get_shape_polygon() {
	uint index = a_Indices.xywz[gl_VertexIndex];
	Polygon poly = Polygon(
		//texelFetch(t_Position, int(index)),
		vec4(0.0, 0.0, 0.0, 1.0), //TODO
		a_OriginSquare.xyz,
		normalize(a_Normal.xyz),
		a_OriginSquare.w
	);
	return poly;
}

#endif
