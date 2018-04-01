#ifdef SHADER_VS
void main() {
    gl_Position = vec4(0.0, 0.0, 0.0, 1.0);
}
#endif //VS


#ifdef SHADER_FS

out vec4 Target0;

void main() {
    Target0 = vec4(0.0);
}
#endif //FS
