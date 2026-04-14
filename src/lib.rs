#![allow(
    // We don't use syntax sugar where it's not necessary.
    clippy::match_like_matches_macro,
    // Redundant matching is more explicit.
    clippy::redundant_pattern_matching,
    // Explicit lifetimes are often easier to reason about.
    clippy::needless_lifetimes,
    // No need for defaults in the internal types.
    clippy::new_without_default,
    // Matches are good and extendable, no need to make an exception here.
    clippy::single_match,
    // Push commands are more regular than macros.
    clippy::vec_init_then_push,
    // Let me decide when it's too many.
    clippy::too_many_arguments,
    // Keep the symmetry in operations.
    clippy::identity_op,
    clippy::erasing_op,
)]
#![warn(
    trivial_casts,
    trivial_numeric_casts,
    unused_extern_crates,
    rust_2018_idioms,
    future_incompatible,
    nonstandard_style,
    // We don't match on a reference, unless required.
    clippy::pattern_type_mismatch,
)]

#[macro_use]
extern crate log;
#[macro_use]
extern crate serde_derive;

pub mod config;
pub mod level;
pub mod model;
pub mod physics;
pub mod render;
pub mod space;
pub mod vfs;

#[cfg(feature = "web")]
pub mod data;
