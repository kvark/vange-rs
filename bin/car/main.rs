extern crate cgmath;
extern crate getopts;
extern crate gfx;
#[macro_use]
extern crate log;
extern crate m3d;
extern crate vangers;

mod app;
#[path = "../boilerplate.rs"]
mod boilerplate;

fn main() {
    use std::env;

    let (mut harness, settings, main_targets) = boilerplate::Harness::init();

    let args: Vec<_> = env::args().collect();
    let mut options = getopts::Options::new();
    //TODO: normals on/off
    //TODO: collision volumes on/off
    //TODO: render all vehicles, by mask
    options
        .parsing_style(getopts::ParsingStyle::StopAtFirstFree)
        .optflag("h", "help", "print this help menu");

    let matches = options.parse(&args[1 ..]).unwrap();
    if matches.opt_present("h") || !matches.free.is_empty() {
        println!("Vangers mechos explorer");
        let brief = format!("Usage: {} [options]", args[0]);
        println!("{}", options.usage(&brief));
        return;
    }

    let app = app::CarView::new(&settings, main_targets, &mut harness.factory);

    harness.main_loop(app);
}
