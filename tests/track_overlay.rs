//! Online, the clients cut their tyre tracks into a cosmetic overlay
//! (`level::overlay`) rather than the level, which has to stay the
//! server's: the car is predicted over it. This drives the same script
//! with and without the overlay and checks the physics never notices,
//! bit for bit, while the overlay does end up holding the tracks.

#[path = "../bin/determ/trace.rs"]
#[allow(dead_code)]
mod trace;

use glam::{Quat, Vec3};
use vangers::{
    level::{
        overlay::{GhostTreads, TrackOverlay},
        terraform,
    },
    physics::{
        Dynamo, QuantClock, ServerCar,
        client::RemoteCar,
        prediction::{fold_home, step_input},
    },
    space,
};

/// Quanta per client frame: the tracks are taken after every frame.
const FRAME: u32 = 3;

struct Drive {
    poses: Vec<trace::Pose>,
    /// Most tiles the overlay held at once.
    peak_tiles: usize,
}

/// The scripted drive from `at`. With an overlay the tracks are recorded
/// and cut into it after every frame, as the online clients do; without,
/// nothing is recorded, as on the server.
fn drive(
    world: &mut trace::World,
    at: (i32, i32),
    quanta: u32,
    mut overlay: Option<&mut TrackOverlay>,
) -> Drive {
    let mut t = space::Transform {
        disp: Vec3::new(at.0 as f32, at.1 as f32, world.level.get(at).high() + 5.0),
        rot: Quat::IDENTITY,
        scale: world.car.scale,
    };
    let mut d = Dynamo::default();
    let mut tracks = terraform::Tracks::default();
    let mut regions = Vec::new();
    let config = terraform::Config::default();
    let factor = QuantClock::input_factor(world.max_quant, &world.common);
    let mut poses = Vec::new();
    let mut peak_tiles = 0;
    for q in 0..quanta {
        step_input(
            &mut d,
            &mut t,
            &trace::script(q),
            world.max_quant,
            factor,
            &world.car,
            &world.level,
            &world.common,
            overlay.as_ref().map(|_| &mut tracks),
        );
        fold_home(&mut t.disp, world.level.period());
        poses.push(trace::pose(&t, &d));
        if q % FRAME == FRAME - 1
            && let Some(overlay) = overlay.as_deref_mut()
        {
            overlay.cut_vehicle(&mut world.level, &config, &mut tracks, &mut regions);
            peak_tiles = peak_tiles.max(overlay.len());
        }
    }
    Drive { poses, peak_tiles }
}

/// Drives `quanta` from `at` both ways and checks the physics agree to the
/// bit and the overlay holds tracks. Returns the overlay.
fn check(name: &str, make: impl Fn() -> trace::World, at: (i32, i32), quanta: u32) -> TrackOverlay {
    let server = drive(&mut make(), at, quanta, None);
    let level = make().level_hash();

    let mut world = make();
    let mut overlay = TrackOverlay::default();
    let client = drive(&mut world, at, quanta, Some(&mut overlay));
    assert_eq!(
        world.level_hash(),
        level,
        "{name}: the overlay edited the level"
    );
    let first = server
        .poses
        .iter()
        .zip(&client.poses)
        .position(|(a, b)| a != b);
    assert_eq!(first, None, "{name}: the overlay moved the car");

    let changed = overlay.changed_texels(&world.level);
    println!(
        "{name}: {:.0}s of driving, {changed} track texels in {} tiles, {} KiB (peak {} tiles, {} KiB; cap {} KiB)",
        quanta as f32 / world.quanta_per_sec(),
        overlay.len(),
        overlay.bytes() / 1024,
        client.peak_tiles,
        client.peak_tiles * vangers::level::overlay::TILE_BYTES / 1024,
        vangers::level::overlay::MAX_BYTES / 1024,
    );
    assert!(changed > 0, "{name}: the overlay holds no tracks");
    overlay
}

#[test]
fn overlay_tracks_leave_the_physics_alone() {
    check("test level", trace::World::test, (100, 100), 600);
}

fn data(level: &str) -> Option<(Vec<u8>, Vec<u8>)> {
    let dir = std::path::Path::new("docs/data-0");
    match (
        std::fs::read(dir.join("common.zip")),
        std::fs::read(dir.join(format!("{level}.zip"))),
    ) {
        (Ok(common), Ok(level)) => Some((common, level)),
        _ => {
            eprintln!("skipping: no {level} data in docs/data-0");
            None
        }
    }
}

#[test]
fn overlay_tracks_leave_the_physics_alone_on_khox() {
    let Some((common, khox)) = data("khox") else {
        return;
    };
    for at in [(332, 341), (519, 283), (410, 195)] {
        check(
            &format!("Khox {at:?}"),
            || trace::World::from_zips(&[&common, &khox]),
            at,
            600,
        );
    }
}

#[test]
fn overlay_tracks_leave_the_physics_alone_on_fostral() {
    let Some((common, fostral)) = data("fostral") else {
        return;
    };
    check(
        "Fostral (830, 3983)",
        || trace::World::from_zips(&[&common, &fostral]),
        (830, 3983),
        600,
    );
}

/// Client frames per server snapshot, at 60 fps and the server's 20 Hz.
const FRAMES_PER_SNAPSHOT: u32 = 3;

