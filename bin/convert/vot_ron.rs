//! Dumps a `*.vot` moving-land file as RON, for inspecting the animation
//! without running the game.

use serde_derive::Serialize;
use std::{fs::File, io::BufReader, path::Path};

#[derive(Serialize)]
struct FrameInfo {
    pos: (i32, i32),
    size: (i32, i32),
    period: i32,
    surface_type: i32,
    /// Texels the frame actually writes - the rest of the rectangle is zero.
    active_texels: usize,
    /// Largest altitude offset (or target altitude, in absolute mode).
    max_delta: u8,
    /// How many of the active texels move downwards.
    negative_texels: usize,
    /// Whether the frame carries a per-texel terrain plane.
    per_texel_terrain: bool,
}

#[derive(Serialize)]
struct LocationInfo {
    name: String,
    mode: String,
    dry_terrain: i32,
    impulse: i32,
    key_phases: [i32; vot::MAX_KEY_PHASE],
    /// Total quants in one loop.
    max_stage: i32,
    /// Extent of the largest frame.
    max_frame_size: (i32, i32),
    frames: Vec<FrameInfo>,
}

pub fn export(src_path: &Path, dst_path: &Path, terrain_count: i32) {
    let file = File::open(src_path).expect("Unable to open the VOT file");
    let ml = vot::MobileLocation::load(&mut BufReader::new(file), terrain_count)
        .unwrap_or_else(|e| panic!("Unable to parse {:?}: {}", src_path, e));

    let frames = ml
        .frames
        .iter()
        .map(|f| {
            let active = (0..f.area()).filter(|&i| f.delta[i] != 0);
            let (active_texels, negative_texels) = active.fold((0, 0), |(total, negative), i| {
                (total + 1, negative + f.is_negative(i) as usize)
            });
            FrameInfo {
                pos: f.pos,
                size: f.size,
                period: f.period,
                surface_type: f.surface_type,
                active_texels,
                max_delta: f.delta.iter().copied().max().unwrap_or(0),
                negative_texels,
                per_texel_terrain: !f.terrain.is_empty(),
            }
        })
        .collect();

    let info = LocationInfo {
        name: ml.name.clone(),
        mode: format!("{:?}", ml.mode),
        dry_terrain: ml.dry_terrain,
        impulse: ml.impulse,
        key_phases: ml.key_phases,
        max_stage: ml.max_stage(),
        max_frame_size: ml.max_frame_size(),
        frames,
    };

    println!(
        "\tGot '{}' in {} mode, {} frames",
        info.name,
        info.mode,
        info.frames.len()
    );
    let string = ron::ser::to_string_pretty(&info, ron::ser::PrettyConfig::default()).unwrap();
    std::fs::write(dst_path, string).unwrap();
}
