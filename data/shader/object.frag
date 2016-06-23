#version 150 core

uniform sampler1D t_Palette;

in vec3 v_Normal;
out vec4 Target0;


void main() {
	//TODO
	float color_id = 0.5;
	Target0 = texture(t_Palette, color_id);
}
