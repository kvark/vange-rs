//! Multiplayer clients predict their car over the same terrain the server
//! steps it on. The server never cuts tyre tracks (or smashes ceilings)
//! into its level; a client that did drove over different ground from the
//! next quantum on, the rear wheels over the front wheels' fresh track,
//! and the server corrected it, by 90u after a jump on Khox. Online, the
//! clients take the tracks with `terraform::drain_vehicle`, which leaves
//! the level alone.

#[path = "../bin/determ/trace.rs"]
#[allow(dead_code)]
mod trace;

use glam::{Quat, Vec3};
use vangers::{
    level::terraform,
    physics::{
        Dynamo, QuantClock,
        prediction::{fold_home, step_input},
    },
    space,
};

/// Quanta per client frame: the tracks are taken after every frame.
const FRAME: u32 = 3;

#[derive(Clone, Copy)]
enum Tracks {
    /// The server: nothing recorded.
    None,
    /// Offline play: cut into the level.
    Cut,
    /// Online play: recorded and dropped.
    Drained,
}

/// The scripted drive from `at`, each quantum's pose.
fn drive(world: &mut trace::World, at: (i32, i32), quanta: u32, mode: Tracks) -> Vec<trace::Pose> {
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
            match mode {
                Tracks::None => None,
                _ => Some(&mut tracks),
            },
        );
        fold_home(&mut t.disp, world.level.period());
        poses.push(trace::pose(&t, &d));
        if q % FRAME == FRAME - 1 {
            match mode {
                Tracks::None => {}
                Tracks::Cut => {
                    terraform::apply_vehicle(
                        &mut world.level,
                        &config,
                        &mut tracks,
                        8,
                        &mut regions,
                    );
                }
                Tracks::Drained => {
                    terraform::drain_vehicle(&mut tracks);
                }
            }
        }
    }
    poses
}

/// The first quantum the client left the server's bits, and the worst
/// position gap.
fn compare(name: &str, server: &[trace::Pose], client: &[trace::Pose]) -> (Option<usize>, f32) {
    let first = server.iter().zip(client).position(|(a, b)| a != b);
    let worst = server
        .iter()
        .zip(client)
        .map(|(a, b)| Vec3::new(a[0] - b[0], a[1] - b[1], a[2] - b[2]).length())
        .fold(0.0f32, f32::max);
    println!("{name}: first divergence {first:?}, worst gap {worst:.2}u");
    (first, worst)
}

fn check(name: &str, make: impl Fn() -> trace::World, at: (i32, i32), quanta: u32) -> f32 {
    let server = drive(&mut make(), at, quanta, Tracks::None);
    let level = make().level_hash();

    let mut world = make();
    let drained = drive(&mut world, at, quanta, Tracks::Drained);
    assert_eq!(
        world.level_hash(),
        level,
        "{name}: drained tracks edited the level"
    );
    let (first, _) = compare(&format!("{name}, drained"), &server, &drained);
    assert_eq!(first, None, "{name}: the online client left the server");

    let cut = drive(&mut make(), at, quanta, Tracks::Cut);
    compare(&format!("{name}, cut"), &server, &cut).1
}

#[test]
fn online_client_drives_over_the_servers_ground() {
    let worst = check("test level", trace::World::test, (100, 100), 600);
    // What the clients did before: their own tracks threw the car off.
    assert!(
        worst > 1.0,
        "cutting tracks was expected to move the car: {worst}"
    );
}

/// Khox, where the 90u correction was seen, from the jump of that run.
#[test]
fn online_client_drives_over_the_servers_ground_on_khox() {
    let dir = std::path::Path::new("docs/data-0");
    let (Ok(common), Ok(khox)) = (
        std::fs::read(dir.join("common.zip")),
        std::fs::read(dir.join("khox.zip")),
    ) else {
        eprintln!("skipping: no Khox data in docs/data-0");
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
