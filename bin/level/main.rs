mod app;
#[path = "../boilerplate.rs"]
mod boilerplate;
mod headless;

/// Vangers level viewer
#[derive(clap::Parser)]
struct Cli {
    /// Optional path to the level
    path: Option<String>,
    /// Render a single frame to a PNG file and exit (headless)
    #[arg(long)]
    snapshot: Option<String>,
    /// Terrain rendering mode: RayTraced, Sliced, or Painted
    #[arg(long, default_value = "RayTraced")]
    terrain: String,
}

fn parse_terrain(name: &str) -> vangers::config::settings::Terrain {
    match name {
        "RayTraced" => vangers::config::settings::Terrain::RayTraced,
        "Sliced" => vangers::config::settings::Terrain::Sliced,
        "Painted" => vangers::config::settings::Terrain::Painted,
        other => panic!(
            "Unknown terrain mode '{}'. Supported: RayTraced, Sliced, Painted",
            other
        ),
    }
}

fn main() {
    use clap::Parser as _;
    let cli = Cli::parse();

    if let Some(ref snapshot_path) = cli.snapshot {
        env_logger::init();
        let terrain = parse_terrain(&cli.terrain);
        headless::render_snapshot(snapshot_path, cli.path.as_deref(), terrain);
        return;
    }

    let (harness, settings) =
        boilerplate::Harness::init(boilerplate::HarnessOptions { title: "level" });

    let app = app::LevelView::new(cli.path.as_deref(), &settings, &harness.graphics_ctx);

    harness.main_loop(app);
}
