extern crate ron;
extern crate vangers;

use std::fs::File;

#[test]
fn load_settings() {
    let file = File::open("config/settings.template.ron").unwrap();
    ron::de::from_reader::<_, vangers::config::settings::Settings>(file)
        .unwrap();
}
