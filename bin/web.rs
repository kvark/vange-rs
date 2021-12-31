use std::cell::RefCell;
use std::rc::Rc;

use log::info;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

extern crate console_error_panic_hook;

use crate::boilerplate::Application;

static mut BINDED: bool = false;

pub fn bind_once<T: Application + 'static>(application: Rc<RefCell<T>>) {
    use std::collections::HashMap;
    use winit::event::{ElementState, KeyboardInput, ModifiersState, VirtualKeyCode as Key};

    unsafe {
        if BINDED {
            return;
        }

        BINDED = true;
    }

    let window = web_sys::window().expect("should have a window in this context");

    {
        let mappings: HashMap<String, Key> = [
            (String::from("w"), Key::W),
            (String::from("ц"), Key::W),
            (String::from("a"), Key::A),
            (String::from("a"), Key::A),
            (String::from("s"), Key::S),
            (String::from("ы"), Key::S),
            (String::from("d"), Key::D),
            (String::from("в"), Key::D),
            (String::from("p"), Key::P),
            (String::from("з"), Key::P),
            (String::from("r"), Key::R),
            (String::from("к"), Key::R),
            (String::from("e"), Key::E),
            (String::from("у"), Key::E),
            (String::from("q"), Key::Q),
            (String::from("й"), Key::Q),
            (String::from("shift"), Key::LShift),
            (String::from("alt"), Key::LAlt),
        ]
        .iter()
        .cloned()
        .collect();

        let closure = Closure::wrap(Box::new(move |event: web_sys::KeyboardEvent| {
            let pressed = event.type_() == "keydown";
            let key = event.key().to_ascii_lowercase();

            match mappings.get(&key) {
                Some(key) => application.borrow_mut().on_key(KeyboardInput {
                    state: if pressed {
                        ElementState::Pressed
                    } else {
                        ElementState::Released
                    },
                    virtual_keycode: Some(*key),
                    modifiers: ModifiersState::default(),
                    scancode: 0,
                }),
                None => false,
            };
        }) as Box<dyn FnMut(_)>);

        window
            .add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref())
            .expect("should be able to bind keydown listener");
        window
            .add_event_listener_with_callback("keyup", closure.as_ref().unchecked_ref())
            .expect("should be able to bind keyup listener");
        closure.forget();
    }
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = log)]
    pub fn log(a: &str);

    #[wasm_bindgen(js_namespace = Date, js_name = now)]
    pub fn now() -> f64;

    #[wasm_bindgen(js_namespace = window, js_name = getFile)]
    pub fn get_js_file(file: &str) -> String;
}

fn create_file(file: &str, buf: &[u8]) {
    use std::{fs::File, io::Write};

    info!("Creating file {}", file);
    File::create(file)
        .expect(&format!("Unable to create {}", file))
        .write(buf)
        .expect(&format!("Unable to write in {}", file));
}

#[macro_use]
mod rsrc {
    macro_rules! get_bundled_file {
        () => {
            "../res_linux/"
        };
    }

    macro_rules! create_file {
        ($path:expr, $src_dir:expr) => {{
            create_file($path, include_bytes!(concat!($src_dir, $path)));
        }};
    }
}

