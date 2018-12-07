extern crate env_logger;
extern crate getopts;
extern crate image;
extern crate vangers;

use vangers::{config, level, model};

use std::path::PathBuf;

fn main() {
    use std::env;
    env_logger::init().unwrap();

    let settings = config::settings::Settings::load("config/settings.ron");

    let args: Vec<_> = env::args().collect();
    let mut options = getopts::Options::new();
    options
        .parsing_style(getopts::ParsingStyle::StopAtFirstFree)
        .optopt("m", "model", "M3D model resourcepath to export", "resource_path")
        .optopt("o", "object", "Object folder to import as M3D model", "resource_path")
        .optopt("l", "level", "INI level resource path to export", "level_path")
        .optflag("h", "help", "print this help menu");

    let matches = options.parse(&args[1 ..]).unwrap();
    if matches.opt_present("h") || matches.free.len() != 1 {
        println!("Vangers resource converter");
        let brief = format!(
            "Usage: {} [options] <out_dir>",
            args[0]
        );
        println!("{}", options.usage(&brief));
        return;
    }

    let out_dir = PathBuf::from(matches.free[0].as_str());
    if let Some(model_path) = matches.opt_str("m") {
        let file = settings.open_relative(&model_path);
        model::convert_m3d(file, &out_dir);
    }
    if let Some(object_path) = matches.opt_str("o") {
        let model = model::FullModel::import(&PathBuf::from(object_path));
        model.save(&out_dir);
    }
    if let Some(level_path) = matches.opt_str("l") {
        let ini_path = settings.data_path.join(&level_path);
        let config = level::LevelConfig::load(&ini_path);
        let level = level::load(&config);

        let mut data = vec![0u8; level.size.0 as usize * level.size.1 as usize * 4];
        for y in 0 .. level.size.1 {
            let base_y = (y * level.size.0) as usize * 4;
            for x in 0 .. level.size.0 {
                let base_x = base_y + x as usize * 4;
                let mut color = &mut data[base_x .. base_x + 4];
                match level.get((x, y)) {
                    level::Texel::Single(level::Point(alt, ty)) => {
                        color[0] = alt;
                        color[1] = alt;
                        color[2] = 0;
                        color[3] = ty << 4;
                    }
                    level::Texel::Dual {
                        low: level::Point(low_alt, low_ty),
                        high: level::Point(high_alt, high_ty),
                        delta,
                    } => {
                        color[0] = low_alt;
                        color[1] = high_alt;
                        color[2] = delta;
                        color[3] = low_ty + (high_ty << 4);
                    }
                }
            }
        }

        image::save_buffer(
            out_dir.join("level.bmp"), &data,
            level.size.0 as u32, level.size.1 as u32,
            image::ColorType::RGBA(8),
        ).unwrap();
    }
}
