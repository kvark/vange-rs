//!include globals.inc

struct Debug {
    color: vec4<f32>,
};

@group(1) @binding(0) var<uniform> c_Debug: Debug;

struct Varyings {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn main_vs(@location(0) pos: vec4<f32>, @location(1) color: vec4<f32>) -> Varyings {
    return Varyings(
        u_Globals.view_proj * pos,
        color,
    );
}

@fragment
fn main_fs(in: Varyings) -> @location(0) vec4<f32> {
    return in.color;
}
