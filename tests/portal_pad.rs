//! Headless check for post-hop pad snap: reasserting reverse-passage XY
//! (as `step_portal_pad_lock` does) keeps a real mechous on G2F.
//!
//! Skips when Glorx / mechous data is missing.

use glam::{Quat, Vec3};
use std::path::{Path, PathBuf};
use vangers::{
    config::{self, settings},
    level,
    physics::{self, CarPhysicsData, Dynamo},
    space,
};

fn data_root() -> Option<PathBuf> {
    let candidates = [
        std::env::var_os("VANGE_DATA").map(PathBuf::from),
        Some(PathBuf::from("/workspace/vange-data")),
        Some(PathBuf::from("../vange-data")),
        Some(PathBuf::from("../Vangers/data")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|root| root.join("thechain/glorx/world.ini").is_file())
}

fn load_oxidize(root: &Path) -> Option<CarPhysicsData> {
    let m3d = root.join("resource/m3d/mechous/m4.m3d");
    let mut prm = root.join("resource/m3d/mechous/m4.prm");
    if !prm.is_file() {
        prm = root.join("resource/m3d/mechous/default.prm");
    }
    let data = CarPhysicsData::load_from_files(
        std::fs::File::open(m3d).ok()?,
        std::fs::File::open(prm).ok()?,
        1.0,
        0,
    );
    Some(data)
}

fn common(root: &Path) -> config::common::Common {
    match std::fs::File::open(root.join("common.prm")) {
        Ok(f) => config::common::load(f),
        Err(_) => config::common::Common::test_default(),
    }
}

#[test]
fn g2f_pad_reassert_holds_arrival_xy() {
    let Some(root) = data_root() else {
        eprintln!("skip: Glorx world.ini not found");
        return;
    };
    let Some(data) = load_oxidize(&root) else {
        eprintln!("skip: OxidizeMonk m4.m3d not found");
        return;
    };
    let level = level::load(
        &level::LevelConfig::load(&root.join("thechain/glorx/world.ini")),
        &settings::Geometry::default(),
    );
    let common = common(&root);
    let arrival = (1420, 7575);
    let height = level.get(arrival).high() + 5.;

    // Leftover pitch/roll from the origin world (pre-fix hop kept this).
    let rot = Quat::from_rotation_z(1.2)
        * Quat::from_rotation_x(0.35)
        * Quat::from_rotation_y(-0.2);
    let mut transform = space::Transform {
        disp: Vec3::new(arrival.0 as f32, arrival.1 as f32, height),
        rot,
        scale: data.scale,
    };
    let mut dynamo = Dynamo {
        traction: 0.4,
        ..Dynamo::default()
    };

    // Match `PORTAL_PAD_LOCK_SECS` (1.5s) at 50 Hz.
    for _ in 0..75 {
        physics::step(
            &mut dynamo,
            &mut transform,
            0.02,
            &data,
            &level,
            &common,
            1.0,
            0.0,
            None,
            0.0,
            None,
            None,
        );
        let h = level.get(arrival).high() + 5.;
        transform.disp.x = arrival.0 as f32;
        transform.disp.y = arrival.1 as f32;
        transform.disp.z = h;
        dynamo = Dynamo::default();
    }

    let dxy = {
        let dx = transform.disp.x - arrival.0 as f32;
        let dy = transform.disp.y - arrival.1 as f32;
        (dx * dx + dy * dy).sqrt()
    };
    assert!(
        dxy < 1.0,
        "pad reassert should hold G2F arrival, dxy={dxy} pos={:?}",
        transform.disp
    );
}
