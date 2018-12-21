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
    height_low: String,
    height_high: String,
    delta: String,
    meta_low: String,
    meta_high: String,
}

pub fn save_multi_png(path: &PathBuf, layers: vangers::level::LevelLayers) {
    use png::Parameter;
    use std::io::Write;

    let mp = MultiPng {
        size: layers.size,
        height_low: "height_low.png".to_string(),
        height_high: "height_high.png".to_string(),
        delta: "delta.png".to_string(),
        meta_low: "meta_low.png".to_string(),
        meta_high: "meta_high.png".to_string(),
    };
    let string = ron::ser::to_string_pretty(&mp, ron::ser::PrettyConfig::default()).unwrap();
    let mut level_file = File::create(path).unwrap();
    write!(level_file, "{}", string).unwrap();

    let entries = [
        (&mp.height_low, &layers.het0, png::BitDepth::Eight),
        (&mp.height_high, &layers.het1, png::BitDepth::Eight),
        (&mp.delta, &layers.delta, png::BitDepth::Eight),
        (&mp.meta_low, &layers.mat0, png::BitDepth::Four),
        (&mp.meta_high, &layers.mat1, png::BitDepth::Four),
    ];
    for &(name, data, bpp) in &entries {
        println!("\t\t{}...", name);
        let file = File::create(path.with_file_name(name)).unwrap();
        let mut encoder = png::Encoder::new(file, layers.size.0 as u32, layers.size.1 as u32);
        png::ColorType::Grayscale.set_param(&mut encoder);
        bpp.set_param(&mut encoder);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(data)
            .unwrap();
    }
}

pub fn load_multi_png(path: &PathBuf) -> vangers::level::LevelLayers {
    let level_file = File::open(path).unwrap();
    let mp = ron::de::from_reader::<_, MultiPng>(level_file).unwrap();
    let mut layers = vangers::level::LevelLayers::new(mp.size);

    {
        let mut entries = [
            (&mp.height_low, &mut layers.het0, png::BitDepth::Eight),
            (&mp.height_high, &mut layers.het1, png::BitDepth::Eight),
            (&mp.delta, &mut layers.delta, png::BitDepth::Eight),
            (&mp.meta_low, &mut layers.mat0, png::BitDepth::Four),
            (&mp.meta_high, &mut layers.mat1, png::BitDepth::Four),
        ];
        for &mut (name, ref mut data, bpp) in &mut entries {
            println!("\t\t{}...", name);
            let file = File::open(path.with_file_name(name)).unwrap();
            let decoder = png::Decoder::new(file);
            let (info, mut reader) = decoder.read_info().unwrap();
            assert_eq!((info.width, info.height), mp.size);
            assert_eq!(info.color_type, png::ColorType::Grayscale);
            assert_eq!(info.bit_depth, bpp);
            assert_eq!(info.buffer_size(), data.capacity());
            data.resize(info.buffer_size(), 0);
            reader.next_frame(data).unwrap();
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
