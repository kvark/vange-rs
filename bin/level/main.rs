extern crate cgmath;
extern crate getopts;
extern crate gfx;
#[macro_use]
extern crate log;
extern crate vangers;

mod app;
#[path = "../boilerplate.rs"]
mod boilerplate;

fn main() {
    let (mut harness, settings, main_targets) = boilerplate::Harness::init();

    let app = app::LevelView::new(&settings, main_targets, &mut harness.factory);

    harness.main_loop(app);
}
