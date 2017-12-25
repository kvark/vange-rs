use config::text::Reader;
use std::fs::File;

pub const ORIGINAL_FPS: u8 = 14; //TODO: read from PRM
pub const SPEED_CORRECTION_FACTOR: f32 = 1.0; // it is in the config, but the original game uses a hard-coded 1.0

pub type Traction = f32;
pub type Angle = f32;

#[derive(Debug)]
pub struct VelocityPair {
    pub v: f32, //linear
    pub w: f32, //angular
}

pub struct Nature {
    pub gravity: f32,
    pub density: f32,
    pub time_delta0: f32,
    pub scale_general: f32,
    pub num_calls_analysis: u8,
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
    pub rudder_step: Angle,
    pub rudder_max: Angle,
    pub rudder_k_decr: f32,
    pub traction_incr: Traction,
    pub traction_decr: Traction,
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
    pub speed: VelocityPair,
    pub wheel_speed: f32,
    pub z: f32,
    pub free: VelocityPair,
    pub wheel: VelocityPair,
    pub spring: VelocityPair,
    pub coll: VelocityPair,
    pub helicopter: VelocityPair,
    pub float: VelocityPair,
    pub friction: VelocityPair,
    pub abs_stop: VelocityPair,
    pub stuff: f32,
    pub swamp: f32,
    pub mole: f32,
    pub abs_min: VelocityPair,
}

pub struct Terrain {
    pub dz_max: f32,
    pub min_wall_delta: f32,
}

pub struct Mole {
    pub k_elastic_mole: f32,
    pub k_mole: f32,
    pub k_mole_rudder: f32,
    pub mole_emerging_fz: f32,
    pub mole_submerging_fz: f32,
}

pub struct Contact {
    pub k_elastic_wheel: f32,
    pub k_elastic_spring: f32,
    pub k_elastic_xy: f32,
    pub k_elastic_db_coll: f32,
    pub k_destroy_level: f32,
    pub strong_ground_collision_threshold: f32,
    pub strong_double_collision_threshold: f32,
    pub k_friction_wheel_x: f32,
    pub k_friction_wheel_x_back: f32,
    pub k_friction_wheel_y: f32,
    pub k_friction_wheel_z: f32,
    pub k_friction_spring: f32,
}

pub struct Common {
    pub nature: Nature,
    pub impulse: Impulse,
    pub car: Car,
    pub global: Global,
    pub heli: Helicopter,
    pub drag: Drag,
    pub terrain: Terrain,
    pub mole: Mole,
    pub contact: Contact,
}

fn get_pair(
    reader: &mut Reader<File>,
    name: &str,
) -> VelocityPair {
    let sv = format!("V_{}:", name);
    let sw = format!("W_{}:", name);
    VelocityPair {
        v: reader.next_key_value(&sv),
        w: reader.next_key_value(&sw),
    }
}

pub fn load(file: File) -> Common {
    let mut fi = Reader::new(file);
    fi.advance();
    assert_eq!(fi.cur(), "COMMON:\t\t2");
    let traction_scale = 1.0 / 64.0;
    let angle_scale = {
        use std::f32::consts::PI;
        const PI_BITS: usize = 11;
        PI / (1 << PI_BITS) as f32
    };
    Common {
        nature: Nature {
            gravity: fi.next_key_value("g:"),
            density: fi.next_key_value("density:"),
            time_delta0: fi.next_key_value("dt0:"),
            scale_general: fi.next_key_value("scale_general:"),
            num_calls_analysis: fi.next_key_value("num_calls_analysis:"),
            movement_detection_threshold: {
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
            rudder_step: fi.next_key_value::<u16>("rudder_step:") as f32 * angle_scale,
            rudder_max: fi.next_key_value::<u16>("rudder_max:") as f32 * angle_scale,
            rudder_k_decr: fi.next_key_value("rudder_k_decr:"),
            traction_incr: fi.next_key_value::<u16>("traction_increment:") as f32 * traction_scale,
            traction_decr: fi.next_key_value::<u16>("traction_decrement:") as f32 * traction_scale,
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
            speed: get_pair(&mut fi, "drag_speed"),
            wheel_speed: fi.next_key_value("V_drag_wheel_speed:"),
            z: fi.next_key_value("V_drag_z:"),
            free: get_pair(&mut fi, "drag_free"),
            wheel: get_pair(&mut fi, "drag_wheel"),
            spring: get_pair(&mut fi, "drag_spring"),
            coll: get_pair(&mut fi, "drag_coll"),
            helicopter: get_pair(&mut fi, "drag_helicopter"),
            float: get_pair(&mut fi, "drag_float"),
            friction: get_pair(&mut fi, "drag_friction"),
            abs_stop: get_pair(&mut fi, "abs_stop"),
            stuff: fi.next_key_value("V_drag_stuff:"),
            swamp: fi.next_key_value("V_drag_swamp:"),
            mole: fi.next_key_value("V_drag_mole:"),
            abs_min: get_pair(&mut fi, "abs_min"),
        },
        terrain: Terrain {
            dz_max: fi.next_key_value("dZ_max:"),
            min_wall_delta: fi.next_key_value("MIN_WALL_DELTA:"),
        },
        mole: Mole {
            k_elastic_mole: fi.next_key_value("k_elastic_mole:"),
            k_mole: fi.next_key_value("K_mole:"),
            k_mole_rudder: fi.next_key_value("k_mole_rudder:"),
            mole_emerging_fz: fi.next_key_value("mole_emerging_fz:"),
            mole_submerging_fz: fi.next_key_value("mole_submerging_fz:"),
        },
        contact: Contact {
            k_elastic_wheel: fi.next_key_value("k_elastic_wheel:"),
            k_elastic_spring: fi.next_key_value("k_elastic_spring:"),
            k_elastic_xy: fi.next_key_value("k_elastic_xy:"),
            k_elastic_db_coll: fi.next_key_value("k_elastic_db_coll:"),
            k_destroy_level: fi.next_key_value("k_destroy_level:"),
            strong_ground_collision_threshold: fi.next_key_value(
                "strong_ground_collision_threshould:",
            ),
            strong_double_collision_threshold: fi.next_key_value(
                "strong_double_collision_threshould:",
            ),
            k_friction_wheel_x: fi.next_key_value("k_friction_wheel_x:"),
            k_friction_wheel_x_back: fi.next_key_value("k_friction_wheel_x_back:"),
            k_friction_wheel_y: fi.next_key_value("k_friction_wheel_y:"),
            k_friction_wheel_z: fi.next_key_value("k_friction_wheel_z:"),
            k_friction_spring: fi.next_key_value("k_friction_spring:"),
        },
    }
}
