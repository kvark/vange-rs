#![allow(irrefutable_let_patterns)]

use log::info;

#[path = "../boilerplate.rs"]
mod boilerplate;
mod game;

/// Vangers game prototype
#[derive(clap::Parser)]
struct Cli {}

fn main() {
    use clap::Parser as _;
    let _cli = Cli::parse();

    let (harness, settings) =
        boilerplate::Harness::init(boilerplate::HarnessOptions { title: "road" });

    info!("Parsing command line");
    let game = game::Game::new(&settings, &harness.graphics_ctx);

    harness.main_loop(game);
}
