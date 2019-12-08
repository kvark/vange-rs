#[test]
fn load_settings() {
    let file = std::fs::File::open("config/settings.template.ron").unwrap();
    ron::de::from_reader::<_, vangers::config::settings::Settings>(file)
        .unwrap();
}
