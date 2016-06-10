#version 150 core

in ivec4 a_Pos;

void main() {
    gl_Position = vec4(a_Pos);
}
