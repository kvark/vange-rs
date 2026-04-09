#![allow(irrefutable_let_patterns)]

use log::info;

#[path = "../boilerplate.rs"]
mod boilerplate;
mod game;
mod net;

/// Vangers game prototype
#[derive(clap::Parser)]
struct Cli {
    /// Connect to a multiplayer server (address:port)
    #[arg(long)]
    server: Option<String>,

    /// Player name for multiplayer
    #[arg(long, default_value = "Player")]
    name: String,
}

fn main() {
    use clap::Parser as _;
    let cli = Cli::parse();

    let (harness, settings) =
        boilerplate::Harness::init(boilerplate::HarnessOptions { title: "road" });

    info!("Parsing command line");
    let game = game::Game::new(&settings, &harness.graphics_ctx, cli.server, cli.name);

    harness.main_loop(game);
}
