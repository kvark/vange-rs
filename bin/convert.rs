extern crate env_logger;
extern crate getopts;
extern crate image;
extern crate m3d;
extern crate vangers;


use std::fs::File;
use std::path::PathBuf;

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
        ("ini", "vmp") => {
            println!("\tLoading the VMC...");
            let mut config = vangers::level::LevelConfig::load(&src_path);
            config.is_compressed = true;
            let level = vangers::level::load(&config);
            println!("\tSaving VMP...");
            level.save_vmp(File::create(&dst_path).unwrap());
        }
        ("bmp", "ini") | ("png", "ini") | ("tga", "ini") => {
            println!("\tLoading the image...");
            let image = image::open(&src_path).unwrap().to_rgba().into_raw();
            println!("\tImporting the level...");
            let config = vangers::level::LevelConfig::load(&dst_path);
            let level = vangers::level::Level::import(&image, &config);
            let output = File::create(&config.path_data()).unwrap();
            if config.is_compressed {
                println!("\tSaving VMC...");
                level.save_vmc(output);
            } else {
                println!("\tSaving VMP...");
                level.save_vmp(output);
            }
        }
        (in_ext, out_ext) => {
            panic!("Don't know how to convert {} to {}", in_ext, out_ext);
        }
    }
}
