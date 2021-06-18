#![warn(
    trivial_casts,
    trivial_numeric_casts,
    unused_extern_crates,
    unused_qualifications
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
