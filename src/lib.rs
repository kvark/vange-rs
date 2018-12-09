extern crate byteorder;
extern crate cgmath;
#[macro_use]
extern crate gfx;
extern crate ini;
#[macro_use]
extern crate log;
extern crate m3d;
extern crate rayon;
extern crate ron;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_scan;
extern crate splay;

//TODO: cleanup externs

pub mod config;
pub mod level;
pub mod model;
pub mod space;
pub mod render;
