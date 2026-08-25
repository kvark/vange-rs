#[test]
fn load_settings() {
    let file = std::fs::File::open("config/settings.template.ron").unwrap();
    ron::de::from_reader::<_, vangers::config::settings::Settings>(file).unwrap();
}

#[test]
fn template_points_at_the_sibling_vangers_tree() {
    let file = std::fs::File::open("config/settings.template.ron").unwrap();
    let set: vangers::config::settings::Settings = ron::de::from_reader(file).unwrap();
    assert_eq!(
        set.data_path.as_os_str(),
        std::ffi::OsStr::new("../Vangers/data")
    );
}

#[test]
fn load_resolves_sibling_vangers_when_present() {
    if !std::path::Path::new("../Vangers/data/thechain/fostral/world.ini").exists() {
        eprintln!("skipping: no sibling ../Vangers/data");
        return;
    }
    let set = vangers::config::Settings::load("config/settings.template.ron");
    assert!(
        set.check_path("thechain/fostral/world.ini"),
        "resolved data_path {:?}",
        set.data_path
    );
}

#[test]
fn load_ffi_config() {
    ron::de::from_reader::<_, vangers::config::settings::Geometry>(
        std::fs::File::open("res/ffi/geometry.ron").unwrap(),
    )
    .unwrap();
    ron::de::from_reader::<_, vangers::config::settings::Render>(
        std::fs::File::open("res/ffi/render-full.ron").unwrap(),
    )
    .unwrap();
    ron::de::from_reader::<_, vangers::config::settings::Render>(
        std::fs::File::open("res/ffi/render-compat.ron").unwrap(),
    )
    .unwrap();
}
