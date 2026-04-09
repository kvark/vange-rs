mod app;
#[path = "../boilerplate.rs"]
mod boilerplate;

/// Vangers model viewer
#[derive(clap::Parser)]
struct Cli {
    /// Path to a model file (.m3d or .c3d)
    path: Option<String>,
}

fn main() {
    use clap::Parser as _;
    let cli = Cli::parse();

    let (harness, settings) =
        boilerplate::Harness::init(boilerplate::HarnessOptions { title: "model" });

    let app = app::ResourceView::new(cli.path.as_deref(), &settings, &harness.graphics_ctx);

    harness.main_loop(app);
}
