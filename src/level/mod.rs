use byteorder::{LittleEndian as E, ReadBytesExt};

pub const NUM_TERRAINS: usize = 8;

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TerrainType {
    Water,
    Main,
}

pub struct Power(pub i32);
impl Power {
    fn as_value(&self) -> i32 {
        1 << self.0
    }
    fn as_power(&self) -> i32 {
        self.0
    }
}

#[derive(Clone, Copy)]
pub struct TerrainConfig {
    pub shadow_offset: u8,
    pub height_shift: u8,
    pub color_range: (u8, u8),
}

pub struct LevelConfig {
    pub name: String,
    pub path_palette: String,
    pub path_vpr: String,
    pub path_vmc: String,
    pub is_compressed: bool,
    pub size: (Power, Power),
    pub geo: Power,
    pub section: Power,
    pub min_square: Power,
    pub terrains: [TerrainConfig; NUM_TERRAINS],
}

pub type Altitude = u8;
pub type Delta = u8;
const DOUBLE_LEVEL: u8 = 1 << 6;
const DELTA_BITS: u8 = 2;
const DELTA_MASK: u8 = (1 << DELTA_BITS) - 1;
const DELTA_SHIFT: u8 = 3;
pub const HEIGHT_SCALE: u32 = 48;

pub struct Level {
    pub size: (i32, i32),
    pub flood_map: Vec<u32>,
    pub height: Vec<u8>,
    pub meta: Vec<u8>,
    pub palette: [[u8; 4]; 0x100],
    pub terrains: [TerrainConfig; NUM_TERRAINS],
}

pub struct Texel {
    pub low: (Altitude, TerrainType),
    pub high: Option<(Delta, Altitude, TerrainType)>,
}

impl Texel {
    pub fn get_top(&self) -> Altitude {
        match self.high {
            Some((_, alt, _)) => alt,
            None => self.low.0,
        }
    }
}

impl Level {
    pub fn new_test() -> Level {
        let tc = TerrainConfig {
            shadow_offset: 0,
            height_shift: 0,
            color_range: (0, 1),
        };
        Level {
            size: (2, 1),
            flood_map: vec![0],
            height: vec![0, 0],
            meta: vec![0, 0],
            palette: [[0xFF; 4]; 0x100],
            terrains: [tc; NUM_TERRAINS],
        }
    }

    pub fn get(
        &self,
        mut coord: (i32, i32),
    ) -> Texel {
        fn get_terrain(meta: u8) -> TerrainType {
            match (meta >> DELTA_SHIFT) & (NUM_TERRAINS as u8 - 1) {
                0 => TerrainType::Water,
                _ => TerrainType::Main,
            }
        }
        while coord.0 < 0 {
            coord.0 += self.size.0;
        }
        while coord.1 < 0 {
            coord.1 += self.size.1;
        }
        let i = ((coord.1 % self.size.1) * self.size.0 + (coord.0 % self.size.0)) as usize;
        let meta = self.meta[i];
        if meta & DOUBLE_LEVEL != 0 {
            let meta0 = self.meta[i & !1];
            let meta1 = self.meta[i | 1];
            let delta = ((meta0 & DELTA_MASK) << DELTA_BITS + (meta1 & DELTA_MASK)) << DELTA_SHIFT;
            Texel {
                low: (self.height[i & !1], get_terrain(meta0)),
                high: Some((delta, self.height[i | 1], get_terrain(meta1))),
            }
        } else {
            Texel {
                low: (self.height[i], get_terrain(meta)),
                high: None,
            }
        }
    }
}

pub fn load_palette(path: &str) -> [[u8; 4]; 0x100] {
    use std::fs::File;
    use std::io::{BufReader, Read};

    info!("Loading palette {}...", path);
    let mut file = BufReader::new(File::open(path).unwrap());
    let mut data = [[0; 4]; 0x100];
    for p in data.iter_mut() {
        file.read(&mut p[.. 3]).unwrap();
    }
    data
}

pub fn load(config: &LevelConfig) -> Level {
    use std::fs::File;
    use std::io::{BufReader, Seek, SeekFrom};
    use std::time::Instant;
    use rayon::prelude::*;
    use splay::Splay;

    assert!(config.is_compressed);
    let size = (config.size.0.as_value(), config.size.1.as_value());

    info!("Loading vpr...");
    let start_vpr = Instant::now();
    let flood = {
        let vpr_file = File::open(&config.path_vpr).unwrap();
        let flood_size = size.1 >> config.section.as_power();
        let geo_pow = config.geo.as_power();
        let net_size = size.0 * size.1 >> (2 * geo_pow);
        let flood_offset = (2 * 4 + (1 + 4 + 4) * 4 + 2 * net_size + 2 * geo_pow * 4 + 2 * flood_size * geo_pow * 4) as u64;
        let expected_file_size = flood_offset + (flood_size * 4) as u64;
        assert_eq!(
            vpr_file.metadata().unwrap().len(),
            expected_file_size as u64
        );
        let mut vpr = BufReader::new(vpr_file);
        vpr.seek(SeekFrom::Start(flood_offset)).unwrap();
        (0 .. flood_size)
            .map(|_| vpr.read_u32::<E>().unwrap())
            .collect()
    };
    info!("\ttook {} sec", (Instant::now() - start_vpr).as_secs());

    info!("Loading vmc...");
    let start_vmc = Instant::now();
    let (height, meta) = {
        let mut vpc_base = BufReader::new(File::open(&config.path_vmc).unwrap());
        info!("\tLoading compression tables...");
        let mut st_table = Vec::<i32>::with_capacity(size.1 as usize);
        let mut sz_table = Vec::<i16>::with_capacity(size.1 as usize);
        for _ in 0 .. size.1 {
            st_table.push(vpc_base.read_i32::<E>().unwrap());
            sz_table.push(vpc_base.read_i16::<E>().unwrap());
        }
        info!("\tDecompressing level data...");
        let splay = Splay::new(&mut vpc_base);
        let total = (size.0 * size.1) as usize;
        let mut height = vec![0u8; total];
        let mut meta = vec![0u8; total];

        height.chunks_mut(size.0 as _)
            .zip(meta.chunks_mut(size.0 as _))
            .zip(st_table.iter())
            .collect::<Vec<_>>()
            .par_chunks_mut(64)
            .for_each(|source_group| {
                //Note: a separate file per group is required
                let mut vpc = BufReader::new(File::open(&config.path_vmc).unwrap());
                for &mut ((ref mut h_row, ref mut m_row), offset) in source_group {
                    vpc.seek(SeekFrom::Start(*offset as u64)).unwrap();
                    splay.expand1(&mut vpc, h_row);
                    splay.expand2(&mut vpc, m_row);
                }
            });

        (height, meta)
    };
    info!("\ttook {} sec", (Instant::now() - start_vmc).as_secs());

    Level {
        size: size,
        flood_map: flood,
        height: height,
        meta: meta,
        palette: load_palette(&config.path_palette),
        terrains: config.terrains,
    }
}
