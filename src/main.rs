extern crate byteorder;
extern crate env_logger;
#[macro_use]
extern crate log;
extern crate progressive;

mod level;
mod splay;

use level::Power;


fn main() {
	env_logger::init().unwrap();
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
}
