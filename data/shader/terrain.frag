#version 150 core

uniform sampler2DArray t_Height;
uniform usampler2DArray t_Meta;

const vec4 c_Scale = vec4(1.0/2048.0, 1.0/16384, 0.1, 0.0);

out vec4 Target0;

void main() {
	vec3 tc = vec3(gl_FragCoord.xy / vec2(800.0, 540.0), 0.0);
	Target0 = vec4(texture(t_Height, tc).xxx, 1.0);
}
