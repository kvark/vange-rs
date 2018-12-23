extern crate env_logger;
extern crate getopts;
extern crate image;
extern crate m3d;
extern crate png;
extern crate ron;
#[macro_use]
extern crate serde;
extern crate tiff;
extern crate vangers;


use std::io::BufWriter;
use std::fs::File;
use std::path::PathBuf;

fn import_image(path: &PathBuf) -> vangers::level::LevelData {
    println!("\tLoading the image...");
    let image = image::open(path).unwrap().to_rgba();
    println!("\tImporting the level...");
    let size = (image.width() as i32, image.height() as i32);
    vangers::level::LevelData::import(&image.into_raw(), size)
}

pub fn save_tiff(path: &PathBuf, layers: vangers::level::LevelLayers) {
    let images = [
        tiff::Image {
            width: layers.size.0 as u32,
            height: layers.size.1 as u32,
            bpp: 8,
            name: "h0",
            data: &layers.het0,
        },
        tiff::Image {
            width: layers.size.0 as u32,
            height: layers.size.1 as u32,
            bpp: 8,
            name: "h1",
            data: &layers.het1,
        },
        tiff::Image {
            width: layers.size.0 as u32,
            height: layers.size.1 as u32,
            bpp: 8,
            name: "del",
            data: &layers.delta,
        },
        tiff::Image {
            width: layers.size.0 as u32,
            height: layers.size.1 as u32,
            bpp: 4,
            name: "m0",
            data: &layers.mat0,
        },
        tiff::Image {
            width: layers.size.0 as u32,
            height: layers.size.1 as u32,
            bpp: 4,
            name: "m1",
            data: &layers.mat1,
        },
    ];

    let file = BufWriter::new(File::create(path).unwrap());
    tiff::save(file, &images).unwrap();
}

#[derive(Serialize, Deserialize)]
struct MultiPng {
    size: (u32, u32),
    height: String,
    material: String,
}

pub fn save_multi_png(path: &PathBuf, layers: vangers::level::LevelLayers) {
    use png::Parameter;
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
        png::ColorType::RGB.set_param(&mut encoder);
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
        png::ColorType::RGB.set_param(&mut encoder);
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

pub fn load_multi_png(path: &PathBuf) -> vangers::level::LevelLayers {
    let level_file = File::open(path).unwrap();
    let mp = ron::de::from_reader::<_, MultiPng>(level_file).unwrap();
    let mut layers = vangers::level::LevelLayers::new(mp.size);
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


fn main() {
    use std::env;
    env_logger::init().unwrap();

    let args: Vec<_> = env::args().collect();
    let mut options = getopts::Options::new();
    options
        .parsing_style(getopts::ParsingStyle::StopAtFirstFree)
        .optflag("h", "help", "print this help menu");

    let matches = options.parse(&args[1 ..]).unwrap();
    if matches.opt_present("h") || matches.free.len() != 2 {
        println!("Vangers resource converter");
        let brief = format!(
            "Usage: {} [options] <input> <output>",
            args[0]
        );
        println!("{}", options.usage(&brief));
        return;
    }

    let src_path = PathBuf::from(matches.free[0].as_str());
    let dst_path = PathBuf::from(matches.free[1].as_str());

    match (
        src_path.extension().and_then(|ostr| ostr.to_str()).unwrap_or(""),
        dst_path.extension().and_then(|ostr| ostr.to_str()).unwrap_or(""),
    ) {
        ("m3d", "ron") => {
            let file = File::open(&src_path).unwrap();
            println!("\tLoading M3D...");
            let raw = m3d::FullModel::load(file);
            println!("\tExporting OBJ data...");
            raw.export_obj(&dst_path);
        }
        ("ron", "md3") => {
            println!("\tImporting OBJ data...");
            let model = m3d::FullModel::import_obj(&src_path);
            println!("\tSaving M3D...");
            model.save(File::create(&dst_path).unwrap());
        }
        ("ini", "bmp") | ("ini", "png") | ("ini", "tga") => {
            println!("\tLoading the level...");
            let config = vangers::level::LevelConfig::load(&src_path);
            let level = vangers::level::load(&config);
            let data = level.export();
            println!("\tSaving the image...");
            image::save_buffer(
                &dst_path, &data,
                level.size.0 as u32, level.size.1 as u32,
                image::ColorType::RGBA(8),
            ).unwrap();
        }
        ("ini", "ron") => {
            println!("\tLoading the level...");
            let config = vangers::level::LevelConfig::load(&src_path);
            let layers = vangers::level::load_layers(&config);
            println!("\tSaving multiple PNGs...");
            save_multi_png(&dst_path, layers);
        }
        ("ini", "tiff") => {
            println!("\tLoading the level...");
            let config = vangers::level::LevelConfig::load(&src_path);
            let layers = vangers::level::load_layers(&config);
            println!("\tSaving TIFF layers...");
            save_tiff(&dst_path, layers);
        }
        ("ini", "vmp") => {
            println!("\tLoading the VMC...");
            let config = vangers::level::LevelConfig::load(&src_path);
            let level = vangers::level::load(&config);
            println!("\tSaving VMP...");
            vangers::level::LevelData::from(level).save_vmp(&dst_path);
        }
        ("bmp", "vmc") | ("png", "vmc") | ("tga", "vmc") => {
            let level = import_image(&src_path);
            println!("\tSaving VMC...");
            level.save_vmc(&dst_path);
        }
        ("bmp", "vmp") | ("png", "vmp") | ("tga", "vmp") => {
            let level = import_image(&src_path);
            println!("\tSaving VMP...");
            level.save_vmp(&dst_path);
        }
        ("ron", "vmp") => {
            println!("\tLoading multiple PNGs...");
            let layers = load_multi_png(&src_path);
            println!("\tSaving VMP...");
            let level = vangers::level::LevelData::import_layers(layers);
            level.save_vmp(&dst_path);
        }
        (in_ext, out_ext) => {
            panic!("Don't know how to convert {} to {}", in_ext, out_ext);
        }
    }
}
