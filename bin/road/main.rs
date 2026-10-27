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

    /// Debug: drive the local car from a script of `seconds:motor:rudder`
    /// segments, comma separated (motor and rudder in -1..=1), e.g.
    /// `2:0:0,20:1:0.6`. Controls go through the normal input path.
    #[arg(long)]
    autodrive: Option<String>,
}

fn main() {
    use clap::Parser as _;
    let cli = Cli::parse();

    let (harness, settings) =
        boilerplate::Harness::init(boilerplate::HarnessOptions { title: "road" });

    info!("Parsing command line");
    let mut game = game::Game::new(&settings, &harness.graphics_ctx, cli.server, cli.name);
    if let Some(script) = cli.autodrive.as_deref() {
        game.set_autodrive(script);
    }

    harness.main_loop(game);
}