/// A remote player's car as the client sees it: the server drives it (no
/// tracks recorded there) and the client gets a snapshot every tick, draws
/// it interpolated, and cuts its treads into the overlay from that drawn
/// pose (`GhostTreads`), as the native and web clients do. Meanwhile the
/// client's own car drives the same script from the same spot, over the
/// ghost's fresh treads and cutting its own. The client's car has to come
/// out bit for bit as the server stepped it, and the level untouched, while
/// the overlay holds the ghost's tracks.
fn check_ghost(name: &str, make: impl Fn() -> trace::World, at: (i32, i32), quanta: u32) {
    let start = |world: &trace::World| space::Transform {
        disp: Vec3::new(at.0 as f32, at.1 as f32, world.level.get(at).high() + 5.0),
        rot: Quat::IDENTITY,
        scale: world.car.scale,
    };

    // The server's run of the script: the remote car's snapshots, and the
    // reference for the client's own car.
    let server = make();
    let factor = QuantClock::input_factor(server.max_quant, &server.common);
    let per_tick = ((server.quanta_per_sec() / RemoteCar::SNAPSHOT_RATE).round() as u32).max(1);
    let mut t = start(&server);
    let mut d = Dynamo::default();
    let mut snapshots = Vec::new();
    let mut reference = Vec::new();
    for q in 0..quanta {
        step_input(
            &mut d,
            &mut t,
            &trace::script(q),
            server.max_quant,
            factor,
            &server.car,
            &server.level,
            &server.common,
            None,
        );
        fold_home(&mut t.disp, server.level.period());
        reference.push(trace::pose(&t, &d));
        if q % per_tick == per_tick - 1 {
            snapshots.push(t);
        }
    }

    let mut world = make();
    let level = world.level_hash();
    let config = terraform::Config::default();
    let mut overlay = TrackOverlay::default();
    let mut regions = Vec::new();
    let mut remote = RemoteCar::new(world.car.clone());
    let mut treads = GhostTreads::default();
    let wheels: Vec<Vec3> = world.car.wheels.iter().map(|w| Vec3::from(w.pos)).collect();
    let mut t = start(&world);
    let mut d = Dynamo::default();
    let mut own = terraform::Tracks::default();
    let mut poses = Vec::new();
    let mut ghost_stretches = 0;
    for (tick, snapshot) in snapshots.iter().enumerate() {
        for q in tick as u32 * per_tick..(tick as u32 + 1) * per_tick {
            step_input(
                &mut d,
                &mut t,
                &trace::script(q),
                world.max_quant,
                factor,
                &world.car,
                &world.level,
                &world.common,
                Some(&mut own),
            );
            fold_home(&mut t.disp, world.level.period());
            poses.push(trace::pose(&t, &d));
        }
        overlay.cut_vehicle(&mut world.level, &config, &mut own, &mut regions);
        remote.on_snapshot(
            &ServerCar {
                transform: *snapshot,
                linear_velocity: Vec3::ZERO,
                angular_velocity: Vec3::ZERO,
                traction: 0.0,
                rudder: 0.0,
                bumped: false,
            },
            Some(world.level.period()),
        );
        let dt = 1.0 / (RemoteCar::SNAPSHOT_RATE * FRAMES_PER_SNAPSHOT as f32);
        for _ in 0..FRAMES_PER_SNAPSHOT {
            remote.interpolate(dt);
            if treads.record(
                &world.level,
                &remote.render_transform,
                wheels.iter().copied(),
                dt,
            ) {
                ghost_stretches += overlay
                    .cut_vehicle(&mut world.level, &config, treads.tracks_mut(), &mut regions)
                    .len();
            }
        }
    }

    assert_eq!(
        world.level_hash(),
        level,
        "{name}: the ghost's treads edited the level"
    );
    let first = reference.iter().zip(&poses).position(|(a, b)| a != b);
    assert_eq!(
        first, None,
        "{name}: the ghost's treads moved the client's car"
    );
    let changed = overlay.changed_texels(&world.level);
    println!(
        "{name}: {} snapshots, {ghost_stretches} ghost wheel stretches, {changed} track texels in {} tiles",
        snapshots.len(),
        overlay.len(),
    );
    assert!(ghost_stretches > 0, "{name}: the ghost cut no treads");
    assert!(changed > 0, "{name}: the overlay holds no tracks");
}

#[test]
fn ghost_tracks_leave_the_physics_alone() {
    check_ghost("test level", trace::World::test, (100, 100), 600);
}

#[test]
fn ghost_tracks_leave_the_physics_alone_on_khox() {
    let Some((common, khox)) = data("khox") else {
        return;
    };
    for at in [(332, 341), (278, 278)] {
        check_ghost(
            &format!("Khox {at:?}"),
            || trace::World::from_zips(&[&common, &khox]),
            at,
            600,
        );
    }
}

/// Memory over a long drive (ten minutes of play): how far below the cap
/// the overlay stays. `--ignored --nocapture` to see the numbers.
#[test]
#[ignore]
fn overlay_memory_over_a_long_drive() {
    for (name, at) in [("khox", (332, 341)), ("fostral", (830, 3983))] {
        let Some((common, zip)) = data(name) else {
            continue;
        };
        let make = || trace::World::from_zips(&[&common, &zip]);
        let quanta = (make().quanta_per_sec() * 600.0) as u32;
        let mut world = make();
        let mut overlay = TrackOverlay::default();
        let d = drive(&mut world, at, quanta, Some(&mut overlay));
        println!(
            "{name} {at:?}: 600s, {} track texels in {} tiles, {} KiB (peak {} tiles)",
            overlay.changed_texels(&world.level),
            overlay.len(),
            overlay.bytes() / 1024,
            d.peak_tiles,
        );
    }
}
