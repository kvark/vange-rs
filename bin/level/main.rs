mod app;
#[path = "../boilerplate.rs"]
mod boilerplate;

fn main() {
    let (harness, settings) = boilerplate::Harness::init(boilerplate::HarnessOptions {
        title: "level",
        uses_level: true,
    });

    let app = app::LevelView::new(
        &settings,
        harness.extent,
        &harness.device,
        &harness.queue,
        &harness.downlevel_caps,
    );

    harness.main_loop(app);
}
