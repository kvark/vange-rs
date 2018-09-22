// Common VS routines for fetching the collision shape data.

uniform samplerBuffer t_Position;

attribute uvec4 a_Indices;
attribute vec4 a_Normal;
attribute vec4 a_OriginSquare;

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
