//! The car physics' transcendental functions, through `libm` on every
//! target. The server steps natively and the web client as wasm; std's
//! `powf`, `ln` and `sin_cos` come from the platform's C library natively
//! (glibc, the macOS and Windows runtimes) and from Rust's own port of
//! musl in wasm, and these differ in the last bit. One flipped bit in the
//! first quantum grows into a different tumble within seconds, which the
//! web client then shows as server corrections. `libm` gives the same
//! bits everywhere.

use glam::{Quat, Vec3};

#[inline]
pub fn powf(x: f32, y: f32) -> f32 {
    libm::powf(x, y)
}

#[inline]
pub fn ln(x: f32) -> f32 {
    libm::logf(x)
}

#[inline]
pub fn exp2(x: f32) -> f32 {
    libm::exp2f(x)
}

#[inline]
pub fn sin_cos(x: f32) -> (f32, f32) {
    libm::sincosf(x)
}

/// `Quat::from_axis_angle`, with `libm`'s sine and cosine.
#[inline]
pub fn quat_from_axis_angle(axis: Vec3, angle: f32) -> Quat {
    let (s, c) = sin_cos(angle * 0.5);
    let v = axis * s;
    Quat::from_xyzw(v.x, v.y, v.z, c)
}
