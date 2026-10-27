//! Determinism check: one scripted solo tumble through the shared car
//! physics, printed as the bits of every quantum's pose. Built natively
//! and as wasm (see `determ.mjs`), the two records must match line for
//! line; the first line that differs is where the web build's prediction
//! starts to drift from the server.
//!
//! Native:
//!   determ test X Y QUANTA
//!   determ zips COMMON.zip LEVEL.zip X Y QUANTA
//!   determ files SETTINGS.ron WORLD.ini X Y QUANTA   (the server's loading)

#[path = "trace.rs"]
#[allow(dead_code)]
mod trace;

#[cfg(target_arch = "wasm32")]
mod web {
    use super::trace;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub fn tumble_test(x: i32, y: i32, quanta: u32) -> String {
        let world = trace::World::test();
        trace::render(&world, &trace::tumble(&world, (x, y), quanta))
    }

    #[wasm_bindgen]
    pub fn tumble_zips(common: &[u8], level: &[u8], x: i32, y: i32, quanta: u32) -> String {
        let world = trace::World::from_zips(&[common, level]);
        trace::render(&world, &trace::tumble(&world, (x, y), quanta))
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use vangers::{config, level};
    let args: Vec<String> = std::env::args().skip(1).collect();
    let num = |i: usize| -> i32 { args[i].parse().expect("a number") };
    let (world, rest) = match args.first().map(String::as_str) {
        Some("test") => (trace::World::test(), 1),
        Some("zips") => {
            let common = std::fs::read(&args[1]).expect("common zip");
            let level = std::fs::read(&args[2]).expect("level zip");
            (trace::World::from_zips(&[&common, &level]), 3)
        }
        Some("files") => {
            // The server's way: files under `data_path`.
            let settings = config::Settings::load(&args[1]);
            let level_config = level::LevelConfig::load(std::path::Path::new(&args[2]));
            let lvl = level::load(&level_config, &settings.game.geometry);
            let common = config::common::load(settings.open_relative("common.prm"));
            let reg = config::game::Registry::load(&settings);
            let cars = config::car::load_physics_registry(
                &settings.data_path,
                &reg,
                settings.game.physics.shape_sampling,
            );
            let car = cars[&settings.car.id].clone();
            let world = trace::World {
                level: lvl,
                common,
                car,
                max_quant: settings.game.physics.max_quant,
            };
            (world, 3)
        }
        _ => {
            eprintln!("usage: determ test|zips C L|files S I  X Y QUANTA");
            std::process::exit(2);
        }
    };
    let poses = trace::tumble(&world, (num(rest), num(rest + 1)), num(rest + 2) as u32);
    print!("{}", trace::render(&world, &poses));
}
