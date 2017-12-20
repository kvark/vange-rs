extern crate cgmath;
extern crate getopts;
extern crate gfx;
extern crate glutin;
#[macro_use]
extern crate log;
extern crate time;
extern crate vangers;

mod app;
#[path = "../boilerplate.rs"]
mod boilerplate;

fn main() {
    let (settings, mut events_loop, window, mut device, mut factory, main_targets) = boilerplate::init();

    let mut app = app::LevelView::new(&settings, main_targets, &mut factory);

    let mut encoder = gfx::Encoder::from(factory.create_command_buffer());
    let mut last_time = time::precise_time_s() as f32;
    let mut running = true;

    while running {
        use gfx::Device;
        use glutin::GlContext;

        events_loop.poll_events(|event| {
            if let glutin::Event::WindowEvent { event, .. } = event {
                if !app.react(event, &mut factory) {
                    running = false;
                }
            }
        });

        let delta = time::precise_time_s() as f32 - last_time;
        app.update(delta);
        app.draw(&mut encoder);

        encoder.flush(&mut device);
        window.swap_buffers().unwrap();
        device.cleanup();
        last_time += delta;
    }
}
