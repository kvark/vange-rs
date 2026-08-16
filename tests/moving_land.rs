//! End-to-end check of the moving land: real `*.vot`, `*.vlc` and
//! `location.lst` files on disk, driven the way the game drives them.

use vangers::config::settings;
use vangers::level::{Level, LevelConfig, moving::MovingLand, trigger::Triggers};

use std::path::{Path, PathBuf};

const SIZE: i32 = 64;
/// The strip the test bridge covers.
const BRIDGE: (i32, i32, i32) = (20, 30, 3);

fn level() -> Level {
    let total = (SIZE * SIZE) as usize;
    Level {
        size: (SIZE, SIZE),
        flood_map: vec![0; SIZE as usize].into_boxed_slice(),
        height: vec![10u8; total].into_boxed_slice(),
        meta: vec![0u8; total].into_boxed_slice(),
        palette: [[0; 4]; 0x100],
        terrains: LevelConfig::new_test().terrains,
        geometry: settings::Geometry::default(),
    }
}

fn strip(level: &Level, x0: i32, y: i32, w: i32) -> Vec<u8> {
    (0..w)
        .map(|i| level.height[(y * SIZE + x0 + i) as usize])
        .collect()
}

fn write_i32(data: &mut Vec<u8>, value: i32) {
    data.extend_from_slice(&value.to_le_bytes());
}

/// Four absolute frames raising a strip, with key phase 1 at the far end.
fn bridge_vot() -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(b"ML3");
    let mut name = [0u8; 16];
    name[.."bridge".len()].copy_from_slice(b"bridge");
    data.extend_from_slice(&name);
    for value in [4 /* frames */, 3 /* dry */, 0 /* impulse */] {
        write_i32(&mut data, value);
    }
    data.extend_from_slice(&[0, 1 /* Absolute */, 0, 0]);
    for value in [3, 0, -1] {
        write_i32(&mut data, value); // KeyPhase[1..3]
    }
    write_i32(&mut data, 0);

    for height in [60u8, 80, 100, 120] {
        for value in [
            BRIDGE.0, BRIDGE.1, BRIDGE.2, 1, // x0, y0, sx, sy
            1, -1, // period, surfType
            0, 0, // csd, cst - stored raw
            0, 0, // reserved
        ] {
            write_i32(&mut data, value);
        }
        data.extend(std::iter::repeat_n(height, BRIDGE.2 as usize));
    }
    data
}

/// One sensor named `pad`, sitting on the bridge.
fn sensor_table() -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(b"VLS1");
    write_i32(&mut data, 1);
    for value in [25, 30, 100] {
        write_i32(&mut data, value); // position
    }
    write_i32(&mut data, 1); // SensorType::SENSOR
    write_i32(&mut data, 8); // radius
    write_i32(&mut data, 3);
    data.extend_from_slice(b"pad");
    write_i32(&mut data, 50); // z0
    for value in [0, 0, 0, 0] {
        write_i32(&mut data, value); // vData, Power
    }
    write_i32(&mut data, 150); // z1
    write_i32(&mut data, 0);
    write_i32(&mut data, 0);
    data
}

const LOCATION_LST: &str = "\
NumEngine 1
Part 0
EngineType 0
MLName bridge
ActivePhase 1
DeactivePhase 0
ActiveTime 0
DeactiveTime 0
SoundID 0
NumSensorLink 1
SensorName pad
LockFlag 0
Luck 0
";

