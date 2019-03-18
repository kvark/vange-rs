use byteorder::{LittleEndian as E, ReadBytesExt, WriteBytesExt};

use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::fs::File;
use std::path::Path;
use std::time::Instant;


mod config;

pub use self::config::{LevelConfig, TerrainConfig};

pub type TerrainType = u8;
pub const NUM_TERRAINS: usize = 8;

pub type Altitude = u8;
pub type Delta = Altitude;
const DOUBLE_LEVEL: u8 = 1 << 6;
const DELTA_SHIFT0: u8 = 2 + 3;
const DELTA_SHIFT1: u8 = 0 + 3;
const DELTA_MASK: u8 = 0x3;
const TERRAIN_SHIFT: u8 = 3;
pub const HEIGHT_SCALE: u32 = 255;

pub struct Level {
    pub size: (i32, i32),
    pub flood_map: Vec<u8>,
    pub flood_section_power: usize,
    pub height: Vec<u8>,
    pub meta: Vec<u8>,
    pub palette: [[u8; 4]; 0x100],
    pub terrains: [TerrainConfig; NUM_TERRAINS],
}

pub struct LevelLayers {
    pub size: (u32, u32),
    pub het0: Vec<u8>,
    pub het1: Vec<u8>,
    pub delta: Vec<u8>,
    pub mat0: Vec<u8>,
    pub mat1: Vec<u8>,
}

impl LevelLayers {
    pub fn new(size: (u32, u32)) -> Self {
        let total = (size.0 * size.1) as usize;
        LevelLayers {
            size,
            het0: Vec::with_capacity(total),
            het1: Vec::with_capacity(total),
            delta: Vec::with_capacity(total),
            mat0: Vec::with_capacity(total / 2),
            mat1: Vec::with_capacity(total / 2),
        }
    }
}

pub struct Point(pub Altitude, pub TerrainType);

pub enum Texel {
    Single(Point),
    Dual {
        low: Point,
        high: Point,
        delta: Delta,
    }
}

impl Texel {
    pub fn top(&self) -> Altitude {
        match *self {
            Texel::Single(ref p) => p.0,
            Texel::Dual { ref high, .. } => high.0,
        }
    }
}

impl Level {
    pub fn new_test() -> Self {
        let tc = TerrainConfig {
            shadow_offset: 0,
            height_shift: 0,
            colors: 0..1,
        };
        Level {
            size: (2, 1),
            flood_map: vec![0],
            flood_section_power: 0,
            height: vec![0, 0],
            meta: vec![0, 0],
            palette: [[0xFF; 4]; 0x100],
            // not pretty, I know
            terrains: [
                tc.clone(), tc.clone(), tc.clone(), tc.clone(),
                tc.clone(), tc.clone(), tc.clone(), tc.clone(),
            ],
        }
    }

    pub fn get(&self, mut coord: (i32, i32)) -> Texel {
        fn get_terrain(meta: u8) -> TerrainType {
            (meta >> TERRAIN_SHIFT) & (NUM_TERRAINS as u8 - 1)
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
            let d0 = (meta0 & DELTA_MASK) << DELTA_SHIFT0;
            let d1 = (meta1 & DELTA_MASK) << DELTA_SHIFT1;
            Texel::Dual {
                low: Point(self.height[i & !1], get_terrain(meta0)),
                high: Point(self.height[i | 1], get_terrain(meta1)),
                delta: d0 + d1,
            }
        } else {
            Texel::Single(Point(self.height[i], get_terrain(meta)))
        }
    }

    pub fn export(&self) -> Vec<u8> {
        let mut data = vec![0; self.size.0 as usize * self.size.1 as usize * 4];
        for y in 0 .. self.size.1 {
            let base_y = (y * self.size.0) as usize * 4;
            for x in 0 .. self.size.0 {
                let base_x = base_y + x as usize * 4;
                let color = &mut data[base_x .. base_x + 4];
                match self.get((x, y)) {
                    Texel::Single(Point(alt, ty)) => {
                        color[0] = alt;
                        color[1] = alt;
                        color[2] = 0;
                        color[3] = ty | (ty << 4);
                    }
                    Texel::Dual {
                        low: Point(low_alt, low_ty),
                        high: Point(high_alt, high_ty),
                        delta,
                    } => {
                        color[0] = low_alt;
                        color[1] = high_alt;
                        color[2] = delta;
                        color[3] = low_ty | (high_ty << 4);
                    }
                }
            }
        }
        data
    }
}

