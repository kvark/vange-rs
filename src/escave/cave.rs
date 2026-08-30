//! Underground iscreen maps: the living thing's voxel interior.
//!
//! Original `location_data` points at `resource/iscreen/ldata/lN/escave.ini`.
//! The VMC is a 2048×1024 atlas of 800×600 screens (`I_RES_X`/`I_RES_Y` in
//! `iscreen.h`). `put_map(iScreenOffs, 0, I_RES_X, I_RES_Y)` copies one tile:
//! talk at `screen_offs 0` (`escave00.inc`), shop at `800` (`escave02.inc`).
//! We clip to that rectangle and orbit the camera around it.

use crate::config::settings;
use crate::level::{self, Level, LevelConfig, Power, Region, Texel};
use crate::space::Camera;
use crate::vfs::Vfs;
use glam::{Quat, Vec3};
use std::path::{Path, PathBuf};

/// Original `I_RES_X` / `I_RES_Y`.
pub const VIEW: (i32, i32) = (800, 600);

/// Which 800×600 tile of the iscreen atlas to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Situation {
    /// Counselor / location screen. `screen_offs 0`.
    Talk,
    /// Shop. `screen_offs 800`.
    Shop,
}

impl Situation {
    /// Original `iScreen::ScreenOffs`.
    pub fn screen_offs(self) -> i32 {
        match self {
            Situation::Talk => 0,
            Situation::Shop => 800,
        }
    }

    /// Atlas rectangle `put_map` copies onto the 800×600 framebuffer.
    pub fn region(self) -> Region {
        Region {
            x: self.screen_offs(),
            y: 0,
            w: VIEW.0,
            h: VIEW.1,
        }
    }
}

fn name_key(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Folder under the data path for this escave's iscreen map.
pub fn ldata_dir(name: &str) -> Option<&'static str> {
    Some(match name_key(name).as_str() {
        "podish" => "resource/iscreen/ldata/l0",
        "incubator" => "resource/iscreen/ldata/l1",
        "vigboo" => "resource/iscreen/ldata/l2",
        "lampasso" => "resource/iscreen/ldata/l3",
        "ogorod" => "resource/iscreen/ldata/l4",
        "zeepa" => "resource/iscreen/ldata/l5",
        "bzone" => "resource/iscreen/ldata/l6",
        "spobs" => "resource/iscreen/ldata/l7",
        _ => return None,
    })
}

/// A named pad from `escaves.prm` / `spots.prm`.
#[derive(Clone, Debug)]
pub struct Pad {
    pub name: String,
    pub pos: (i32, i32),
}

/// Scan original prm text for name + world-XY. Item lines and `none` are skipped.
pub fn pads_from_prm(text: &str) -> Vec<Pad> {
    let mut pads = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with("/*")
            || line.starts_with('*')
            || line.starts_with("uniVang")
        {
            continue;
        }
        let mut bits = line.split_whitespace();
        let Some(name) = bits.next() else {
            continue;
        };
        let Some(_world) = bits.next() else {
            continue;
        };
        let Some(x) = bits.next().and_then(|s| s.parse().ok()) else {
            continue;
        };
        let Some(y) = bits.next().and_then(|s| s.parse().ok()) else {
            continue;
        };
        if ldata_dir(name).is_some() {
            pads.push(Pad {
                name: name.to_string(),
                pos: (x, y),
            });
        }
    }
    pads
}

/// VLC sensors are `Escave1` / `Spot2`. Map them to Podish / VigBoo / … by XY.
pub fn resolve_name(sensor: &str, pos: (i32, i32), pads: &[Pad]) -> String {
    if ldata_dir(sensor).is_some() {
        return sensor.to_string();
    }
    const REACH2: i32 = 256 * 256;
    pads.iter()
        .filter(|p| {
            let dx = p.pos.0 - pos.0;
            let dy = p.pos.1 - pos.1;
            dx * dx + dy * dy <= REACH2
        })
        .min_by_key(|p| {
            let dx = p.pos.0 - pos.0;
            let dy = p.pos.1 - pos.1;
            dx * dx + dy * dy
        })
        .map(|p| p.name.clone())
        .unwrap_or_else(|| sensor.to_string())
}

pub fn ini_path(data_path: &Path, name: &str) -> Option<PathBuf> {
    Some(data_path.join(ldata_dir(name)?).join("escave.ini"))
}

/// VFS key of this escave's `escave.ini`, matching the packed level zip.
pub fn ini_key(name: &str) -> Option<String> {
    Some(format!("{}/escave.ini", ldata_dir(name)?))
}

/// Load the iscreen height map if the purchased data is on disk.
pub fn load(
    data_path: &Path,
    name: &str,
    geometry: &settings::Geometry,
    situation: Situation,
) -> Option<(LevelConfig, Level)> {
    let ini = ini_path(data_path, name)?;
    if !ini.is_file() {
        log::info!("escave iscreen missing at {}", ini.display());
        return None;
    }
    let mut config = LevelConfig::load(&ini);
    let mut level = level::load(&config, geometry);
    clip(&mut config, &mut level, situation);
    Some((config, level))
}