struct World {
    dir: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("vange-rs-world-{}-{}", tag, std::process::id()));
        let data_vot = dir.join("data.vot");
        std::fs::create_dir_all(&data_vot).unwrap();
        std::fs::write(data_vot.join("bridge.vot"), bridge_vot()).unwrap();
        std::fs::write(data_vot.join("snstable.vlc"), sensor_table()).unwrap();
        std::fs::write(dir.join("location.lst"), LOCATION_LST).unwrap();
        World { dir }
    }

    fn data_vot(&self) -> PathBuf {
        self.dir.join("data.vot")
    }

    fn load(&self) -> (MovingLand, Triggers) {
        let mut land = MovingLand::load_dir(&self.data_vot(), 8);
        let triggers = Triggers::load(&self.dir, &self.data_vot(), &land);
        triggers.reset_locations(&mut land);
        triggers.free_unowned(&mut land);
        (land, triggers)
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// One game quant: sensors, then engines, then the land itself.
fn quant(
    land: &mut MovingLand,
    triggers: &mut Triggers,
    level: &mut Level,
    driver: Option<(i32, i32, i32)>,
) -> Vec<vangers::level::moving::Region> {
    if let Some(pos) = driver {
        triggers.touch(pos, 3, (SIZE, SIZE));
    }
    triggers.update(land);
    let mut regions = Vec::new();
    land.update(level, &mut regions);
    regions
}

#[test]
fn a_door_opens_when_driven_onto_and_closes_after() {
    let world = World::new("door");
    let (mut land, mut triggers) = world.load();
    let mut level = level();

    assert_eq!(land.locations.len(), 1);
    assert_eq!(land.locations[0].source.name, "bridge");
    assert_eq!(triggers.engines.len(), 1);
    assert_eq!(triggers.sensors.len(), 1);
    assert_eq!(triggers.sensors[0].name, "pad");

    // Parked at the closed phase, so nothing moves on its own.
    for _ in 0..10 {
        let regions = quant(&mut land, &mut triggers, &mut level, None);
        assert!(regions.is_empty(), "a parked door must not animate");
    }
    assert_eq!(strip(&level, BRIDGE.0, BRIDGE.1, BRIDGE.2), [10, 10, 10]);

    // Drive onto the pad. The door runs to key phase 1 and stops there.
    let driver = Some((25, 30, 100));
    let mut touched_regions = 0;
    for _ in 0..10 {
        touched_regions += quant(&mut land, &mut triggers, &mut level, driver).len();
    }
    assert!(touched_regions > 0, "the door never moved");
    assert!(triggers.engines[0].is_open());
    assert_eq!(
        strip(&level, BRIDGE.0, BRIDGE.1, BRIDGE.2),
        [100, 100, 100],
        "stopped on the frame before key phase 1"
    );

    // It stays put while the driver waits on it.
    let settled = strip(&level, BRIDGE.0, BRIDGE.1, BRIDGE.2);
    for _ in 0..10 {
        quant(&mut land, &mut triggers, &mut level, driver);
    }
    assert_eq!(strip(&level, BRIDGE.0, BRIDGE.1, BRIDGE.2), settled);

    // Drive off: the door heads back to the closed phase.
    for _ in 0..10 {
        quant(&mut land, &mut triggers, &mut level, None);
    }
    assert!(!triggers.engines[0].is_open());
    assert_ne!(strip(&level, BRIDGE.0, BRIDGE.1, BRIDGE.2), settled);
}

#[test]
fn standing_next_to_the_sensor_does_nothing() {
    let world = World::new("miss");
    let (mut land, mut triggers) = world.load();
    let mut level = level();

    // Sensor radius 8 plus the car's 3, so 12 away is outside.
    for _ in 0..10 {
        let regions = quant(&mut land, &mut triggers, &mut level, Some((37, 30, 100)));
        assert!(regions.is_empty());
    }
    assert!(!triggers.engines[0].is_open());
}

#[test]
fn a_clone_animates_at_its_own_offset() {
    let world = World::new("clone");
    let (mut land, mut triggers) = world.load();
    let mut level = level();

    // Same bridge, 40 texels to the east, running free.
    let clone = land.add_clone(0, (40, 0));
    land.locations[clone].set_go_phase(vangers::level::moving::FREE_RUNNING);

    for _ in 0..4 {
        quant(&mut land, &mut triggers, &mut level, None);
    }

    // The original is parked; the clone has been stepping.
    assert_eq!(strip(&level, BRIDGE.0, BRIDGE.1, BRIDGE.2), [10, 10, 10]);
    assert_ne!(
        strip(&level, BRIDGE.0 + 40, BRIDGE.1, BRIDGE.2),
        [10, 10, 10],
        "the clone should have written at its offset"
    );
}

#[test]
fn a_level_without_moving_land_is_fine() {
    let dir = Path::new("/definitely/not/here");
    let land = MovingLand::load_dir(dir, 8);
    let triggers = Triggers::load(dir, dir, &land);
    assert!(land.is_empty());
    assert!(triggers.is_empty());
}
