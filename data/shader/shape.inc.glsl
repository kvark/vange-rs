// Common VS routines for fetching the collision shape data.

#ifdef SHADER_VS

uniform samplerBuffer t_Position;

in uvec4 a_Indices;
in vec4 a_Normal;
in vec4 a_OriginSquare;

struct Polygon {
	vec4 vertex;
	vec3 origin;
	vec3 normal;
	float square;
};

Polygon get_shape_polygon() {
	uint index = a_Indices.xywz[gl_VertexID];
	Polygon poly = Polygon(
		texelFetch(t_Position, int(index)),
		a_OriginSquare.xyz,
		normalize(a_Normal.xyz),
		a_OriginSquare.w
	);
	return poly;
}

#endif