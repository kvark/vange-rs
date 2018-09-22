//!include tev:surface.inc fs:surface.inc fs:color.inc

uniform c_Globals {
    vec4 u_CameraPos;
    mat4 u_ViewProj;
    mat4 u_InvViewProj;
    vec4 u_LightPos;
    vec4 u_LightColor;
};


#ifdef SHADER_VS

attribute ivec4 a_Pos;

out block {
    vec2 pos;
} Out;

uniform c_Surface {
    vec4 u_TextureScale;    // XY = size, Z = height scale, w = number of layers
};

void main() {
	Out.pos = vec2(a_Pos.xy) * u_TextureScale.xy;
}
#endif //VS

#ifdef SHADER_TEC

layout(vertices = 4) out;

in block {
	vec2 pos;
} In[];

out block {
    vec2 pos;
} Out[];

void main() {	 
	gl_TessLevelOuter[0] = gl_TessLevelOuter[1] = gl_TessLevelOuter[2] = gl_TessLevelOuter[3] = 64.0;
	gl_TessLevelInner[0] = gl_TessLevelInner[1] = 64.0;

	Out[gl_InvocationID].pos = In[gl_InvocationID].pos; 
}
#endif //TEC

#ifdef SHADER_TEV
//imported: Surface, get_surface

layout(quads, equal_spacing, ccw) in;

in block {
    vec2 pos;
} In[];

out vec3 v_Pos;

void main() {	  
    vec2 pos = mix(
        mix(In[0].pos, In[1].pos, gl_TessCoord.x),
        mix(In[3].pos, In[2].pos, gl_TessCoord.x),
        gl_TessCoord.y);

   	Surface suf = get_surface(pos);
   	v_Pos = vec3(pos, suf.high_alt);
    
	gl_Position = u_ViewProj * vec4(v_Pos, 1.0);
}
#endif //TEV

#ifdef SHADER_FS
//imported: Surface, u_TextureScale, get_surface, evaluate_color

in vec3 v_Pos;
out vec4 Target0;

void main() {
	Surface suf = get_surface(v_Pos.xy);
	vec4 color = evaluate_color(suf.high_type, suf.tex_coord, v_Pos.z / u_TextureScale.z, 1.0);
    Target0 = color;
}
#endif //FS
