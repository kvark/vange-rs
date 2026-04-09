mod app;
#[path = "../boilerplate.rs"]
mod boilerplate;

/// Vangers level viewer
#[derive(clap::Parser)]
struct Cli {
    /// Path to a level INI file
    path: Option<String>,
}

fn main() {
    use clap::Parser as _;
    let cli = Cli::parse();

    let (harness, settings) =
        boilerplate::Harness::init(boilerplate::HarnessOptions { title: "level" });

    let app = app::LevelView::new(cli.path.as_deref(), &settings, &harness.graphics_ctx);

    harness.main_loop(app);
}
