mod app;
#[path = "../boilerplate.rs"]
mod boilerplate;
mod headless;

/// Vangers level viewer / snapshot benchmark
#[derive(clap::Parser)]
struct Cli {
    /// Optional path to the level world.ini (native filesystem load)
    path: Option<String>,
    /// Render to a PNG file and exit (headless). Path is the output PNG.
    #[arg(long)]
    snapshot: Option<String>,
    /// Terrain rendering mode: RayTraced, Sliced, Painted, RayVoxelTraced
    #[arg(long, default_value = "RayTraced")]
    terrain: String,
    /// Path to a level zip archive (for VFS-based loading; matches web)
    #[arg(long)]
    level_zip: Option<String>,
    /// Path to common.zip (for VFS-based loading; matches web)
    #[arg(long)]
    common_zip: Option<String>,
    /// Render width in pixels
    #[arg(long, default_value_t = 800)]
    width: u32,
    /// Render height in pixels
    #[arg(long, default_value_t = 600)]
    height: u32,
    /// Camera target as "x,y,z" in level coords
    #[arg(long, default_value = "128,128,0")]
    cam_target: String,
    /// Camera distance from target
    #[arg(long, default_value_t = 300.0)]
    cam_distance: f32,
    /// Camera elevation in degrees from horizontal (0 = horizontal, 90 = top-down)
    #[arg(long, default_value_t = 60.0)]
    cam_elev: f32,
    /// Number of frames to render after warmup (last one is saved)
    #[arg(long, default_value_t = 1)]
    frames: u32,
    /// Number of warmup frames before timing starts
    #[arg(long, default_value_t = 0)]
    warmup: u32,
    /// Optional path to write the per-frame timing summary as JSON
    #[arg(long)]
    bench_out: Option<String>,
    /// Reuse the voxel grid for shadow casting (matches the WebGPU build).
    /// Only meaningful with --terrain RayVoxelTraced.
    #[arg(long, default_value_t = false)]
    shadow_voxel: bool,
    /// Use the height-field ray-traced shadow path. Mirrors what the
    /// WebGL2 fallback renders.
    #[arg(long, default_value_t = false)]
    shadow_ray: bool,
    /// Override the RayVoxelTraced voxel cell size as "X,Y,Z". Larger
    /// values make the grid coarser and the storage buffer smaller —
    /// useful when the target adapter (e.g. lavapipe) caps storage
    /// buffer bindings below the default.
    #[arg(long)]
    voxel_size: Option<String>,
}

fn parse_terrain(name: &str, voxel_size: [u32; 3]) -> vangers::config::settings::Terrain {
    use vangers::config::settings::Terrain;
    match name {
        "RayTraced" => Terrain::RayTraced,
        "Sliced" => Terrain::Sliced,
        "Painted" => Terrain::Painted,
        // RayVoxelTraced uses the same parameters the web build hard-codes,
        // so the snapshot exercises the same path the user is benchmarking.
        "RayVoxelTraced" => Terrain::RayVoxelTraced {
            voxel_size,
            max_outer_steps: 40,
            max_inner_steps: 40,
            max_update_texels: 1_000_000,
        },
        other => panic!(
            "Unknown terrain mode '{}'. Supported: RayTraced, Sliced, Painted, RayVoxelTraced",
            other
        ),
    }
}

fn parse_voxel_size(s: &str) -> [u32; 3] {
    let parts: Vec<u32> = s
        .split(',')
        .map(|p| {
            p.trim()
                .parse::<u32>()
                .unwrap_or_else(|_| panic!("Invalid number in voxel-size: {}", p))
        })
        .collect();
    if parts.len() != 3 {
        panic!(
            "Expected 3 comma-separated numbers for voxel-size, got {}: {}",
            parts.len(),
            s
        );
    }
    [parts[0], parts[1], parts[2]]
}

fn parse_vec3(s: &str) -> glam::Vec3 {
    let parts: Vec<f32> = s
        .split(',')
        .map(|p| {
            p.trim()
                .parse::<f32>()
                .unwrap_or_else(|_| panic!("Invalid number in vec3: {}", p))
        })
        .collect();
    if parts.len() != 3 {
        panic!(
            "Expected 3 comma-separated numbers, got {}: {}",
            parts.len(),
            s
        );
    }
    glam::Vec3::new(parts[0], parts[1], parts[2])
}

fn main() {
    use clap::Parser as _;
    let cli = Cli::parse();

    if let Some(ref snapshot_path) = cli.snapshot {
        env_logger::init();
        let opts = headless::SnapshotOptions {
            output_path: snapshot_path.clone(),
            level_zip: cli.level_zip.clone(),
            common_zip: cli.common_zip.clone(),
            level_path: cli.path.clone(),
            terrain: parse_terrain(
                &cli.terrain,
                cli.voxel_size
                    .as_deref()
                    .map(parse_voxel_size)
                    .unwrap_or([2, 4, 1]),
            ),
            width: cli.width,
            height: cli.height,
            cam_target: parse_vec3(&cli.cam_target),
            cam_distance: cli.cam_distance,
            cam_elev_deg: cli.cam_elev,
            frames: cli.frames,
            warmup: cli.warmup,
            bench_out: cli.bench_out.clone(),
            shadow_voxel: cli.shadow_voxel,
            shadow_ray: cli.shadow_ray,
        };
        headless::render_snapshot(opts);
        return;
    }

    let (harness, settings) =
        boilerplate::Harness::init(boilerplate::HarnessOptions { title: "level" });

    let app = app::LevelView::new(cli.path.as_deref(), &settings, &harness.graphics_ctx);

    harness.main_loop(app);
}
