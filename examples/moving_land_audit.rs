//! Loads a world's moving land and location engines and reports what came
//! out, so the readers can be checked against real game data.
//!
//! ```text
//! cargo run --example moving_land_audit -- path/to/world.ini
//! ```

use vangers::level::{LevelConfig, moving::MovingLand, trigger::Triggers, vlc};

use std::{collections::BTreeMap, path::PathBuf};

fn main() {
    env_logger::init();
    let mut args = std::env::args().skip(1);
    let ini_path = PathBuf::from(args.next().expect("usage: moving_land_audit <world.ini>"));
    let world_dir = ini_path.parent().expect("no parent dir").to_path_buf();
    let data_vot = world_dir.join("data.vot");

    let config = LevelConfig::load(&ini_path);
    let terrain_count = config.terrains.len() as i32;
    println!(
        "world {:?}  size {}x{}  terrains {}",
        world_dir,
        config.size.0.as_value(),
        config.size.1.as_value(),
        terrain_count
    );

    let land = MovingLand::load_dir(&data_vot, terrain_count);
    let triggers = Triggers::load(&world_dir, &data_vot, &land);
    let clones = vlc::load_clone_markers(&data_vot);

    println!("\n== moving land: {} locations ==", land.locations.len());
    let mut modes = BTreeMap::new();
    let mut total_frames = 0;
    let mut total_texels = 0usize;
    let mut periods = BTreeMap::new();
    let mut with_terrain_plane = 0;
    for location in land.locations.iter() {
        let ml = &location.source;
        *modes.entry(format!("{:?}", ml.mode)).or_insert(0) += 1;
        total_frames += ml.frames.len();
        for frame in ml.frames.iter() {
            total_texels += frame.area();
            *periods.entry(frame.period).or_insert(0usize) += 1;
            if !frame.terrain.is_empty() {
                with_terrain_plane += 1;
            }
        }
    }
    println!("  modes: {modes:?}");
    println!("  frames: {total_frames}, texels: {total_texels}");
    println!("  periods: {periods:?}");
    println!("  frames carrying a terrain plane: {with_terrain_plane}");

    // Names have to be unique for `location.lst` to address them.
    let mut by_name = BTreeMap::new();
    for location in land.locations.iter() {
        *by_name.entry(location.source.name.clone()).or_insert(0) += 1;
    }
    let dupes = by_name.iter().filter(|&(_, &n)| n > 1).collect::<Vec<_>>();
    if dupes.is_empty() {
        println!("  all {} names are unique", by_name.len());
    } else {
        println!("  DUPLICATE names: {dupes:?}");
    }

    println!("\n== locations ==");
    for (index, location) in land.locations.iter().enumerate() {
        let ml = &location.source;
        let first = &ml.frames[0];
        let (w, h) = ml.max_frame_size();
        println!(
            "  [{index:2}] {:16} {:?} at {:?} up to {}x{}  frames {:3}  stages {:3}  keys {:?}",
            ml.name,
            ml.mode,
            first.pos,
            w,
            h,
            ml.frames.len(),
            ml.max_stage(),
            ml.key_phases,
        );
    }

    println!("\n== sensors: {} ==", triggers.sensors.len());
    let mut kinds = BTreeMap::new();
    for sensor in triggers.sensors.iter() {
        *kinds.entry(sensor.kind).or_insert(0) += 1;
    }
    println!("  kinds: {kinds:?}");
    let unnamed = triggers
        .sensors
        .iter()
        .filter(|s| s.name.is_empty())
        .count();
    println!("  unnamed: {unnamed}");
    for (index, sensor) in triggers.sensors.iter().enumerate().take(40) {
        println!(
            "  [{index:2}] {:20} kind {:3} r {:3} at {:?} z {:?}",
            sensor.name, sensor.kind, sensor.radius, sensor.pos, sensor.z_range
        );
    }

    println!("\n== clone markers: {} ==", clones.len());
    for marker in clones.iter().take(8) {
        println!("  {:?} -> source {}", marker.pos, marker.source);
    }

    println!("\n== engines: {} ==", triggers.engines.len());
    let mut engine_kinds = BTreeMap::new();
    let mut unlinked = 0;
    let mut driven = 0;
    for engine in triggers.engines.iter() {
        let name = match engine.kind {
            vangers::level::trigger::Kind::Door { .. } => "Door",
            vangers::level::trigger::Kind::Tiristor { .. } => "Tiristor",
            vangers::level::trigger::Kind::Cyclic { .. } => "Cyclic",
            vangers::level::trigger::Kind::Unsupported(t) => {
                *engine_kinds.entry(format!("Unsupported({t})")).or_insert(0) += 1;
                continue;
            }
        };
        *engine_kinds.entry(name.to_string()).or_insert(0) += 1;
        match engine.location {
            Some(_) => driven += 1,
            None => unlinked += 1,
        }
    }
    println!("  kinds: {engine_kinds:?}");
    println!("  driving a location: {driven}, unlinked: {unlinked}");

    // Key phases have to land inside the location's stage range, or the
    // engine sends it somewhere it can never reach and it runs forever.
    println!("\n== phase sanity ==");
    let mut bad = 0;
    for engine in triggers.engines.iter() {
        let Some(index) = engine.location else {
            continue;
        };
        let location = &land.locations[index];
        let max_stage = location.source.max_stage();
        for (label, key) in [
            ("active", engine.active_phase),
            ("deactive", engine.deactive_phase),
        ] {
            let Some(&phase) = location.source.key_phases.get(key.max(0) as usize) else {
                continue;
            };
            if key >= 0 && phase >= max_stage {
                println!(
                    "  {} {} phase: key {} -> stage {} but max_stage is {}",
                    location.source.name, label, key, phase, max_stage
                );
                bad += 1;
            }
        }
    }
    if bad == 0 {
        println!("  every engine's key phases are inside their location's range");
    }

    println!("\n== unowned locations ==");
    let mut owned = vec![false; land.locations.len()];
    for engine in triggers.engines.iter() {
        if let Some(index) = engine.location {
            owned[index] = true;
        }
    }
    let free = owned.iter().filter(|&&o| !o).count();
    println!("  {free} of {} have no engine", land.locations.len());

    // Whether the authored deltas of a relative animation cancel out over a
    // full loop. If they do not, the surface cannot come back to where it
    // started no matter how faithful the playback is.
    println!("\n== authored delta balance ==");
    for location in land.locations.iter() {
        let ml = &location.source;
        if !ml.mode.is_relative() {
            continue;
        }
        let mut sums: BTreeMap<(i32, i32), i64> = BTreeMap::new();
        for f in ml.frames.iter() {
            for j in 0..f.size.1 {
                for i in 0..f.size.0 {
                    let local = (j * f.size.0 + i) as usize;
                    let d = f.delta[local] as i64;
                    if d == 0 {
                        continue;
                    }
                    let signed = if f.is_negative(local) { -d } else { d };
                    *sums.entry((f.pos.0 + i, f.pos.1 + j)).or_insert(0) += signed;
                }
            }
        }
        let unbalanced = sums.values().filter(|&&v| v != 0).count();
        if unbalanced != 0 {
            let extreme = sums.values().map(|v| v.abs()).max().unwrap_or(0);
            println!(
                "  {:16} {unbalanced:6} of {:6} touched texels do not cancel (largest |sum| {extreme})",
                ml.name,
                sums.len()
            );
        }
    }

    // Run every location against the real level, which is the only way to
    // catch a frame that reaches outside the map or an accumulator that runs
    // away.
    println!("\n== animation ==");
    let mut level = vangers::level::load(&config, &vangers::config::settings::Geometry::default());
    let baseline = level.height.clone();
    let mut land = land;
    triggers.reset_locations(&mut land);

    let mut worst_run = 0;
    let mut not_closed = Vec::new();
    for location in land.locations.iter_mut() {
        let name = location.source.name.clone();
        let max_stage = location.source.max_stage();
        location.set_go_phase(vangers::level::moving::FREE_RUNNING);

        let mut regions = Vec::new();
        let mut touched = 0usize;
        // Two full loops. A location whose authored deltas cancel and whose
        // altitudes never hit a clamp comes back exactly where it started.
        // Drift is therefore expected for two kinds of location, and neither
        // is a playback fault: ones whose deltas do not cancel (see the
        // balance report above), and one-shot animations that the game parks
        // at a key phase rather than looping - those often drive a texel
        // below zero on the way, and the clamp there is lossy in the
        // original too.
        for _ in 0..2 * max_stage {
            regions.clear();
            location.update(&mut level, &mut regions);
            touched += regions.len();
            for r in regions.iter() {
                assert!(
                    r.x >= 0 && r.y >= 0 && r.x + r.w <= level.size.0 && r.y + r.h <= level.size.1,
                    "{name}: region {r:?} outside the {}x{} level",
                    level.size.0,
                    level.size.1
                );
            }
        }
        worst_run = worst_run.max(touched);

        let drift = baseline
            .iter()
            .zip(level.height.iter())
            .filter(|(a, b)| a != b)
            .count();
        if drift != 0 {
            // The original clamps altitudes at both ends, so a delta that
            // would push a texel past 0 or 255 loses the excess and the
            // reverse delta cannot restore it. Report whether that is what
            // happened, or whether the drift is unexplained.
            let mut clamped = 0;
            let mut sample = Vec::new();
            for (i, (&was, &now)) in baseline.iter().zip(level.height.iter()).enumerate() {
                if was == now {
                    continue;
                }
                if now == 0 || now == 255 || was == 0 || was == 255 {
                    clamped += 1;
                }
                if sample.len() < 6 {
                    let (x, y) = (i as i32 % level.size.0, i as i32 / level.size.0);
                    sample.push(format!("({x},{y}) {was}->{now}"));
                }
            }
            not_closed.push((name, drift, clamped, sample));
            // Put the surface back so the next location starts clean.
            level.height.copy_from_slice(&baseline);
        }
    }
    println!("  ran every location for two full loops, max {worst_run} regions each");
    if not_closed.is_empty() {
        println!("  every location returns to the original surface after a full loop");
    } else {
        println!("  locations that do not return to the original surface:");
        for (name, drift, clamped, sample) in not_closed.iter() {
            println!("    {name}: {drift} texels off, {clamped} of them at an altitude clamp");
            println!("      {}", sample.join(", "));
        }
    }
}
