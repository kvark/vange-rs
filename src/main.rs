extern crate byteorder;

struct Level {
	size: (u32, u32),
	flood_map: Vec<u32>,
}

fn load(path: &str, size_power: (u32, u32), geo_power: u32, section_power: u32) -> Level {
	use std::fs::File;
	use std::io::{Seek, SeekFrom};
	use byteorder::{BigEndian, ReadBytesExt};

	let mut vpr = File::open(format!("{}.vpr", path)).unwrap();
	let size = (1 << size_power.0, 1 << size_power.1);
	let flood_size = size.1 >> section_power;
	let net_size = size.0 * size.1 >> (geo_power + geo_power);
	let flood_offset = (2*4 + (1 + 4 + 4)*4 + 2*net_size + 2*geo_power*4 + 2*flood_size*geo_power*4) as u64;
	let expected_file_size = flood_offset + (flood_size*4) as u64;
	assert_eq!(vpr.metadata().unwrap().len(), expected_file_size as u64);
	vpr.seek(SeekFrom::Start(flood_offset)).unwrap();
	let flood = (0..flood_size).map(|_|
		vpr.read_u32::<BigEndian>().unwrap()
	).collect();

	Level {
		size: size,
		flood_map: flood,
	}
}

fn main() {
    let _lev = load("D:/gog/Vangers/thechain/fostral/output", (11, 14), 5, 7);
}