/// Same map, from a VFS (web packs `resource/iscreen/ldata/lN/` into the world zip).
pub fn load_from_vfs(
    vfs: &Vfs,
    name: &str,
    geometry: &settings::Geometry,
    situation: Situation,
) -> Option<(LevelConfig, Level)> {
    let ini = ini_key(name)?;
    if !vfs.contains(&ini) {
        log::info!("escave iscreen missing at {ini}");
        return None;
    }
    let mut config = LevelConfig::load_from_vfs(vfs, &ini);
    let mut level = level::load_from_vfs(vfs, &config, geometry);
    clip(&mut config, &mut level, situation);
    Some((config, level))
}

/// Keep only `situation`'s 800×600 tile, at the origin. Terrain textures are
/// a power of two, so the field is padded with void (height 0).
pub fn clip(config: &mut LevelConfig, level: &mut Level, situation: Situation) {
    let region = situation.region();
    let (src_w, src_h) = level.size;
    let dst_w = (region.w.max(1) as u32).next_power_of_two() as i32;
    let dst_h = (region.h.max(1) as u32).next_power_of_two() as i32;
    let mut height = vec![0u8; (dst_w * dst_h) as usize];
    let mut meta = vec![0u8; (dst_w * dst_h) as usize];
    let x0 = region.x.max(0);
    let y0 = region.y.max(0);
    let copy_w = (region.x + region.w).min(src_w) - x0;
    let copy_h = (region.y + region.h).min(src_h) - y0;
    if copy_w > 0 && copy_h > 0 {
        let cw = copy_w as usize;
        for y in 0..copy_h {
            let src = ((y0 + y) * src_w + x0) as usize;
            let dst = (y * dst_w) as usize;
            height[dst..dst + cw].copy_from_slice(&level.height[src..src + cw]);
            meta[dst..dst + cw].copy_from_slice(&level.meta[src..src + cw]);
        }
    }
    level.height = height.into_boxed_slice();
    level.meta = meta.into_boxed_slice();
    level.size = (dst_w, dst_h);
    config.size = (Power::from_value(dst_w), Power::from_value(dst_h));
}

fn sample_height(level: &Level, x: i32, y: i32) -> f32 {
    match level.get((x, y)) {
        Texel::Single(p) => p.0,
        Texel::Dual { high, .. } => high.0,
    }
}

/// Height-weighted centroid of non-void texels. After [`clip`] that is the
/// situation tile; the atlas centre is often a hole (height 0).
pub fn look_target(level: &Level) -> Vec3 {
    let (w, h) = level.size;
    let mut sx = 0.0;
    let mut sy = 0.0;
    let mut sz = 0.0;
    let mut wt = 0.0;
    for y in (0..h).step_by(4) {
        for x in (0..w).step_by(4) {
            let z = sample_height(level, x, y);
            if z > 0.0 {
                sx += x as f32 * z;
                sy += y as f32 * z;
                sz += z * z;
                wt += z;
            }
        }
    }
    if wt < 1.0 {
        let cx = w as f32 * 0.5;
        let cy = h as f32 * 0.5;
        return Vec3::new(cx, cy, sample_height(level, cx as i32, cy as i32) + 8.0);
    }
    Vec3::new(sx / wt, sy / wt, sz / wt + 8.0)
}

/// Stand back from the original 800×600 window, not the padded power-of-two.
pub fn look_distance(situation: Situation) -> f32 {
    let r = situation.region();
    (r.w.min(r.h) as f32 * 0.82).max(64.0)
}

/// Radians above the XY plane. 90° is straight down; this is 30° off vertical
/// so the living thing reads as a shape instead of a cliff face.
pub const ELEVATION: f32 = std::f32::consts::FRAC_PI_3;

pub fn orbit(cam: &mut Camera, target: Vec3, distance: f32, yaw: f32, elevation: f32) {
    let ce = elevation.cos();
    let se = elevation.sin();
    let offset = distance * Vec3::new(yaw.sin() * ce, -yaw.cos() * ce, se);
    cam.loc = target + offset;
    cam.rot = Camera::look_rotation(-offset);
}

pub fn camera(
    level: &Level,
    proj: crate::space::Projection,
    situation: Situation,
) -> (Camera, Vec3, f32) {
    let target = look_target(level);
    let distance = look_distance(situation);
    let mut cam = Camera {
        loc: target,
        rot: Quat::IDENTITY,
        scale: Vec3::new(1.0, -1.0, 1.0),
        proj,
    };
    orbit(&mut cam, target, distance, 0.35, ELEVATION);
    (cam, target, distance)
}

