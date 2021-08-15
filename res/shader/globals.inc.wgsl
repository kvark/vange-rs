[[block]]
struct Globals {
    camera_pos: vec4<f32>;
    view_proj: mat4x4<f32>;
    inv_view_proj: mat4x4<f32>;
    light_view_proj: mat4x4<f32>;
    light_pos: vec4<f32>;
    light_color: vec4<f32>; // not used
};

[[group(0), binding(0)]] var<uniform> u_Globals: Globals;
