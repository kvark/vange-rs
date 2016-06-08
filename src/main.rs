extern crate byteorder;
extern crate env_logger;
#[macro_use]
extern crate log;

mod splay;


use byteorder::{BigEndian as BE, ReadBytesExt};
use splay::Splay;

struct LevelConfig {
	name: String,
	path_vpr: String,
	path_vmc: String,
	size_power: (u32, u32),
	geo_power: u32,
	section_power: u32,
}

struct Level {
	size: (u32, u32),
	flood_map: Vec<u32>,
	data: Vec<u8>,
	splay: Splay,
}

fn load(config: &LevelConfig) -> Level {
	use std::fs::File;
	use std::io::{Read, Seek, SeekFrom};

	let size = (1 << config.size_power.0, 1 << config.size_power.1);

	info!("Loading vpr...");
	let flood = {
		let mut vpr = File::open(&config.path_vpr).unwrap();
		let flood_size = size.1 >> config.section_power;
		let net_size = size.0 * size.1 >> (2*config.geo_power);
		let flood_offset = (2*4 + (1 + 4 + 4)*4 + 2*net_size + 2*config.geo_power*4 + 2*flood_size*config.geo_power*4) as u64;
		let expected_file_size = flood_offset + (flood_size*4) as u64;
		assert_eq!(vpr.metadata().unwrap().len(), expected_file_size as u64);
		vpr.seek(SeekFrom::Start(flood_offset)).unwrap();
		(0..flood_size).map(|_|
			vpr.read_u32::<BE>().unwrap()
		).collect()
	};
	
	info!("Loading vmc...");
	let (data, splay) = {
		let mut vpc = File::open(&config.path_vmc).unwrap();
		info!("\tLoading compression tables...");
		let mut st_table = Vec::<i32>::with_capacity(size.1 as usize);
		let mut sz_table = Vec::<i16>::with_capacity(size.1 as usize);
		for _ in 0 .. size.1 {
			st_table.push(vpc.read_i32::<BE>().unwrap());
			sz_table.push(vpc.read_i16::<BE>().unwrap());
		}
		let data_size = vpc.metadata().unwrap().len() - vpc.seek(SeekFrom::Current(0)).unwrap();
		let splay = Splay::new(&mut vpc);
		let mut data = Vec::with_capacity(data_size as usize);
		vpc.read_to_end(&mut data).unwrap();
		(data, splay)
	};

	info!("Done.");
	Level {
		size: size,
		flood_map: flood,
		data: data,
		splay: splay,
	}
}

fn main() {
	env_logger::init().unwrap();
	let name = "fostral";
	let base = "D:/gog/Vangers/thechain";
	let config = LevelConfig {
		name: name.to_owned(),
		path_vpr: format!("{}/{}/output.vpr", base, name),
		path_vmc: format!("{}/{}/output.vmc", base, name),
		size_power: (11, 14),
		geo_power: 5,
		section_power: 7,
	};
    let _lev = load(&config);
}
