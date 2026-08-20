mod app;
#[path = "../boilerplate.rs"]
mod boilerplate;
mod headless;
mod moving_ui;

/// Vangers level viewer / snapshot benchmark
#[derive(clap::Parser)]
struct Cli {
    /// Optional path to the level world.ini (native filesystem load)
    path: Option<String>,
    /// Render to a PNG file and exit (headless). Path is the output PNG.
    #[arg(long)]
    snapshot: Option<String>,
    /// Terrain rendering mode: RayTraced, Sliced, Painted, RayVoxelTraced, Mesh
    #[arg(long, default_value = "RayTraced")]
    terrain: String,
    /// Forward samples for --terrain RayTraced. Larger values preserve more
    /// thin features on long, grazing rays.
    #[arg(long, default_value_t = 128)]
    ray_steps: u32,
    /// Mesh fit quality in 0..=1. Only meaningful with --terrain Mesh.
    #[arg(long, default_value_t = 0.75)]
    mesh_quality: f32,
    /// Overlay the mesh triangle edges. Only meaningful with --terrain Mesh.
    #[arg(long, default_value_t = false)]
    mesh_wireframe: bool,
    /// Write the frustum and the per-chunk cull/LOD decisions as JSON, for
    /// a top-down plot of what the renderer chose.
    #[arg(long)]
    cull_dump: Option<String>,
    /// Horizontal slices for --terrain Sliced. Defaults to the level height.
    #[arg(long)]
    slice_layers: Option<u32>,
    /// Sample density for --terrain Scattered, as `x,y,z`.
    #[arg(long, default_value = "2,2,2")]
    scatter_density: String,
    /// Pin the mesh to one LOD (0 = finest) instead of choosing by distance.
    #[arg(long)]
    mesh_lod: Option<usize>,
    /// Distance in texels at which the mesh drops to the next coarser LOD.
    #[arg(long)]
    mesh_lod_distance: Option<f32>,
    /// Draw every chunk of every wrap copy, skipping the frustum test.
    /// Slow, but it is the reference a culled render has to match.
    #[arg(long, default_value_t = false)]
    no_cull: bool,
    /// Carve a crater into the level after the first frame, exercising the
    /// incremental terrain-update path.
    #[arg(long, default_value_t = false)]
    dig: bool,
    /// Frame to dig on; 0 digs before the mesh is built (from-scratch
    /// reference), 1 exercises the incremental refit.
    #[arg(long, default_value_t = 1)]
    dig_frame: u32,
    /// Crater center as "x,y"; defaults to the middle of the level.
    #[arg(long)]
    dig_center: Option<String>,
    /// Crater radius in terrain texels.
    #[arg(long)]
    dig_radius: Option<i32>,
    /// Drive a wheel across the level after the first frame, so the tread
    /// it presses into the ground can be looked at.
    #[arg(long, default_value_t = false)]
    tracks: bool,
    /// Where the wheel starts and ends, as "x0,y0,x1,y1". Defaults to a
    /// line through the middle of the level.
    #[arg(long)]
    tracks_line: Option<String>,
    /// Passes to drive along that line. Each one cuts deeper.
    #[arg(long, default_value_t = 1)]
    tracks_passes: u32,
    /// Altitude units one stamp of the tread moves the ground. The game
    /// uses 1; a snapshot wants it exaggerated to be legible.
    #[arg(long, default_value_t = 8)]
    tracks_depth: i32,
    /// Drive the grader blade along the `--tracks-line` instead of a
    /// wheel, so the trench and the windrows it leaves can be looked at.
    #[arg(long, default_value_t = false)]
    grader: bool,
    /// How far below the surface the blade rides, in altitude units.
    #[arg(long, default_value_t = 30)]
    grader_depth: i32,
    /// Width of the blade in texels, across the line it travels.
    #[arg(long, default_value_t = 40.0)]
    grader_width: f32,
    /// Swell a lava spot at "x,y" and render it this many quants in, so
    /// the dome can be caught part-way out of the ground.
    #[arg(long)]
    lava: Option<String>,
    /// Quants of the lava spot to run before the snapshot.
    #[arg(long, default_value_t = 4)]
    lava_quants: u32,
    /// Bring the caves inside "x0,y0,x1,y1" down.
    #[arg(long)]
    landslide: Option<String>,
    /// Set the tide to this many days into the world's own cycle, so the
    /// water can be rendered at any point of its swing.
    #[arg(long)]
    tide_day: Option<f64>,
    /// Blow a crater at "x,y" - a rim thrown up around a bowl dug out,
    /// the shape the original's explosions leave.
    #[arg(long)]
    crater: Option<String>,
    /// Radius of that crater in texels.
    #[arg(long, default_value_t = 20)]
    crater_radius: i32,
    /// Render the world in one of its story cycles, by index. The cycles
    /// and their palettes come from `bunches.prm`; a world whose escave
    /// runs none simply ignores this.
    #[arg(long)]
    cycle: Option<usize>,
    /// Run the world's dynamic palette this many quants before the
    /// snapshot, so the animation can be looked at a frame at a time.
    #[arg(long, default_value_t = 0)]
    palette_quants: u32,
    /// Load the world's moving land (`data.vot`) and location engines
    /// (`location.lst`).
    #[arg(long, default_value_t = false)]
    moving_land: bool,
    /// Quants of moving land to run before the snapshot.
    #[arg(long, default_value_t = 0)]
    ml_quants: u32,
    /// Frame to run the moving land on; 0 is a from-scratch reference,
    /// 1 exercises the incremental terrain update.
    #[arg(long, default_value_t = 1)]
    ml_frame: u32,
    /// Stand an object at "x,y,z" so proximity engines fire.
    #[arg(long)]
    ml_touch: Option<String>,
    /// Release every location from its parked start so it animates without
    /// anything driving it.
    #[arg(long, default_value_t = false)]
    ml_free_run: bool,
    /// Run the moving land on every frame instead of once, so the frame
    /// timing includes the continuous terrain update.
    #[arg(long, default_value_t = false)]
    ml_continuous: bool,
    /// First-person camera: stand at "x,y" on the terrain instead of
    /// orbiting a target. Overrides --cam-target/--cam-distance/--cam-elev.
    #[arg(long)]
    fp: Option<String>,
    /// Horizontal distance to move in the viewing direction at fixed altitude.
    #[arg(long)]
    fp_travel: Option<f32>,
    /// Eye height above the local ground surface
    #[arg(long, default_value_t = 8.0)]
    fp_height: f32,
    /// First-person heading in degrees; 0 looks along +Y
    #[arg(long, default_value_t = 0.0)]
    fp_yaw: f32,
    /// First-person pitch in degrees; 0 is horizontal
    #[arg(long, default_value_t = 0.0)]
    fp_pitch: f32,
    /// Stand on the lower floor of a double-level region (inside the cave)
    /// rather than on its roof.
    #[arg(long, default_value_t = false)]
    fp_under: bool,
    /// Near clip distance. The 10-unit default clips the ground away from
    /// under a first-person camera; rasterized terrain then shows through.
    #[arg(long, default_value_t = 10.0)]
    near: f32,
    /// Far clip distance. The painter needs this bounded: it emits one
    /// instance per visible ground sample and clamps at a million.
    #[arg(long, default_value_t = 4000.0)]
    far: f32,
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
    /// Ray-march step budget for RayVoxelTraced, used for both the outer
    /// (octree) and inner (leaf) loops. Matches the web build's default.
    /// Below ~100 long sightlines exhaust it and solid terrain reads as sky.
    #[arg(long, default_value_t = 200)]
    voxel_steps: u32,
    /// Draw the voxel occupancy grid at this LOD instead of the terrain.
    #[arg(long)]
    voxel_debug_lod: Option<usize>,
    /// Probe the voxel occupancy grid at "x,y" against the height map.
    #[arg(long)]
    voxel_probe: Option<String>,
    /// Dump the depth buffer as raw f32 alongside the snapshot.
    #[arg(long)]
    depth_out: Option<String>,
    /// Save every timed color frame as a numbered PNG in this directory.
    #[arg(long)]
    frame_dir: Option<String>,
    /// Print the raw packed data and decoded surface for one texel pair.
    #[arg(long)]
    dump_texel: Option<String>,
}

