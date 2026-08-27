//! Underground iscreen maps: the living thing's voxel interior.
//!
//! Original `location_data` points at `resource/iscreen/ldata/lN/escave.ini`.
//! The road renderer already knows how to voxelize a height map, so a visit
//! loads that INI as a second [`Level`] and draws it behind the shop.

use crate::config::settings;
use crate::level::{self, Level, LevelConfig, Texel};
use crate::space::Camera;
use glam::{Quat, Vec3};
use std::path::{Path, PathBuf};

/// Folder under the data path for this escave's iscreen map.
pub fn ldata_dir(name: &str) -> Option<&'static str> {
    Some(match name {
        "Podish" => "resource/iscreen/ldata/l0",
        "Incubator" => "resource/iscreen/ldata/l1",
        "VigBoo" => "resource/iscreen/ldata/l2",
        "Lampasso" => "resource/iscreen/ldata/l3",
        "Ogorod" => "resource/iscreen/ldata/l4",
        "ZeePa" => "resource/iscreen/ldata/l5",
        "B-Zone" | "BZone" => "resource/iscreen/ldata/l6",
        "Spobs" => "resource/iscreen/ldata/l7",
        _ => return None,
    })
}

pub fn ini_path(data_path: &Path, name: &str) -> Option<PathBuf> {
    Some(data_path.join(ldata_dir(name)?).join("escave.ini"))
}

/// Load the iscreen height map if the purchased data is on disk.
pub fn load(
    data_path: &Path,
    name: &str,
    geometry: &settings::Geometry,
) -> Option<(LevelConfig, Level)> {
    let ini = ini_path(data_path, name)?;
    if !ini.is_file() {
        log::info!("escave iscreen missing at {}", ini.display());
        return None;
    }
    let config = LevelConfig::load(&ini);
    let level = level::load(&config, geometry);
    Some((config, level))
}

pub fn look_target(level: &Level) -> Vec3 {
    let cx = level.size.0 as f32 * 0.5;
    let cy = level.size.1 as f32 * 0.5;
    let ground = match level.get((cx as i32, cy as i32)) {
        Texel::Single(p) => p.0,
        Texel::Dual { high, .. } => high.0,
    };
    Vec3::new(cx, cy, ground + 8.0)
}

pub fn look_distance(level: &Level) -> f32 {
    (level.size.0.min(level.size.1) as f32 * 0.42).max(64.0)
}

pub fn orbit(cam: &mut Camera, target: Vec3, distance: f32, yaw: f32, elevation: f32) {
    let ce = elevation.cos();
    let se = elevation.sin();
    let offset = distance * Vec3::new(yaw.sin() * ce, -yaw.cos() * ce, se);
    cam.loc = target + offset;
    cam.rot = Camera::look_rotation(-offset);
}

pub fn camera(level: &Level, proj: crate::space::Projection) -> (Camera, Vec3, f32) {
    let target = look_target(level);
    let distance = look_distance(level);
    let mut cam = Camera {
        loc: target,
        rot: Quat::IDENTITY,
        scale: Vec3::new(1.0, -1.0, 1.0),
        proj,
    };
    orbit(&mut cam, target, distance, 0.35, 0.55);
    (cam, target, distance)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fostral_escaves_have_iscreen_folders() {
        assert_eq!(ldata_dir("Podish"), Some("resource/iscreen/ldata/l0"));
        assert_eq!(ldata_dir("Incubator"), Some("resource/iscreen/ldata/l1"));
        assert!(ldata_dir("MysteryHole").is_none());
        let path = ini_path(Path::new("/data"), "Podish").unwrap();
        assert!(path.ends_with("resource/iscreen/ldata/l0/escave.ini"));
    }
}
