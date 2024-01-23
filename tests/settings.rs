#[test]
fn load_settings() {
    let file = std::fs::File::open("config/settings.template.ron").unwrap();
    ron::de::from_reader::<_, vangers::config::settings::Settings>(file).unwrap();
}

#[test]
fn load_ffi_config() {
    ron::de::from_reader::<_, vangers::config::settings::Geometry>(std::fs::File::open("res/ffi/geometry.ron").unwrap()).unwrap();
    ron::de::from_reader::<_, vangers::config::settings::Render>(std::fs::File::open("res/ffi/render-full.ron").unwrap()).unwrap();
    ron::de::from_reader::<_, vangers::config::settings::Render>(std::fs::File::open("res/ffi/render-compat.ron").unwrap()).unwrap();
}
