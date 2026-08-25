use crate::config::text::Reader;
use crate::config::Settings;

use std::collections::HashMap;
use std::fs::File;

pub type Worlds = HashMap<String, String>;

pub fn load(file: File) -> Worlds {
    let mut fi = Reader::new(file);
    let count = fi.next_value::<usize>();
    (0..count)
        .map(|_| {
            fi.advance();
            fi.scan()
        })
        .collect()
}

/// `wrlds.dat` from a full install, or Fostral from the open-source tree.
pub fn load_from_settings(settings: &Settings) -> Worlds {
    if settings.check_path("wrlds.dat") {
        return load(settings.open_relative("wrlds.dat"));
    }
    const FOSTRAL: &str = "thechain/fostral/world.ini";
    if settings.check_path(FOSTRAL) {
        let mut worlds = HashMap::new();
        worlds.insert("Fostral".to_string(), FOSTRAL.to_string());
        return worlds;
    }
    panic!(
        "Can't find wrlds.dat or Fostral world data at {:?}",
        settings.data_path
    );
}
