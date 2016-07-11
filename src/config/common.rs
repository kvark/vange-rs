use std::fs::File;
use config::text::Reader;


pub struct Nature {
    pub gravity: f32,
    pub density: f32,
    pub scale_general: f32,
    pub movement_detection_threshold: u8,
}

pub struct Impulse {
    pub elastic_restriction: f32,
    pub elastic_time_scale_factor: f32,
    pub rolling_scale: f32,
    pub normal_threshold: f32,
    pub k_wheel: f32,
    pub factors: [f32; 2],
    pub k_friction: f32,
}

pub struct Car {
    pub rudder_step: u16,
    pub rudder_max: u16,
    pub rudder_k_decr: f32,
    pub traction_inc: u16,
    pub traction_dec: u16,
}

pub struct Global {
    pub speed_factor: f32,
    pub mobility_factor: f32,
    pub water_speed_factor: f32,
    pub air_speed_factor: f32,
    pub underground_speed_factor: f32,
    pub k_traction_turbo: f32,
    pub f_brake_max: f32,
}

pub struct Helicopter {
    pub max_height: u16,
    pub height_incr: u16,
    pub height_decr: u16,
    pub k_thrust: f32,
    pub k_rotate: f32,
    pub k_strife: f32,
    pub max_time: u8,
    pub convert: [f32; 2],
    pub rudder_decr: f32,
    pub traction_decr: f32,
    pub z_offset: f32,
    pub ampl: f32,
    pub dphi: u16,
    pub circle_radius: [f32; 2],
    pub circle_dphi: u16,
}

pub struct Drag {
    pub speed_vw: (f32, f32),
    pub wheel_speed: f32,
    pub z: f32,
    pub free_vw: (f32, f32),
    pub wheel_vw: (f32, f32),
    pub spring_vw: (f32, f32),
    pub coll_vw: (f32, f32),
    pub helicopter_vw: (f32, f32),
    pub float_vw: (f32, f32),
    pub friction_vw: (f32, f32),
    pub abs_stop_vw: (f32, f32),
    pub stuff: f32,
    pub swamp: f32,
    pub mole: f32,
    pub abs_min_vw: (f32, f32),
}

pub struct Common {
    pub nature: Nature,
    //pub impulse: Impulse,
    //pub car: Car,
    //pub global: Global,
    //pub heli: Helicopter,
    //pub drag: Drag,
}

pub fn load(file: File) -> Common {
    let mut fi = Reader::new(file);
    fi.advance();
    assert_eq!(fi.cur(), "COMMON:\t\t2");
    Common {
        nature: Nature {
            gravity: fi.next_key_value("g:"),
            density: fi.next_key_value("density:"),
            scale_general: {
                fi.advance(); //skip dt0
                fi.next_key_value("scale_general:")
            },
            movement_detection_threshold: {
                fi.advance(); //num_calls_analysis
                let mdt = fi.next_key_value("movement_detection_threshold:");
                fi.advance(); //num_skip_updates
                fi.advance(); //wheel_analyze
                fi.advance(); //analysis_off
                mdt
            },
        },
    }
}
