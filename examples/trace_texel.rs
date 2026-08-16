//! Traces one texel of one moving-land location through two full loops,
//! printing the authored per-frame delta and the resulting altitude.
//!
//! This is the tool for answering "why did this texel not come back":
//! usually an intermediate altitude clamp, which the original has too.
//!
//! ```text
//! cargo run --example trace_texel -- path/to/world.ini <location> <x> <y>
//! ```
use std::path::PathBuf;
use vangers::level::{LevelConfig, moving::MovingLand};

fn main() {
    let ini = PathBuf::from(std::env::args().nth(1).unwrap());
    let name = std::env::args().nth(2).unwrap();
    let tx: i32 = std::env::args().nth(3).unwrap().parse().unwrap();
    let ty: i32 = std::env::args().nth(4).unwrap().parse().unwrap();
    let dir = ini.parent().unwrap().to_path_buf();
    let config = LevelConfig::load(&ini);
    let mut level = vangers::level::load(&config, &vangers::config::settings::Geometry::default());
    let mut land = MovingLand::load_dir(&dir.join("data.vot"), config.terrains.len() as i32);
    let index = land.find(&name).unwrap();

    let idx = (ty * level.size.0 + tx) as usize;
    let ml = &land.locations[index].source;
    println!(
        "{} : {} frames, max_stage {}",
        name,
        ml.frames.len(),
        ml.max_stage()
    );
    println!(
        "meta[{tx},{ty}] = {:#04x} (double={})",
        level.meta[idx],
        level.meta[idx] & vangers::level::DOUBLE_LEVEL != 0
    );
    // Per-frame authored delta at this texel.
    for (fi, f) in ml.frames.iter().enumerate() {
        let (lx, ly) = (tx - f.pos.0, ty - f.pos.1);
        if lx < 0 || ly < 0 || lx >= f.size.0 || ly >= f.size.1 {
            println!("  frame {fi}: outside");
            continue;
        }
        let local = (ly * f.size.0 + lx) as usize;
        let d = f.delta[local] as i32;
        let signed = if f.is_negative(local) { -d } else { d };
        println!("  frame {fi}: period {} delta {:+}", f.period, signed);
    }

    let max_stage = ml.max_stage();
    land.locations[index].set_go_phase(vangers::level::moving::FREE_RUNNING);
    let mut regions = Vec::new();
    println!("start h = {}", level.height[idx]);
    for q in 1..=2 * max_stage {
        regions.clear();
        land.locations[index].update(&mut level, &mut regions);
        println!("  q{q:3}: h = {}", level.height[idx]);
    }
}
