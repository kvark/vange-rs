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
    level::{overlay::TrackOverlay, terraform},
    physics::{
        Dynamo, QuantClock,
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
