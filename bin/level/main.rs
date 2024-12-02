mod app;
#[path = "../boilerplate.rs"]
mod boilerplate;

fn main() {
    use std::env;

    let (harness, settings) =
        boilerplate::Harness::init(boilerplate::HarnessOptions { title: "level" });

    let args: Vec<_> = env::args().collect();
    let mut options = getopts::Options::new();
    options
        .parsing_style(getopts::ParsingStyle::StopAtFirstFree)
        .optflag("h", "help", "print this help menu");

    let matches = options.parse(&args[1..]).unwrap();
    if matches.opt_present("h") || matches.free.len() > 1 {
        println!("Vangers level viewer");
        let brief = format!("Usage: {} [options] [<path_to_model>]", args[0]);
        println!("{}", options.usage(&brief));
        return;
    }

    let path = matches.free.first();
    let app = app::LevelView::new(path, &settings, &harness.graphics_ctx);

    harness.main_loop(app);
}
