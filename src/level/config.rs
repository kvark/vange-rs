use ini::Ini;
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::vfs::Vfs;

#[derive(Copy, Clone)]
pub struct Power(pub i32);
impl Power {
    pub fn from_value(value: i32) -> Self {
        assert_eq!(value & (value - 1), 0);
        Power(value.trailing_zeros() as _)
    }
    pub fn as_value(&self) -> i32 {
        1 << self.0
    }
    pub fn as_power(&self) -> i32 {
        self.0
    }
}

#[derive(Clone, Default)]
pub struct TerrainConfig {
    pub shadow_offset: u8,
    pub height_shift: u8,
    pub colors: Range<u8>, // note: actually, this is inclusive range
}

pub struct LevelConfig {
    //pub name: String,
    pub path_palette: PathBuf,
    pub path_data: PathBuf,
    pub is_compressed: bool,
    pub size: (Power, Power),
    pub geo: Power,
    pub section: Power,
    pub min_square: Power,
    pub terrains: Box<[TerrainConfig]>,
}

impl LevelConfig {
    pub fn new_test() -> Self {
        // Each terrain type gets a 32-entry slice of the 256-color palette,
        // giving visible color variation across height zones.
        let terrains: Vec<TerrainConfig> = (0..8u8)
            .map(|i| TerrainConfig {
                shadow_offset: 0,
                height_shift: 0,
                colors: (i * 32)..(i * 32 + 31),
            })
            .collect();
        LevelConfig {
            path_palette: PathBuf::default(),
            path_data: PathBuf::default(),
            is_compressed: false,
            size: (Power(8), Power(8)), // 256x256
            geo: Power(0),
            section: Power(8),
            min_square: Power(0),
            terrains: terrains.into_boxed_slice(),
        }
    }

    pub fn load(ini_path: &Path) -> Self {
        let ini = Ini::load_from_file(ini_path).unwrap_or_else(|_| {
            panic!("Unable to read the level's INI description: {:?}", ini_path)
        });
        Self::from_ini(&ini, ini_path)
    }

    /// Load a level config from a VFS entry. `ini_path` is the VFS key
    /// of the world INI (e.g. `"fostral/world.ini"`); the resulting
    /// `path_data` and `path_palette` are VFS-relative keys too, suitable
    /// for passing back into `vfs.read(...)`.
    pub fn load_from_vfs(vfs: &Vfs, ini_path: &str) -> Self {
        let bytes = vfs
            .read(ini_path)
            .unwrap_or_else(|| panic!("INI not found in VFS: {}", ini_path));
        let text = std::str::from_utf8(&bytes)
            .unwrap_or_else(|e| panic!("INI {} is not valid UTF-8: {}", ini_path, e));
        let ini = Ini::load_from_str(text)
            .unwrap_or_else(|e| panic!("Failed to parse INI {}: {}", ini_path, e));
        Self::from_ini(&ini, Path::new(ini_path))
    }

    fn from_ini(ini: &Ini, ini_path: &Path) -> Self {
        let global = &ini["Global Parameters"];
        let storage = &ini["Storage"];
        let render = &ini["Rendering Parameters"];

        let terra_count = render
            .get("Terrain Max")
            .map_or(8, |value| value.parse::<usize>().unwrap());
        let mut terrains = (0..terra_count)
            .map(|_| TerrainConfig::default())
            .collect::<Box<[_]>>();

        for (t, val) in terrains
            .iter_mut()
            .zip(render["Shadow Offsets"].split_whitespace())
        {
            t.shadow_offset = val.parse().unwrap();
        }
        for (t, val) in terrains
            .iter_mut()
            .zip(render["Height Shifts"].split_whitespace())
        {
            t.height_shift = val.parse().unwrap();
        }
        for (t, val) in terrains
            .iter_mut()
            .zip(render["Begin Colors"].split_whitespace())
        {
            t.colors.start = val.parse().unwrap();
        }
        for (t, val) in terrains
            .iter_mut()
            .zip(render["End Colors"].split_whitespace())
        {
            t.colors.end = val.parse().unwrap();
        }

        let path_data = ini_path.with_file_name(&storage["File Name"]);
        LevelConfig {
            path_data,
            path_palette: ini_path.with_file_name(&storage["Palette File"]),
            is_compressed: &storage["Compressed Format Using"] != "0",
            //name: self.game.level.clone(),
            size: (
                Power(global["Map Power X"].parse().unwrap()),
                Power(global["Map Power Y"].parse().unwrap()),
            ),
            geo: Power(global["GeoNet Power"].parse().unwrap()),
            section: Power(global["Section Size Power"].parse().unwrap()),
            min_square: Power(global["Minimal Square Power"].parse().unwrap()),
            terrains,
        }
    }
}
