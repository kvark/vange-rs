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
    use std::env;

    let (mut harness, settings, main_targets) = boilerplate::Harness::init();

    let args: Vec<_> = env::args().collect();
    let mut options = getopts::Options::new();
    options
        .parsing_style(getopts::ParsingStyle::StopAtFirstFree)
        .optflag("h", "help", "print this help menu");

    let matches = options.parse(&args[1 ..]).unwrap();
    if matches.opt_present("h") || matches.free.len() != 1 {
        println!("Vangers model viewer");
        let brief = format!("Usage: {} [options] <path_to_model>", args[0]);
        println!("{}", options.usage(&brief));
        return;
    }

    let path = &matches.free[0];
    let app = app::ResourceView::new(path, &settings, main_targets, &mut harness.factory);

    harness.main_loop(app);
}
