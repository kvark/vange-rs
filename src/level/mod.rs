use byteorder::{LittleEndian as E, ReadBytesExt, WriteBytesExt};

use std::{
    fs::File,
    io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

mod config;

pub use self::config::{LevelConfig, Power, TerrainConfig};
use crate::config::settings;

pub type TerrainType = u8;

pub const DOUBLE_LEVEL: u8 = 1 << 6;
pub const DELTA_BITS: u8 = 2;
pub const DELTA_MASK: u8 = 0x3;

pub struct Level {
    pub size: (i32, i32),
    pub flood_map: Box<[u8]>,
    pub height: Box<[u8]>,
    pub meta: Box<[u8]>,
    pub palette: [[u8; 4]; 0x100],
    pub terrains: Box<[TerrainConfig]>,
    pub geometry: settings::Geometry,
}

#[derive(Copy, Clone)]
pub struct Point(pub f32, pub TerrainType);

#[derive(Copy, Clone)]
pub struct TerrainBits {
    pub shift: u8,
    pub mask: TerrainType,
}

impl TerrainBits {
    pub fn new(count: u8) -> Self {
        match count {
            8 => TerrainBits {
                shift: 3,
                mask: 0x7,
            },
            16 => TerrainBits {
                shift: 2,
                mask: 0xF,
            },
            other => panic!("Unexpected terrain count {}!", other),
        }
    }

    pub fn read(&self, meta: u8) -> TerrainType {
        (meta >> self.shift) & self.mask
    }

    pub fn write(&self, tt: TerrainType) -> u8 {
        tt << self.shift
    }
}

#[derive(Copy, Clone)]
pub enum Texel {
    Single(Point),
    Dual { low: Point, mid: f32, high: Point },
}

impl Texel {
    pub fn low(&self) -> f32 {
        match *self {
            Texel::Single(ref p) => p.0,
            Texel::Dual { ref low, .. } => low.0,
        }
    }

    pub fn high(&self) -> f32 {
        match *self {
            Texel::Single(ref p) => p.0,
            Texel::Dual { ref high, .. } => high.0,
        }
    }
}

impl Level {
    pub fn terrain_bits(&self) -> TerrainBits {
        TerrainBits::new(self.terrains.len() as u8)
    }

    fn get_mid_altitude(&self, low: u8, high: u8, delta: u8) -> u8 {
        let power = self.geometry.delta_power;
        //Note: this is subject to interpretation in the shaders
        low.saturating_add(delta << power).min(high)
    }

    pub fn get(&self, coord: (i32, i32)) -> Texel {
        let bits = self.terrain_bits();
        let i = (coord.1.rem_euclid(self.size.1) * self.size.0 + coord.0.rem_euclid(self.size.0))
            as usize;
        let meta = self.meta[i];
        let altitude_scale = self.geometry.height as f32 / 256.0;
        if meta & DOUBLE_LEVEL != 0 {
            let meta0 = self.meta[i & !1];
            let meta1 = self.meta[i | 1];
            let delta = ((meta0 & DELTA_MASK) << DELTA_BITS) | (meta1 & DELTA_MASK);
            Texel::Dual {
                low: Point(
                    self.height[i & !1] as f32 * altitude_scale,
                    bits.read(meta0),
                ),
                mid: {
                    let altitude =
                        self.get_mid_altitude(self.height[i & !1], self.height[i | 1], delta);
                    altitude as f32 * altitude_scale
                },
                high: Point(self.height[i | 1] as f32 * altitude_scale, bits.read(meta1)),
            }
        } else {
            Texel::Single(Point(
                self.height[i] as f32 * altitude_scale,
                bits.read(meta),
            ))
        }
    }

    /// A faster version of query that only returns the lowest level altitude.
    pub fn get_low_fast(&self, coord: (i32, i32)) -> f32 {
        assert!(coord.0 >= 0 && coord.1 >= 0);
        let mut i = ((coord.1 % self.size.1) * self.size.0 + (coord.0 % self.size.0)) as usize;
        if i & 1 == 1 && self.meta[i] & DOUBLE_LEVEL != 0 {
            i &= !1;
        }
        let altitude_scale = self.geometry.height as f32 / 256.0;
        self.height[i] as f32 * altitude_scale
    }

    pub fn export(&self) -> Vec<u8> {
        let mut data = vec![0; self.size.0 as usize * self.size.1 as usize * 4];
        for y in 0..self.size.1 {
            let base_y = (y * self.size.0) as usize * 4;
            for x in 0..self.size.0 {
                let base_x = base_y + x as usize * 4;
                let color = &mut data[base_x..base_x + 4];
                match self.get((x, y)) {
                    Texel::Single(Point(height, ty)) => {
                        color[0] = height as u8;
                        color[1] = height as u8;
                        color[2] = 0;
                        color[3] = ty | (ty << 4);
                    }
                    Texel::Dual {
                        low: Point(low, low_ty),
                        mid,
                        high: Point(high, high_ty),
                    } => {
                        color[0] = low as u8;
                        color[1] = high as u8;
                        color[2] = (mid - low) as u8;
                        color[3] = low_ty | (high_ty << 4);
                    }
                }
            }
        }
        data
    }

    pub fn draw_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(format!("Delta mask: {}", self.geometry.delta_mask));
        ui.horizontal(|ui| {
            for terrain_id in 0..8 {
                let mask = 1 << terrain_id;
                let mut checked = self.geometry.delta_mask & mask != 0;
                ui.add(egui::Checkbox::without_text(&mut checked));
                self.geometry.delta_mask &= !mask;
                if checked {
                    self.geometry.delta_mask |= mask;
                }
            }
        });
        ui.add(egui::Slider::new(&mut self.geometry.delta_power, 0..=4).text("Delta power"));
        ui.add(egui::Slider::new(&mut self.geometry.delta_const, 1..=15).text("Delta const"));
    }
}