pub fn create_fs() {
    info!("Creating fs");

    let js_files = [
        // vange-rs files
        "config/settings.ron",
        "res/shader/quat.inc.glsl",
        "res/shader/debug.wgsl",
        "res/shader/object.wgsl",
        "res/shader/encode.inc.glsl",
        "res/shader/quat.inc.wgsl",
        "res/shader/globals.inc.wgsl",
        "res/shader/downsample.glsl",
        "res/shader/body.inc.wgsl",
        "res/shader/shape.inc.glsl",
        "res/shader/surface.inc.wgsl",
        "res/shader/body.inc.glsl",
        "res/shader/globals.inc.glsl",
        "res/shader/debug_shape.glsl",
        "res/shader/shadow.inc.wgsl",
        "res/shader/surface.inc.glsl",
        "res/shader/color.inc.wgsl",
        "res/shader/terrain/locals.inc.wgsl",
        "res/shader/terrain/ray.wgsl",
        "res/shader/terrain/paint.wgsl",
        "res/shader/terrain/mip.wgsl",
        "res/shader/terrain/scatter.wgsl",
        "res/shader/terrain/slice.wgsl",
        "res/shader/physics/collision.inc.glsl",
        "res/shader/physics/collision_clear.glsl",
        "res/shader/physics/body_step.glsl",
        "res/shader/physics/collision_add.glsl",
        "res/shader/physics/body_push.glsl",
        "res/shader/physics/pulse.inc.glsl",
        "res/shader/physics/body_gather.glsl",
    ];

    for next in js_files {
        create_file(next, get_js_file(next).as_bytes());
    }

    #[cfg(not(feature = "nodata"))] {
        // vangers resouces
        create_file!("data/escaves.prm", get_bundled_file!());
        create_file!("data/common.prm", get_bundled_file!());
        create_file!("data/bunches.prm", get_bundled_file!());
        create_file!("data/game.lst", get_bundled_file!());
        create_file!("data/car.prm", get_bundled_file!());
        create_file!("data/wrlds.dat", get_bundled_file!());

        // the chain
        create_file!("data/thechain/threall/world.ini", get_bundled_file!());
        create_file!("data/thechain/threall/output.vmc", get_bundled_file!());
        create_file!("data/thechain/threall/terrain.prm", get_bundled_file!());
        create_file!("data/thechain/threall/output.vpr", get_bundled_file!());
        create_file!("data/thechain/threall/harmony.pal", get_bundled_file!());

        create_file!("data/thechain/boozeena/world.ini", get_bundled_file!());
        create_file!("data/thechain/boozeena/output.vmc", get_bundled_file!());
        create_file!("data/thechain/boozeena/terrain.prm", get_bundled_file!());
        create_file!("data/thechain/boozeena/output.vpr", get_bundled_file!());
        create_file!("data/thechain/boozeena/harmony.pal", get_bundled_file!());

        create_file!("data/thechain/weexow/world.ini", get_bundled_file!());
        create_file!("data/thechain/weexow/output.vmc", get_bundled_file!());
        create_file!("data/thechain/weexow/terrain.prm", get_bundled_file!());
        create_file!("data/thechain/weexow/output.vpr", get_bundled_file!());
        create_file!("data/thechain/weexow/harmony.pal", get_bundled_file!());

        create_file!("data/thechain/xplo/world.ini", get_bundled_file!());
        create_file!("data/thechain/xplo/output.vmc", get_bundled_file!());
        create_file!("data/thechain/xplo/terrain.prm", get_bundled_file!());
        create_file!("data/thechain/xplo/output.vpr", get_bundled_file!());
        create_file!("data/thechain/xplo/harmony.pal", get_bundled_file!());

        create_file!("data/thechain/hmok/world.ini", get_bundled_file!());
        create_file!("data/thechain/hmok/output.vmc", get_bundled_file!());
        create_file!("data/thechain/hmok/terrain.prm", get_bundled_file!());
        create_file!("data/thechain/hmok/output.vpr", get_bundled_file!());
        create_file!("data/thechain/hmok/harmony.pal", get_bundled_file!());

        create_file!("data/thechain/ark-a-znoy/world.ini", get_bundled_file!());
        create_file!("data/thechain/ark-a-znoy/output.vmc", get_bundled_file!());
        create_file!("data/thechain/ark-a-znoy/terrain.prm", get_bundled_file!());
        create_file!("data/thechain/ark-a-znoy/output.vpr", get_bundled_file!());
        create_file!("data/thechain/ark-a-znoy/harmony.pal", get_bundled_file!());

        create_file!("data/thechain/khox/world.ini", get_bundled_file!());
        create_file!("data/thechain/khox/output.vmc", get_bundled_file!());
        create_file!("data/thechain/khox/terrain.prm", get_bundled_file!());
        create_file!("data/thechain/khox/output.vpr", get_bundled_file!());
        create_file!("data/thechain/khox/harmony.pal", get_bundled_file!());

        // palettes
        create_file!("data/resource/pal/necross.pal", get_bundled_file!());
        create_file!("data/resource/pal/necross1.pal", get_bundled_file!());
        create_file!("data/resource/pal/fostral2.pal", get_bundled_file!());
        create_file!("data/resource/pal/xplo.pal", get_bundled_file!());
        create_file!("data/resource/pal/necross2.pal", get_bundled_file!());
        create_file!("data/resource/pal/fostral1.pal", get_bundled_file!());
        create_file!("data/resource/pal/glorx1.pal", get_bundled_file!());
        create_file!("data/resource/pal/glorx2.pal", get_bundled_file!());
        create_file!("data/resource/pal/fostral.pal", get_bundled_file!());
        create_file!("data/resource/pal/objects.pal", get_bundled_file!());
        create_file!("data/resource/pal/glorx.pal", get_bundled_file!());

        // models
        create_file!("data/resource/m3d/mechous/m13.m3d", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m10.m3d", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m5.prm", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m9.m3d", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/u5.prm", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m1.prm", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/u3.m3d", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m11.prm", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/u2.prm", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/u1.prm", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/r2.m3d", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/r3.m3d", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/r4.m3d", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m3.prm", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m14.m3d", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m9.prm", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m7.prm", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m10.prm", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/default.prm", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/r1.m3d", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m13.prm", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m12.prm", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/r4.prm", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m11.m3d", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m2.prm", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m6.m3d", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m12.m3d", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/u2.m3d", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m1.m3d", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m4.prm", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m8.prm", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/u1.m3d", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m4.m3d", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m14.prm", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/r5.m3d", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/r3.prm", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m3.m3d", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/r5.prm", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/u4.m3d", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/u4.prm", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m6.prm", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/r2.prm", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/r1.prm", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m5.m3d", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m7.m3d", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m8.m3d", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/u5.m3d", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/m2.m3d", get_bundled_file!());
        create_file!("data/resource/m3d/mechous/u3.prm", get_bundled_file!());
    }
}
