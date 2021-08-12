#![deny(
    trivial_casts,
    trivial_numeric_casts,
    unused,
    unused_qualifications,
    rust_2018_compatibility,
    rust_2018_idioms,
    future_incompatible,
    nonstandard_style,
    missing_copy_implementations
)]
#![allow(clippy::identity_op, clippy::erasing_op)]

#[macro_use]
extern crate log;
#[macro_use]
extern crate serde_derive;

pub mod config;
mod freelist;
pub mod level;
pub mod model;
pub mod render;
pub mod space;