#[allow(unused)]
fn print_palette(data: &[[u8; 4]], info: &str) {
    print!("Palette - {}:", info);
    for i in 0..3 {
        print!("\n\t");
        for j in 0..0x100 {
            print!("{:02X}", data[j][i]);
        }
    }
    println!();
}

pub fn read_palette(input: File, config: Option<&[TerrainConfig]>) -> [[u8; 4]; 0x100] {
    let mut file = BufReader::new(input);
    let mut data = [[0; 4]; 0x100];
    for p in data.iter_mut() {
        file.read_exact(&mut p[..3]).unwrap();
        //p[0] <<= 2; p[1] <<= 2; p[2] <<= 2;
    }
    //print_palette(&data, "read from file");
    if let Some(terrains) = config {
        // see `PalettePrepare` of the original
        data[0] = [0; 4];

        for tc in terrains {
            for c in &mut data[tc.colors.start as usize][..3] {
                *c >>= 1;
            }
        }

        for i in 0..16 {
            let mut value = [(i * 4) as u8; 4];
            value[3] = 0;
            data[224 + i] = value;
        }

        //print_palette(&data, "corrected");
    }
    // see `XGR_Screen::setpal` of the original
    for p in data.iter_mut() {
        p[0] <<= 2;
        p[1] <<= 2;
        p[2] <<= 2;
    }
    //print_palette(&data, "scale");
    //TODO: there is quite a bit of logic missing here,
    // see `GeneralTableOpen` and `PalettePrepare` of the original.
    data
}

pub fn load_flood(config: &LevelConfig) -> Box<[u8]> {
    profiling::scope!("Flood Map");
    let size = (config.size.0.as_value(), config.size.1.as_value());
    let flood_size = size.1 >> config.section.as_power();

    let vpr_file = match File::open(&config.path_data.with_extension("vpr")) {
        Ok(file) => file,
        Err(_) => return vec![0; flood_size as usize].into_boxed_slice(),
    };

    info!("Loading flood map...");
    let geo_pow = config.geo.as_power();
    let net_size = (size.0 * size.1) >> (2 * geo_pow);
    let flood_offset =
        (2 * 4 + (1 + 4 + 4) * 4 + 2 * net_size + 2 * geo_pow * 4 + 2 * flood_size * geo_pow * 4)
            as u64;
    let expected_file_size = flood_offset + (flood_size * 4) as u64;
    assert_eq!(vpr_file.metadata().unwrap().len(), expected_file_size,);
    let mut vpr = BufReader::new(vpr_file);
    vpr.seek(SeekFrom::Start(flood_offset)).unwrap();
    (0..flood_size)
        .map(|_| vpr.read_u32::<E>().unwrap() as u8)
        .collect()
}

pub struct LevelData {
    pub height: Box<[u8]>,
    pub meta: Box<[u8]>,
    pub size: (i32, i32),
}

impl From<Level> for LevelData {
    fn from(level: Level) -> Self {
        LevelData {
            height: level.height,
            meta: level.meta,
            size: (level.size.0, level.size.1),
        }
    }
}

impl LevelData {
    pub fn save_vmp(&self, path: &Path) {
        let mut vmp = BufWriter::new(File::create(path).unwrap());
        self.height
            .chunks(self.size.0 as _)
            .zip(self.meta.chunks(self.size.0 as _))
            .for_each(|(h_row, m_row)| {
                vmp.write_all(h_row).unwrap();
                vmp.write_all(m_row).unwrap();
            });
    }

