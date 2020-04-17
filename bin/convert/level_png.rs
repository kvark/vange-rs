use vangers::level::LevelLayers;

use ron;
use serde::{Serialize, Deserialize};

use std::{
    fs::File,
    path::PathBuf,
};


#[derive(Serialize, Deserialize)]
struct MultiPng {
    size: (u32, u32),
    height: String,
    material: String,
}

pub fn save(path: &PathBuf, layers: LevelLayers) {
    use std::io::Write;

    let mp = MultiPng {
        size: layers.size,
        height: "height.png".to_string(),
        material: "material.png".to_string(),
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
        for ((&h0, &h1), &delta) in layers.het0
            .iter()
            .zip(&layers.het1)
            .zip(&layers.delta)
        {
            data.extend_from_slice(&[h0, h1, delta]);
        }
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&data)
            .unwrap();
    }
    {
        println!("\t\t{}...", mp.material);
        let file = File::create(path.with_file_name(mp.material)).unwrap();
        let mut encoder = png::Encoder::new(file, layers.size.0 as u32, layers.size.1 as u32);
        encoder.set_color(png::ColorType::RGB);
        data.clear();
        for (&m0, &m1) in layers.mat0.iter().zip(&layers.mat1) {
            data.extend_from_slice(&[m0 << 4, m1 << 4, 0, m0 & 0xF0, m1 & 0xF0, 0]);
        }
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&data)
            .unwrap();
    }
}

pub fn load(path: &PathBuf) -> LevelLayers {
    let level_file = File::open(path).unwrap();
    let mp = ron::de::from_reader::<_, MultiPng>(level_file).unwrap();
    let mut layers = LevelLayers::new(mp.size);
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
        println!("\t\t{}...", mp.material);
        let file = File::open(path.with_file_name(mp.material)).unwrap();
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
        for chunk in data.chunks(stride + stride) {
            layers.mat0.push((chunk[0] >> 4) | chunk[0 + stride]);
            layers.mat1.push((chunk[1] >> 4) | chunk[1 + stride]);
        }
    }

    layers
}
