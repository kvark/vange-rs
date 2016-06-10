extern crate byteorder;
extern crate env_logger;
#[macro_use]
extern crate gfx;
extern crate gfx_window_glutin;
extern crate glutin;
#[macro_use]
extern crate log;
extern crate progressive;

mod level;
mod render;
mod splay;


use level::Power;

fn main() {
    env_logger::init().unwrap();
    let builder = glutin::WindowBuilder::new()
        .with_title("Rusty Vangers".to_string())
        .with_dimensions(800, 540)
        .with_vsync();
    let (window, mut device, mut factory, main_color, _main_depth) =
        gfx_window_glutin::init::<render::ColorFormat, render::DepthFormat>(builder);

    let mut encoder: gfx::Encoder<_, _> = factory.create_command_buffer().into();
    let render = render::init(&mut factory, main_color);

    let name = "fostral";
    let base = "/opt/GOG Games/Vangers/game/thechain";
    let config = level::Config {
        name: name.to_owned(),
        path_vpr: format!("{}/{}/output.vpr", base, name),
        path_vmc: format!("{}/{}/output.vmc", base, name),
        size: (Power(11), Power(14)),
        geo: Power(5),
        section: Power(7),
    };
    let _lev = level::load(&config);

    'main: loop {
        use gfx::Device;
        // loop over events
        for event in window.poll_events() {
            match event {
                glutin::Event::KeyboardInput(_, _, Some(glutin::VirtualKeyCode::Escape)) |
                glutin::Event::Closed => break 'main,
                _ => {},
            }
        }
        // draw a frame
        render.draw(&mut encoder);
        encoder.flush(&mut device);
        window.swap_buffers().unwrap();
        device.cleanup();
    }
}
