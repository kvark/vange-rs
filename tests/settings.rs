extern crate toml;
extern crate vangers;

use std::fs::File;
use std::io::Read;

#[test]
fn load_settings() {
    let mut string = String::new();
    File::open("config/settings.template.toml")
        .unwrap()
        .read_to_string(&mut string)
        .unwrap();
    toml::from_str::<vangers::config::settings::Settings>(&string)
        .unwrap();
}
