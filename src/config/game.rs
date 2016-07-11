use std::collections::HashMap;
use config::Settings;
use config::text::Reader;


pub struct Registry {
    pub model_paths: HashMap<String, String>,
}

impl Registry {
    pub fn load(settings: &Settings) -> Registry {
        let mut reg = Registry {
            model_paths: HashMap::new(),
        };
        let mut fi = Reader::new(settings.open("game.lst"));

        while !fi.cur().starts_with("NumModel") {
            fi.advance();
        }
        let count: u32 = fi.cur().split_whitespace()
            .nth(1).unwrap()
            .parse().unwrap();
        fi.advance(); // MaxSize

        for i in 0 .. count {
            let num = fi.next_key_value("ModelNum");
            assert_eq!(i, num);
            let name: String = fi.next_key_value("Name");
            let _size: u8 = fi.next_key_value("Size");
            let key: String = fi.next_key_value("NameID");
            reg.model_paths.insert(key, name);
        }

        reg
    }
}