fn parse_terrain(
    name: &str,
    voxel_size: [u32; 3],
    voxel_steps: u32,
    mesh_quality: f32,
    scatter_density: [u32; 3],
) -> vangers::config::settings::Terrain {
    use vangers::config::settings::Terrain;
    match name {
        "RayTraced" => Terrain::RayTraced,
        "Sliced" => Terrain::Sliced,
        // Matches the density in `settings.template.ron`.
        "Scattered" => Terrain::Scattered {
            density: scatter_density,
        },
        "Painted" => Terrain::Painted,
        // RayVoxelTraced uses the same parameters the web build hard-codes,
        // so the snapshot exercises the same path the user is benchmarking.
        "RayVoxelTraced" => Terrain::RayVoxelTraced {
            voxel_size,
            max_outer_steps: voxel_steps,
            max_inner_steps: voxel_steps,
            max_update_texels: 1_000_000,
        },
        "Mesh" => Terrain::Mesh {
            quality: mesh_quality,
        },
        other => panic!(
            "Unknown terrain mode '{}'. Supported: RayTraced, Sliced, Painted, \
             Scattered, RayVoxelTraced, Mesh",
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
                cli.voxel_steps,
                cli.mesh_quality,
                parse_voxel_size(&cli.scatter_density),
            ),
            ray_steps: cli.ray_steps,
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
            mesh_wireframe: cli.mesh_wireframe,
            no_cull: cli.no_cull,
            mesh_lod: cli.mesh_lod,
            slice_layers: cli.slice_layers,
            mesh_lod_distance: cli.mesh_lod_distance,
            dig: cli.dig,
            dig_frame: cli.dig_frame,
            dig_center: cli.dig_center.as_deref().map(|s| {
                let v = parse_vec3(&format!("{},0", s));
                (v.x as i32, v.y as i32)
            }),
            dig_radius: cli.dig_radius,
            moving_land: cli.moving_land,
            crater: cli.crater.as_deref().map(|s| {
                let v = s
                    .split(',')
                    .map(|t| t.trim().parse::<i32>().expect("--crater wants x,y"))
                    .collect::<Vec<_>>();
                assert_eq!(v.len(), 2, "--crater wants x,y");
                (v[0], v[1])
            }),
            lava: cli.lava.as_deref().map(|s| {
                let v = s
                    .split(',')
                    .map(|t| t.trim().parse::<i32>().expect("--lava wants x,y"))
                    .collect::<Vec<_>>();
                assert_eq!(v.len(), 2, "--lava wants x,y");
                (v[0], v[1])
            }),
            lava_quants: cli.lava_quants,
            landslide: cli.landslide.as_deref().map(|s| {
                let v = s
                    .split(',')
                    .map(|t| {
                        t.trim()
                            .parse::<i32>()
                            .expect("--landslide wants x0,y0,x1,y1")
                    })
                    .collect::<Vec<_>>();
                assert_eq!(v.len(), 4, "--landslide wants x0,y0,x1,y1");
                (v[0], v[1], v[2], v[3])
            }),
            tide_day: cli.tide_day,
            crater_radius: cli.crater_radius,
            cycle: cli.cycle,
            palette_quants: cli.palette_quants,
            tracks: cli.tracks,
            grader: cli.grader,
            grader_depth: cli.grader_depth,
            grader_width: cli.grader_width,
            tracks_line: cli.tracks_line.as_deref().map(|s| {
                let v = s
                    .split(',')
                    .map(|t| {
                        t.trim()
                            .parse::<i32>()
                            .expect("--tracks-line wants x0,y0,x1,y1")
                    })
                    .collect::<Vec<_>>();
                assert_eq!(v.len(), 4, "--tracks-line wants x0,y0,x1,y1");
                (v[0], v[1], v[2], v[3])
            }),
            tracks_passes: cli.tracks_passes,
            tracks_depth: cli.tracks_depth,
            ml_quants: cli.ml_quants,
            ml_frame: cli.ml_frame,
            ml_touch: cli.ml_touch.as_deref().map(|s| {
                let v = s
                    .split(',')
                    .map(|t| t.trim().parse::<i32>().expect("--ml-touch wants x,y,z"))
                    .collect::<Vec<_>>();
                assert_eq!(v.len(), 3, "--ml-touch wants x,y,z");
                (v[0], v[1], v[2])
            }),
            ml_free_run: cli.ml_free_run,
            ml_continuous: cli.ml_continuous,
            fp: cli.fp.as_deref().map(|s| {
                let v = parse_vec3(&format!("{},0", s));
                (v.x, v.y)
            }),
            fp_travel: cli.fp_travel,
            depth_out: cli.depth_out.clone(),
            frame_dir: cli.frame_dir.clone(),
            cull_dump: cli.cull_dump.clone(),
            dump_texel: cli.dump_texel.as_deref().map(|s| {
                let v = parse_vec3(&format!("{},0", s));
                (v.x as i32, v.y as i32)
            }),
            fp_height: cli.fp_height,
            fp_yaw: cli.fp_yaw,
            fp_pitch: cli.fp_pitch,
            fp_under: cli.fp_under,
            near: cli.near,
            far: cli.far,
            voxel_debug_lod: cli.voxel_debug_lod,
            voxel_probe: cli.voxel_probe.as_deref().map(|s| {
                let v = parse_vec3(&format!("{},0", s));
                (v.x as i32, v.y as i32)
            }),
        };
        headless::render_snapshot(opts);
        return;
    }

    let (harness, settings) =
        boilerplate::Harness::init(boilerplate::HarnessOptions { title: "level" });

    let app = app::LevelView::new(cli.path.as_deref(), &settings, &harness.graphics_ctx);

    harness.main_loop(app);
}
