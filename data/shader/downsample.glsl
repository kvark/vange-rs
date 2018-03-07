varying vec2 v_TexCoord;


#ifdef SHADER_VS

attribute vec2 a_SourcePos;
attribute vec2 a_DestPos;

void main() {
    gl_Position = vec4(a_DestPos, 0.0, 1.0);
    v_TexCoord = a_SourcePos;
}
#endif //VS


#ifdef SHADER_FS

uniform sampler2D t_Source;

out vec4 Target0;

void main() {
    Target0 = texture(t_Source, v_TexCoord);
}
#endif //FS