#[allow(unused)]
fn print_palette(data: &[[u8; 4]], info: &str) {
    print!("Palette - {}:", info);
    for i in 0 .. 3 {
        print!("\n\t");
        for j in 0 .. 0x100 {
            print!("{:02X}", data[j][i]);
        }
    }
    print!("\n");
}

pub fn read_palette(input: File, config: Option<&[TerrainConfig]>) -> [[u8; 4]; 0x100] {
    let mut file = BufReader::new(input);
    let mut data = [[0; 4]; 0x100];
    for p in data.iter_mut() {
        file.read(&mut p[.. 3]).unwrap();
        //p[0] <<= 2; p[1] <<= 2; p[2] <<= 2;
    }
    //print_palette(&data, "read from file");
    if let Some(terrains) = config {
        // see `PalettePrepare` of the original
        data[0] = [0; 4];

        for tc in terrains {
            for c in &mut data[tc.colors.start as usize][.. 3] {
                *c >>= 1;
            }
        }

        for i in 0 .. 16 {
            let mut value = [(i * 4) as u8; 4];
            value[3] = 0;
            data[224 + i] = value;
        }

        //print_palette(&data, "corrected");
    }
    // see `XGR_Screen::setpal` of the original
    for p in data.iter_mut() {
        p[0] <<= 2; p[1] <<= 2; p[2] <<= 2;
    }
    //print_palette(&data, "scale");
    //TODO: there is quite a bit of logic missing here,
    // see `GeneralTableOpen` and `PalettePrepare` of the original.
    data
}


fn report_time(start: Instant) {
    let d = Instant::now() - start;
    info!(
        "\ttook {} ms",
        d.as_secs() as u32 * 1000 + d.subsec_nanos() / 1_000_000
    );
}

pub fn load_flood(config: &LevelConfig) -> Vec<u8> {
    let size = (config.size.0.as_value(), config.size.1.as_value());

    let instant = Instant::now();
    let flood_map = {
        let vpr_file = match File::open(&config.path_data.with_extension("vpr")) {
            Ok(file) => file,
            Err(_) => return Vec::new(),
        };

        info!("Loading flood map...");
        let flood_size = size.1 >> config.section.as_power();
        let geo_pow = config.geo.as_power();
        let net_size = size.0 * size.1 >> (2 * geo_pow);
        let flood_offset = (2 * 4 + (1 + 4 + 4) * 4 + 2 * net_size + 2 * geo_pow * 4
            + 2 * flood_size * geo_pow * 4) as u64;
        let expected_file_size = flood_offset + (flood_size * 4) as u64;
        assert_eq!(
            vpr_file.metadata().unwrap().len(),
            expected_file_size as u64
        );
        let mut vpr = BufReader::new(vpr_file);
        vpr.seek(SeekFrom::Start(flood_offset)).unwrap();
        (0 .. flood_size)
            .map(|_| vpr.read_u32::<E>().unwrap() as u8)
            .collect()
    };

    report_time(instant);
    flood_map
}


pub struct LevelData {
    pub height: Vec<u8>,
    pub meta: Vec<u8>,
    size: (i32, i32),
}

impl From<Level> for LevelData {
    fn from(level: Level) -> Self {
        LevelData {
            height: level.height,
            meta: level.meta,
            size: level.size,
        }
    }
}

fn avg(a: u8, b: u8) -> u8 {
    (a>>1) + (b>>1) + (a & b & 1)
}

impl LevelData {
    pub fn save_vmp(&self, path: &Path) {
        let mut vmp = BufWriter::new(File::create(path).unwrap());
        self.height
            .chunks(self.size.0 as _)
            .zip(self.meta.chunks(self.size.0 as _))
            .for_each(|(h_row, m_row)| {
                vmp.write(h_row).unwrap();
                vmp.write(m_row).unwrap();
            });
    }

