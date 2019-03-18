mod app;
#[path = "../boilerplate.rs"]
mod boilerplate;

fn main() {
    let (mut harness, settings) = boilerplate::Harness::init("level");

    let app = app::LevelView::new(&settings, &mut harness.device);

    harness.main_loop(app);
}
