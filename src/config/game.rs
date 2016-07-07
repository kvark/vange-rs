use std::collections::HashMap;
use std::io::BufReader;
use gfx;
use config::Settings;
use model;
use super::text::Reader;


pub struct Registry<R: gfx::Resources> {
    pub models: HashMap<String, model::Model<R>>,
}

impl<R: gfx::Resources> Registry<R> {
    pub fn load<F: gfx::Factory<R>>(settings: &Settings, factory: &mut F) -> Registry<R> {
        use progressive::progress;

        let mut reg = Registry {
            models: HashMap::new(),
        };
        let mut fi = Reader::new(settings.open("game.lst"));

        while !fi.cur().starts_with("NumModel") {
            fi.advance();
        }
        let count: u32 = fi.cur().split_whitespace()
            .nth(1).unwrap()
            .parse().unwrap();
        fi.advance(); // MaxSize

        info!("Loading game models...");
        for i in progress(0 .. count) {
            let num = fi.next_key_value("ModelNum");
            assert_eq!(i, num);
            let name: String = fi.next_key_value("Name");
            let _size: u8 = fi.next_key_value("Size");
            let key: String = fi.next_key_value("NameID");
            if name.ends_with(".m3d") {
                let mut br = BufReader::new(settings.open(&name));
                let m = model::load_m3d(&mut br, factory);
                reg.models.insert(key, m);
            }
        }

        reg
    }
}
