#![deny(
    trivial_casts,
    trivial_numeric_casts,
    //unused,
    unused_qualifications,
    rust_2018_compatibility,
    rust_2018_idioms,
    future_incompatible,
    nonstandard_style,
)]
#![allow(
    clippy::identity_op,
    clippy::erasing_op,
    clippy::new_without_default,
    missing_copy_implementations,
    missing_debug_implementations
)]

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
