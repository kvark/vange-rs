extern crate env_logger;
extern crate getopts;
extern crate image;
extern crate m3d;
extern crate vangers;

use vangers::{config, level};

use std::fs::File;
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
        .optopt("o", "object", "Object folder to import as M3D model", "object_path")
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
        let raw = m3d::FullModel::load(file);
        raw.export_obj(&out_dir);
    }
    if let Some(object_path) = matches.opt_str("o") {
        let model = m3d::FullModel::import_obj(&PathBuf::from(object_path));
        let qualified_path = settings.data_path.join(&out_dir);
        model.save(File::create(&qualified_path).unwrap());
    }
    if let Some(level_path) = matches.opt_str("l") {
        let ini_path = settings.data_path.join(&level_path);
        let config = level::LevelConfig::load(&ini_path);
        let level = level::load(&config);
        let data = level.export();

        image::save_buffer(
            out_dir.join("level.bmp"), &data,
            level.size.0 as u32, level.size.1 as u32,
            image::ColorType::RGBA(8),
        ).unwrap();
    }
}