/// Cave interiors are a 2048×1024 height field. Mesh TIN chunks tile that
/// at 128 px and show as a grid; the height-field march does not.
pub fn render_settings(world: &settings::Render) -> settings::Render {
    let mut render = world.clone();
    render.terrain = settings::Terrain::RayTraced;
    render.light.shadow.terrain = settings::ShadowTerrain::RayTraced;
    render
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fostral_escaves_have_iscreen_folders() {
        assert_eq!(ldata_dir("Podish"), Some("resource/iscreen/ldata/l0"));
        assert_eq!(ldata_dir("Incubator"), Some("resource/iscreen/ldata/l1"));
        assert_eq!(ldata_dir("VigBoo"), Some("resource/iscreen/ldata/l2"));
        assert_eq!(ldata_dir("vig boo\0"), Some("resource/iscreen/ldata/l2"));
        assert!(ldata_dir("MysteryHole").is_none());
        assert!(ldata_dir("Escave1").is_none());
        let path = ini_path(Path::new("/data"), "Podish").unwrap();
        assert!(path.ends_with("resource/iscreen/ldata/l0/escave.ini"));
        assert_eq!(
            ini_key("Podish").as_deref(),
            Some("resource/iscreen/ldata/l0/escave.ini")
        );
        assert!(
            ((std::f32::consts::FRAC_PI_2 - ELEVATION).to_degrees() - 30.0).abs() < 0.05,
            "cave view should sit 30° off vertical, got {}°",
            (std::f32::consts::FRAC_PI_2 - ELEVATION).to_degrees()
        );
    }

    fn dummy_level(w: i32, h: i32, fill: u8) -> Level {
        let n = (w * h) as usize;
        Level {
            size: (w, h),
            flood_map: vec![0; 4].into_boxed_slice(),
            height: vec![fill; n].into_boxed_slice(),
            meta: vec![0; n].into_boxed_slice(),
            palette: [[0; 4]; 0x100],
            terrains: vec![level::TerrainConfig::default(); 8].into_boxed_slice(),
            geometry: settings::Geometry::default(),
        }
    }

    fn dummy_config(w: i32, h: i32) -> LevelConfig {
        LevelConfig {
            path_palette: PathBuf::new(),
            path_data: PathBuf::new(),
            is_compressed: false,
            size: (Power::from_value(w), Power::from_value(h)),
            geo: Power(0),
            section: Power(8),
            min_square: Power(0),
            terrains: vec![level::TerrainConfig::default(); 8].into_boxed_slice(),
            dynamic_palette: level::DynamicPalette::default(),
        }
    }

    #[test]
    fn original_windows_are_800x600_talk_then_shop() {
        let talk = Situation::Talk.region();
        assert_eq!((talk.x, talk.y, talk.w, talk.h), (0, 0, 800, 600));
        let shop = Situation::Shop.region();
        assert_eq!((shop.x, shop.y, shop.w, shop.h), (800, 0, 800, 600));
        assert_eq!(shop.x, talk.x + talk.w);
    }

    #[test]
    fn clip_extracts_the_shop_tile_to_origin() {
        let mut level = dummy_level(2048, 1024, 0);
        let mut config = dummy_config(2048, 1024);
        // Shop pixel (900, 100) and talk pixel (100, 100).
        level.height[(100 * 2048 + 900) as usize] = 80;
        level.height[(100 * 2048 + 100) as usize] = 80;
        clip(&mut config, &mut level, Situation::Shop);
        assert_eq!(level.size, (1024, 1024));
        assert_eq!(config.size.0.as_value(), 1024);
        assert_eq!(config.size.1.as_value(), 1024);
        // Shop (900, 100) lands at (100, 100); talk did not come along.
        assert_eq!(level.height[(100 * 1024 + 100) as usize], 80);
        assert_eq!(level.height[(100 * 1024) as usize], 0);
        let at = look_target(&level);
        assert!(
            (at.x - 100.0).abs() < 1.0 && (at.y - 100.0).abs() < 1.0,
            "camera should sit on the extracted shop pixel, got {at:?}"
        );
    }

    #[test]
    fn look_distance_frames_the_800x600_window() {
        let d = look_distance(Situation::Shop);
        assert!(
            d > 400.0 && d < 600.0,
            "camera should frame the 600-tall tile, got {d}"
        );
        assert!((d - look_distance(Situation::Talk)).abs() < f32::EPSILON);
    }

    #[test]
    fn vlc_escave1_on_glorx_is_vigboo() {
        let text = "\
uniVang-ParametersFile_Ver_1
Podish Fostral 1500 15879 LEEPURINGA
none
VigBoo Glorx 3 8797 BOORAWCHICK
none
Lampasso Glorx 1903 809 NOBOOL
none
Ogorod Glorx 1102 15109 PIPKA
none
";
        let pads = pads_from_prm(text);
        assert_eq!(resolve_name("Escave1", (21, 8790), &pads), "VigBoo");
        assert_eq!(resolve_name("Spot4", (1898, 799), &pads), "Lampasso");
        assert_eq!(resolve_name("Spot2", (1107, 15100), &pads), "Ogorod");
        assert_eq!(resolve_name("Podish", (0, 0), &pads), "Podish");
    }

    #[test]
    fn missing_iscreen_in_vfs_is_none() {
        let vfs = Vfs::new();
        assert!(
            load_from_vfs(
                &vfs,
                "Podish",
                &settings::Geometry::default(),
                Situation::Shop
            )
            .is_none()
        );
    }
}