    pub fn save_vmc(&self, path: &Path) {
        use splay::Splay;
        let mut vmc = BufWriter::new(File::create(path).unwrap());

        let base_offset = self.size.1 as u64 * (2 + 4) + Splay::tree_size();
        for i in 0 .. self.size.1 {
            vmc.write_i32::<E>(base_offset as i32 + i * self.size.0 * 2).unwrap();
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

    pub fn import(data: &[u8], size: (i32, i32)) -> Self {
        let total = (size.0 * size.1) as usize;
        assert_eq!(data.len(), total * 4);
        let mut level = LevelData {
            height: vec![0u8; total],
            meta: vec![0u8; total],
            size,
        };


        for y in 0 .. size.1 as usize {
            let row = y * size.0 as usize * 4 .. (y + 1) * size.0 as usize * 4;
            for (xd2, color) in data[row].chunks(8).enumerate() {
                let i = y * size.0 as usize + xd2 * 2;
                let delta = (avg(color[2], color[6]) >> DELTA_SHIFT1).min(0xF);
                // check if this is double layer
                if delta != 0 {
                    // average between two texels
                    let mat = avg(color[3], color[7]);
                    level.meta[i + 0] = DOUBLE_LEVEL |
                        ((mat & 0xF) << TERRAIN_SHIFT) |
                        (delta >> 2);
                    level.meta[i + 1] = DOUBLE_LEVEL |
                        ((mat >> 4) << TERRAIN_SHIFT) |
                        (delta & DELTA_MASK);
                    level.height[i + 0] = avg(color[0], color[4]);
                    level.height[i + 1] = avg(color[1], color[5]);
                } else {
                    // average between low and high
                    level.meta[i + 0] = (color[3] & 0xF) << TERRAIN_SHIFT;
                    level.meta[i + 1] = (color[7] & 0xF) << TERRAIN_SHIFT;
                    level.height[i + 0] = avg(color[0], color[1]);
                    level.height[i + 1] = avg(color[4], color[5]);
                }
            }
        }

        level
    }

    pub fn export_layers(&self) -> LevelLayers {
        let mut ll = LevelLayers::new((self.size.0 as u32, self.size.1 as u32));

        for y in 0 .. self.size.1 as usize {
            let range = y * self.size.0 as usize .. (y + 1) * self.size.0 as usize;
            let hrow = &self.height[range.clone()];
            let mrow = &self.meta[range];
            for ((&h0, &h1), (&m0, &m1)) in hrow.iter().step_by(2)
                .zip(hrow[1..].iter().step_by(2))
                .zip(mrow.iter().step_by(2).zip(mrow[1..].iter().step_by(2)))
            {
                let t0 = (m0 >> TERRAIN_SHIFT) & (NUM_TERRAINS as u8 - 1);
                let t1 = (m1 >> TERRAIN_SHIFT) & (NUM_TERRAINS as u8 - 1);
                if m0 & DOUBLE_LEVEL != 0 {
                    let d = ((m0 & DELTA_MASK) << DELTA_SHIFT0) + ((m1 & DELTA_MASK) << DELTA_SHIFT1);
                    //assert!(h0 + d <= h1); //TODO: figure out why this isn't true
                    ll.het0.push(h0); ll.het0.push(h0);
                    ll.het1.push(h1); ll.het1.push(h1);
                    ll.delta.push(d); ll.delta.push(d);
                    ll.mat0.push(t0 | (t0 << 4));
                    ll.mat1.push(t1 | (t1 << 4));
                } else {
                    ll.het0.push(h0); ll.het0.push(h1);
                    ll.het1.push(h0); ll.het1.push(h1);
                    ll.delta.push(0); ll.delta.push(0);
                    ll.mat0.push(t0 | (t1 << 4));
                    ll.mat1.push(t0 | (t1 << 4));
                }
            }
        }

        ll
    }

    pub fn import_layers(ll: LevelLayers) -> Self {
        let total = ll.size.0 as usize * ll.size.1 as usize;
        let mut height = Vec::with_capacity(total);
        let mut meta = Vec::with_capacity(total);

        for (((&h0a, &h0b), (&h1a, &h1b)), ((&da, &db), (&mat0, &mat1))) in
            ll.het0.iter().step_by(2).zip(ll.het0[1..].iter().step_by(2))
            .zip(ll.het1.iter().step_by(2).zip(ll.het1[1..].iter().step_by(2)))
            .zip(ll.delta.iter().step_by(2).zip(ll.delta[1..].iter().step_by(2)).zip(ll.mat0.iter().zip(&ll.mat1)))
        {
            //assert!(h0a + da <= h1a && h0b + db <= h1b);
            let delta = avg(da, db);
            if delta != 0 {
                //Note: mat0b and mat1a are ignored here, assuming the same as mat0a and mat1b respectively
                meta.push(DOUBLE_LEVEL |
                    ((mat0 & 0xF) << TERRAIN_SHIFT) |
                    (delta >> 2));
                meta.push(DOUBLE_LEVEL |
                    ((mat1 >> 4) << TERRAIN_SHIFT) |
                    (delta & DELTA_MASK));
                height.push(avg(h0a, h0b));
                height.push(avg(h1a, h1b));
            } else {
                //Note: mat1 and deltas are ignored here, assuming mat0 == mat1
                height.push(avg(h0a, h1a));
                height.push(avg(h0b, h1b));
                meta.push((mat0 & 0xF) << TERRAIN_SHIFT);
                meta.push((mat0 >> 4) << TERRAIN_SHIFT);
            }
        }

        LevelData {
            size: (ll.size.0 as i32, ll.size.1 as i32),
            meta,
            height,
        }
    }
}


pub fn load_vmc(path: &Path, size: (i32, i32)) -> LevelData {
    use rayon::prelude::*;
    use splay::Splay;

    info!("Loading height map...");
    let instant = Instant::now();
    let total = (size.0 * size.1) as usize;
    let mut level = LevelData {
        height: vec![0u8; total],
        meta: vec![0u8; total],
        size,
    };

    let mut vmc_base = BufReader::new(File::open(path).unwrap());

    info!("\tLoading compression tables...");
    let mut st_table = Vec::<i32>::with_capacity(size.1 as usize);
    let mut sz_table = Vec::<i16>::with_capacity(size.1 as usize);
    for _ in 0 .. size.1 {
        st_table.push(vmc_base.read_i32::<E>().unwrap());
        sz_table.push(vmc_base.read_i16::<E>().unwrap());
    }

    info!("\tDecompressing level data...");
    let splay = Splay::new(&mut vmc_base);

    level.height
        .chunks_mut(size.0 as _)
        .zip(level.meta.chunks_mut(size.0 as _))
        .zip(st_table.iter().zip(&sz_table))
        .collect::<Vec<_>>()
        .par_chunks_mut(64)
        .for_each(|source_group| {
            //Note: a separate file per group is required
            let mut vmc = File::open(path).unwrap();
            let data_size: i16 = source_group.iter().map(|(_, (_, &size))| size).max().unwrap();
            let mut data = vec![0u8; data_size as usize];
            for &mut ((ref mut h_row, ref mut m_row), (offset, &size)) in source_group {
                vmc.seek(SeekFrom::Start(*offset as u64)).unwrap();
                vmc.read(&mut data[.. size as usize]).unwrap();
                splay.expand(&data[.. size as usize], h_row, m_row);
            }
        });

    report_time(instant);
    level
}

pub fn load_vmp(path: &Path, size: (i32, i32)) -> LevelData {
    let total = (size.0 * size.1) as usize;
    let mut level = LevelData {
        height: vec![0u8; total],
        meta: vec![0u8; total],
        size,
    };

    let mut vmp = BufReader::new(File::open(path).unwrap());
    level.height
        .chunks_mut(size.0 as _)
        .zip(level.meta.chunks_mut(size.0 as _))
        .for_each(|(h_row, m_row)| {
            vmp.read(h_row).unwrap();
            vmp.read(m_row).unwrap();
        });

    level
}

pub fn load(config: &LevelConfig) -> Level {
    info!("Loading data map...");
    let size = (config.size.0.as_value(), config.size.1.as_value());
    let LevelData { height, meta, size } = if config.is_compressed {
        load_vmc(&config.path_data.with_extension("vmc"), size)
    } else {
        load_vmp(&config.path_data.with_extension("vmp"), size)
    };

    info!("Loading flood map...");
    let flood_map = load_flood(config);
    let palette = File::open(&config.path_palette)
        .expect("Unable to open the palette file");

    Level {
        size,
        flood_map,
        flood_section_power: config.section.as_power() as usize,
        height,
        meta,
        palette: read_palette(palette, Some(&config.terrains)),
        terrains: config.terrains.clone(),
    }
}

pub fn load_layers(config: &LevelConfig) -> LevelLayers {
    LevelData::from(load(config)).export_layers()
}
