use log::info;

#[path = "../boilerplate.rs"]
mod boilerplate;
mod game;
mod physics;

fn main() {
    use std::env;

    let (mut harness, settings) = boilerplate::Harness::init(boilerplate::HarnessOptions {
        title: "road",
        uses_level: true,
    });

    info!("Parsing command line");
    let args: Vec<_> = env::args().collect();
    let mut options = getopts::Options::new();
    options
        .parsing_style(getopts::ParsingStyle::StopAtFirstFree)
        .optflag("h", "help", "print this help menu");

    let matches = options.parse(&args[1..]).unwrap();
    if matches.opt_present("h") || !matches.free.is_empty() {
        println!("Vangers game prototype");
        let brief = format!("Usage: {} [options]", args[0]);
        println!("{}", options.usage(&brief));
        return;
    }

    let game = game::Game::new(
        &settings,
        harness.extent,
        &harness.device,
        &mut harness.queue,
    );

    harness.main_loop(game);
}
