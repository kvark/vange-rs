mod app;
#[path = "../boilerplate.rs"]
mod boilerplate;

fn main() {
    use std::env;

    let (harness, settings) = boilerplate::Harness::init(boilerplate::HarnessOptions {
        title: "model",
        uses_level: false,
    });

    let args: Vec<_> = env::args().collect();
    let mut options = getopts::Options::new();
    options
        .parsing_style(getopts::ParsingStyle::StopAtFirstFree)
        .optflag("h", "help", "print this help menu");

    let matches = options.parse(&args[1..]).unwrap();
    if matches.opt_present("h") || matches.free.len() != 1 {
        println!("Vangers model viewer");
        let brief = format!("Usage: {} [options] <path_to_model>", args[0]);
        println!("{}", options.usage(&brief));
        return;
    }

    let path = &matches.free[0];
    let app = app::ResourceView::new(path, &settings, &harness.device, &harness.queue);

    harness.main_loop(app);
}
