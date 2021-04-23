mod app;
#[path = "../boilerplate.rs"]
mod boilerplate;

fn main() {
    let (mut harness, settings) = boilerplate::Harness::init(boilerplate::HarnessOptions {
        title: "level",
        uses_level: true,
    });

    let app = app::LevelView::new(
        &settings,
        harness.extent,
        &harness.device,
        &mut harness.queue,
    );

    harness.main_loop(app);
}
