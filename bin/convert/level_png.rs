use crate::layers::LevelLayers;

use ron;
use serde::{Deserialize, Serialize};

use std::{fs::File, path::PathBuf};

#[derive(Serialize, Deserialize)]
struct MultiPng {
    size: (u32, u32),
    height: String,
    num_terrains: u8,
    material_lo: String,
    material_hi: String,
}

pub fn save(path: &PathBuf, layers: LevelLayers, palette: &[u8]) {
    use std::io::Write;

    let mp = MultiPng {
        size: layers.size,
        height: "height.png".to_string(),
        num_terrains: layers.num_terrains,
        material_lo: "material_lo.png".to_string(),
        material_hi: "material_hi.png".to_string(),
    };
    let string = ron::ser::to_string_pretty(&mp, ron::ser::PrettyConfig::default()).unwrap();
    let mut level_file = File::create(path).unwrap();
    write!(level_file, "{}", string).unwrap();
    let mut data = Vec::with_capacity(3 * (layers.size.0 as usize) * layers.size.1 as usize);

    {
        println!("\t\t{}...", mp.height);
        let file = File::create(path.with_file_name(mp.height)).unwrap();
        let mut encoder = png::Encoder::new(file, layers.size.0 as u32, layers.size.1 as u32);
        encoder.set_color(png::ColorType::RGB);
        data.clear();
        for ((&h0, &h1), &delta) in layers.het0.iter().zip(&layers.het1).zip(&layers.delta) {
            data.extend_from_slice(&[h0, h1, delta]);
        }
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&data)
            .unwrap();
    }
    {
        println!("\t\t{}...", mp.material_lo);
        let file = File::create(path.with_file_name(mp.material_lo)).unwrap();
        let mut encoder = png::Encoder::new(file, layers.size.0 as u32, layers.size.1 as u32);
        encoder.set_color(png::ColorType::Indexed);
        encoder.set_palette(palette.to_vec());
        encoder.set_depth(png::BitDepth::Four);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&layers.mat0)
            .unwrap();
    }
    {
        println!("\t\t{}...", mp.material_hi);
        let file = File::create(path.with_file_name(mp.material_hi)).unwrap();
        let mut encoder = png::Encoder::new(file, layers.size.0 as u32, layers.size.1 as u32);
        encoder.set_color(png::ColorType::Indexed);
        encoder.set_palette(palette.to_vec());
        encoder.set_depth(png::BitDepth::Four);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&layers.mat1)
            .unwrap();
    }
}

pub fn load(path: &PathBuf) -> LevelLayers {
    let level_file = File::open(path).unwrap();
    let mp = ron::de::from_reader::<_, MultiPng>(level_file).unwrap();
    let mut layers = LevelLayers::new(mp.size, mp.num_terrains);
    {
        println!("\t\t{}...", mp.height);
        let file = File::open(path.with_file_name(mp.height)).unwrap();
        let decoder = png::Decoder::new(file);
        let (info, mut reader) = decoder.read_info().unwrap();
        assert_eq!((info.width, info.height), mp.size);
        let stride = match info.color_type {
            png::ColorType::RGB => 3,
            png::ColorType::RGBA => 4,
            _ => panic!("non-RGB image provided"),
        };
        let mut data = vec![0u8; stride * (layers.size.0 as usize) * (layers.size.1 as usize)];
        assert_eq!(info.bit_depth, png::BitDepth::Eight);
        assert_eq!(info.buffer_size(), data.len());
        reader.next_frame(&mut data).unwrap();
        for chunk in data.chunks(stride) {
            layers.het0.push(chunk[0]);
            layers.het1.push(chunk[1]);
            layers.delta.push(chunk[2]);
        }
    }
    {
        println!("\t\t{}...", mp.material_lo);
        let file = File::open(path.with_file_name(mp.material_lo)).unwrap();
        let mut decoder = png::Decoder::new(file);
        decoder.set_transformations(png::Transformations::empty());
        let (info, mut reader) = decoder.read_info().unwrap();
        assert_eq!((info.width, info.height), mp.size);
        assert_eq!(info.bit_depth, png::BitDepth::Four);
        layers
            .mat0
            .resize(info.width as usize * info.height as usize / 2, 0);
        assert_eq!(info.buffer_size(), layers.mat0.len());
        reader.next_frame(&mut layers.mat0).unwrap();
    }
    {
        println!("\t\t{}...", mp.material_hi);
        let file = File::open(path.with_file_name(mp.material_hi)).unwrap();
        let mut decoder = png::Decoder::new(file);
        decoder.set_transformations(png::Transformations::empty());
        let (info, mut reader) = decoder.read_info().unwrap();
        assert_eq!((info.width, info.height), mp.size);
        assert_eq!(info.bit_depth, png::BitDepth::Four);
        layers
            .mat1
            .resize(info.width as usize * info.height as usize / 2, 0);
        assert_eq!(info.buffer_size(), layers.mat1.len());
        reader.next_frame(&mut layers.mat1).unwrap();
    }

    layers
}
