#[path = "../boilerplate.rs"]
mod boilerplate;

#[cfg(target_arch = "wasm32")]
#[path = "../web.rs"]
mod web;

mod game;
mod physics;

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use std::env;
    env_logger::init();

    log::info!("Parsing command line");
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

    let (harness, settings) = boilerplate::Harness::init(boilerplate::HarnessOptions {
        title: "road",
        uses_level: true,
    });

    let game = game::Game::new(
        &settings,
        harness.color_format,
        harness.extent,
        &harness.device,
        &harness.queue,
        &harness.downlevel_caps,
    );

    harness.main_loop(game);
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use env_logger::Builder;
    use log::LevelFilter;

    console_error_panic_hook::set_once();

    Builder::new()
        .format(|_buf, record| {
            let message = format!("{}: {}", record.level(), record.args());
            web::log(&message);
            Ok(())
        })
        .filter(None, LevelFilter::Error)
        .init();

    web::create_fs();

    async fn run() {
        let (harness, settings) = boilerplate::Harness::init_async(boilerplate::HarnessOptions {
            title: "road",
            uses_level: true,
        })
        .await;

        let game = game::Game::new(
            &settings,
            harness.color_format,
            harness.extent,
            &harness.device,
            &harness.queue,
            &harness.downlevel_caps,
        );

        harness.main_loop(game);
    }

    wasm_bindgen_futures::spawn_local(run());
}
