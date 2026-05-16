fn main() {
    println!("cargo:rerun-if-changed=config/settings.template.ron");
}
