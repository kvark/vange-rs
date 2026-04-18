use crate::{config::Settings, config::text::Reader};

use std::collections::HashMap;

pub struct ModelInfo {
    pub path: String,
    pub scale: f32,
}

pub struct Registry {
    pub model_infos: HashMap<String, ModelInfo>,
}

impl Registry {
    pub fn load(settings: &Settings) -> Registry {
        Self::load_from_file(settings.open_relative("game.lst"))
    }

    pub fn load_from_file(file: std::fs::File) -> Registry {
        Self::load_reader(file)
    }

    /// Reader-based variant of [`Registry::load_from_file`]. Used by
    /// the web build to parse `game.lst` out of a zip archive.
    pub fn load_reader<R: std::io::Read>(reader: R) -> Registry {
        let mut reg = Registry {
            model_infos: HashMap::new(),
        };
        let mut fi = Reader::new(reader);

        while !fi.cur().starts_with("NumModel") {
            fi.advance();
        }
        let count: u32 = fi.cur().split_whitespace().nth(1).unwrap().parse().unwrap();
        let max_size: u8 = fi.next_key_value("MaxSize");

        for i in 0..count {
            let num = fi.next_key_value("ModelNum");
            assert_eq!(i, num);
            let name: String = fi.next_key_value("Name");
            let size: u8 = fi.next_key_value("Size");
            let key: String = fi.next_key_value("NameID");
            reg.model_infos.insert(
                key,
                ModelInfo {
                    path: name,
                    scale: size as f32 / max_size as f32,
                },
            );
        }

        reg
    }
}
