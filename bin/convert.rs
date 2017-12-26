extern crate env_logger;
extern crate getopts;
extern crate vangers;

use vangers::{config, model};

fn main() {
    use std::env;
    env_logger::init().unwrap();

    let settings = config::settings::Settings::load("config/settings.toml");

    let args: Vec<_> = env::args().collect();
    let mut options = getopts::Options::new();
    options
        .parsing_style(getopts::ParsingStyle::StopAtFirstFree)
        .optflag("h", "help", "print this help menu");

    let matches = options.parse(&args[1 ..]).unwrap();
    if matches.opt_present("h") || matches.free.len() != 2 {
        println!("Vangers model to Wavefront OBJ converter");
        let brief = format!(
            "Usage: {} [options] <path_to_model> <destination_path>",
            args[0]
        );
        println!("{}", options.usage(&brief));
        return;
    }

    let file = settings.open_relative(&matches.free[0]);
    model::convert_m3d(file, matches.free[1].as_str().into());
}
