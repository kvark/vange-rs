extern crate ron;
extern crate vangers;
extern crate speculate;

use speculate::speculate;

speculate! {
    describe "settings" {
        before {
            let _default_settings = r#"
(
	data_path: "",
	game: (
		level: "Fostral", // see `wrlds.dat` for the list
		cycle: "Eleerection", // see `bunches.prm` for the list, leave empty for bonus worlds
		view: Perspective, // can be "Flat" or "Perspective"
		other: (
			count: 10, // number of NPC vangers
		),
		physics: (
			max_quant: 0.05,
			gpu_collision: false,
		),
	),
	car: (
		id: "OxidizeMonk",
		//id: "IronShadow",
		slots: [],
		//slots: ["HeavyLaser", "LightMissile", "LightFireBall"],
	),
	window: (
		title: "Rusty Road",
		size: (1280, 800),
	),
	render: (
		light: (
			pos: (1, 2, 4, 0), // w=0 for directional, w=1 for point light
			color: (1, 1, 1, 1),
		),
		terrain: RayTraced(
			mip_count: 10,
			max_jumps: 25,
			max_steps: 100,
			debug: false,
		),
		debug: (
			max_vertices: 512,
			collision_shapes: false,
			collision_map: false,
			impulses: false,
		),
	),
)
            "#;
            let _raytracedold_settings = r#"
(
	data_path: "",
	game: (
		level: "Fostral", // see `wrlds.dat` for the list
		cycle: "Eleerection", // see `bunches.prm` for the list, leave empty for bonus worlds
		view: Perspective, // can be "Flat" or "Perspective"
		other: (
			count: 10, // number of NPC vangers
		),
		physics: (
			max_quant: 0.05,
			gpu_collision: false,
		),
	),
	car: (
		id: "OxidizeMonk",
		//id: "IronShadow",
		slots: [],
		//slots: ["HeavyLaser", "LightMissile", "LightFireBall"],
	),
	window: (
		title: "Rusty Road",
		size: (1280, 800),
	),
	render: (
		light: (
			pos: (1, 2, 4, 0), // w=0 for directional, w=1 for point light
			color: (1, 1, 1, 1),
		),
		terrain: RayTracedOld,
		debug: (
			max_vertices: 512,
			collision_shapes: false,
			collision_map: false,
			impulses: false,
		),
	),
)
            "#;
        }

        it "Can load default settings" {
            ron::de::from_str::<vangers::config::settings::Settings>(_default_settings)
                .unwrap();
        }

        it "Can load RayTracedOld renderer settings" {
            ron::de::from_str::<vangers::config::settings::Settings>(_raytracedold_settings)
                .unwrap();
        }
    }
}