    pub fn save_vmc(&self, path: &Path) {
        use splay::Splay;
        let mut vmc = BufWriter::new(File::create(path).unwrap());

        let base_offset = self.size.1 as u64 * (2 + 4) + Splay::tree_size();
        for i in 0..self.size.1 {
            vmc.write_i32::<E>(base_offset as i32 + i * self.size.0 * 2)
                .unwrap();
            vmc.write_i16::<E>(self.size.0 as i16 * 2).unwrap();
        }

        Splay::write_trivial(&mut vmc);
        assert_eq!(vmc.seek(SeekFrom::Current(0)).unwrap(), base_offset);

        self.height
            .chunks(self.size.0 as _)
            .zip(self.meta.chunks(self.size.0 as _))
            .for_each(|(h_row, m_row)| {
                Splay::compress_trivial(h_row, m_row, &mut vmc);
            });
    }
}

pub fn load_vmc(path: &Path, size: (i32, i32)) -> LevelData {
    use rayon::prelude::*;
    use splay::Splay;

    info!("Loading height map...");
    let total = (size.0 * size.1) as usize;
    let mut level = LevelData {
        height: vec![0u8; total].into_boxed_slice(),
        meta: vec![0u8; total].into_boxed_slice(),
        size,
    };

    let (splay, st_table, sz_table) = {
        profiling::scope!("Prepare");
        let mut vmc_base = BufReader::new(File::open(path).expect("Unable to open VMC"));

        info!("\tLoading compression tables...");
        let mut st_table = Vec::<i32>::with_capacity(size.1 as usize);
        let mut sz_table = Vec::<i16>::with_capacity(size.1 as usize);
        for _ in 0..size.1 {
            st_table.push(vmc_base.read_i32::<E>().unwrap());
            sz_table.push(vmc_base.read_i16::<E>().unwrap());
        }

        info!("\tDecompressing level data...");
        let splay = Splay::new(&mut vmc_base);
        (splay, st_table, sz_table)
    };

    level
        .height
        .chunks_mut(size.0 as _)
        .zip(level.meta.chunks_mut(size.0 as _))
        .zip(st_table.iter().zip(&sz_table))
        .collect::<Vec<_>>()
        .par_chunks_mut(64)
        .for_each(|source_group| {
            //Note: a separate file per group is required
            let mut vmc = File::open(path).unwrap();
            let data_size: i16 = source_group
                .iter()
                .map(|(_, (_, &size))| size)
                .max()
                .unwrap();
            let mut data = vec![0u8; data_size as usize];
            for &mut ((ref mut h_row, ref mut m_row), (offset, &size)) in source_group {
                vmc.seek(SeekFrom::Start(*offset as u64)).unwrap();
                vmc.read_exact(&mut data[..size as usize]).unwrap();
                splay.expand(&data[..size as usize], h_row, m_row);
            }
        });

    level
}

pub fn load_vmp(path: &Path, size: (i32, i32)) -> LevelData {
    let total = (size.0 * size.1) as usize;
    let mut level = LevelData {
        height: vec![0u8; total].into_boxed_slice(),
        meta: vec![0u8; total].into_boxed_slice(),
        size,
    };

    let mut vmp = BufReader::new(File::open(path).expect("Unable to open VMP"));
    level
        .height
        .chunks_mut(size.0 as _)
        .zip(level.meta.chunks_mut(size.0 as _))
        .for_each(|(h_row, m_row)| {
            vmp.read_exact(h_row).unwrap();
            vmp.read_exact(m_row).unwrap();
        });

    level
}

fn path_empty(path_buf: &PathBuf) -> bool {
    path_buf.as_path().to_str() == Some("")
}

pub fn load(config: &LevelConfig, geometry: &settings::Geometry) -> Level {
    profiling::scope!("Load Level");

    info!("Loading data map...");
    let size = (config.size.0.as_value(), config.size.1.as_value());
    let LevelData { height, meta, size } = if path_empty(&config.path_data) {
        let total = size.0 as usize * size.1 as usize;
        LevelData {
            height: vec![0; total].into_boxed_slice(),
            meta: vec![0; total].into_boxed_slice(),
            size,
        }
    } else if config.is_compressed {
        load_vmc(&config.path_data.with_extension("vmc"), size)
    } else {
        load_vmp(&config.path_data.with_extension("vmp"), size)
    };

    info!("Loading flood map...");
    let flood_map = if path_empty(&config.path_data) {
        let sections = size.1 as usize >> config.section.as_power();
        vec![0; sections].into_boxed_slice()
    } else {
        load_flood(config)
    };

    let palette = if path_empty(&config.path_palette) {
        [[0xFF; 4]; 0x100]
    } else {
        let file = File::open(&config.path_palette).expect("Unable to open the palette file");
        read_palette(file, Some(&config.terrains))
    };

    Level {
        size,
        flood_map,
        height,
        meta,
        palette,
        terrains: config.terrains.clone(),
        geometry: geometry.clone(),
    }
}
