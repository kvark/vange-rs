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
    pub traction_incr: u16,
    pub traction_decr: u16,
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
    pub impulse: Impulse,
    pub car: Car,
    pub global: Global,
    pub heli: Helicopter,
    pub drag: Drag,
}

fn get_pair(reader: &mut Reader<File>, name: &str) -> (f32, f32) {
    let sv = format!("V_{}:", name);
    let sw = format!("W_{}:", name);
    (
        reader.next_key_value(&sv),
        reader.next_key_value(&sw),
    )
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
                let mdt = fi.next_key_value("movement_detection_threshould:");
                fi.advance(); //num_skip_updates
                fi.advance(); //wheel_analyze
                fi.advance(); //analysis_off
                mdt
            },
        },
        impulse: Impulse {
            elastic_restriction: fi.next_key_value("elastic_restriction:"),
            elastic_time_scale_factor: fi.next_key_value("elastic_time_scale_factor:"),
            rolling_scale: fi.next_key_value("rolling_scale:"),
            normal_threshold: fi.next_key_value("normal_threshould:"),
            k_wheel: fi.next_key_value("k_wheel:"),
            factors: [
                fi.next_key_value("horizontal_impulse_factor:"),
                fi.next_key_value("vertical_impulse_factor:"),
            ],
            k_friction: fi.next_key_value("k_friction_impulse:"),
        },
        car: Car {
            rudder_step: fi.next_key_value("rudder_step:"),
            rudder_max: fi.next_key_value("rudder_max:"),
            rudder_k_decr: fi.next_key_value("rudder_k_decr:"),
            traction_incr: fi.next_key_value("traction_increment:"),
            traction_decr: fi.next_key_value("traction_decrement:"),
        },
        global: Global {
            speed_factor: fi.next_key_value("global_speed_factor:"),
            mobility_factor: fi.next_key_value("global_mobility_factor:"),
            water_speed_factor: fi.next_key_value("global_water_speed_factor:"),
            air_speed_factor: fi.next_key_value("global_air_speed_factor:"),
            underground_speed_factor: fi.next_key_value("global_underground_speed_factor:"),
            k_traction_turbo: fi.next_key_value("k_traction_turbo:"),
            f_brake_max: fi.next_key_value("f_brake_max:"),
        },
        heli: Helicopter {
            max_height: fi.next_key_value("max_helicopter_height:"),
            height_incr: fi.next_key_value("helicopter_height_incr:"),
            height_decr: fi.next_key_value("helicopter_height_decr:"),
            k_thrust: fi.next_key_value("k_helicopter_thrust:"),
            k_rotate: fi.next_key_value("k_helicopter_rotate:"),
            k_strife: fi.next_key_value("k_helicopter_strife:"),
            max_time: fi.next_key_value("max_helicopter_time:"),
            convert: [
                fi.next_key_value("heli_x_convert:"),
                fi.next_key_value("heli_y_convert:"),
            ],
            rudder_decr: fi.next_key_value("heli_rudder_decr:"),
            traction_decr: fi.next_key_value("heli_traction_decr:"),
            z_offset: fi.next_key_value("heli_z_offset:"),
            ampl: fi.next_key_value("helicopter_ampl:"),
            dphi: fi.next_key_value("helicopter_dphi:"),
            circle_radius: [
                fi.next_key_value("helicopter_circle_radius_x:"),
                fi.next_key_value("helicopter_circle_radius_y:"),
            ],
            circle_dphi: fi.next_key_value("helicopter_circle_dphi:"),
        },
        drag: Drag {
            speed_vw: get_pair(&mut fi, "drag_speed"),
            wheel_speed: fi.next_key_value("V_drag_wheel_speed:"),
            z: fi.next_key_value("V_drag_z:"),
            free_vw: get_pair(&mut fi, "drag_free"),
            wheel_vw: get_pair(&mut fi, "drag_wheel"),
            spring_vw: get_pair(&mut fi, "drag_spring"),
            coll_vw: get_pair(&mut fi, "drag_coll"),
            helicopter_vw: get_pair(&mut fi, "drag_helicopter"),
            float_vw: get_pair(&mut fi, "drag_float"),
            friction_vw: get_pair(&mut fi, "drag_friction"),
            abs_stop_vw: get_pair(&mut fi, "abs_stop"),
            stuff: fi.next_key_value("V_drag_stuff:"),
            swamp: fi.next_key_value("V_drag_swamp:"),
            mole: fi.next_key_value("V_drag_mole:"),
            abs_min_vw: get_pair(&mut fi, "abs_min"),
        },
    }
}
