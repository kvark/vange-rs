extern crate byteorder;
extern crate env_logger;
#[macro_use]
extern crate log;

use byteorder::{BigEndian as BE, ReadBytesExt};

struct Splay {
	tree1: [i32; 512],
	tree1u: [u8; 512],
	tree3: [i32; 512],
	tree3u: [u8; 512],
}

impl Splay {
	fn new<I: ReadBytesExt>(input: &mut I) -> Splay {
		let mut splay = Splay {
			tree1: [0; 512],
			tree1u: [0; 512],
			tree3: [0; 512],
			tree3u: [0; 512],
		};
		for i in 0..512 {
			let v = input.read_i32::<BE>().unwrap();
			splay.tree1[i] = v;
			splay.tree1u[i] = (!v as u8).wrapping_add(1);
		}
		for i in 0..512 {
			let v = input.read_i32::<BE>().unwrap();
			splay.tree3[i] = v;
			splay.tree3u[i] = (!v as u8).wrapping_add(1);
		}
		splay
	}
}

struct Level {
	size: (u32, u32),
	flood_map: Vec<u32>,
}

fn load(path: &str, size_power: (u32, u32), geo_power: u32, section_power: u32) -> Level {
	use std::fs::File;
	use std::io::{Seek, SeekFrom};

	let size = (1 << size_power.0, 1 << size_power.1);

	info!("Loading vpr...");
	let flood = {
		let mut vpr = File::open(format!("{}.vpr", path)).unwrap();
		let flood_size = size.1 >> section_power;
		let net_size = size.0 * size.1 >> (geo_power + geo_power);
		let flood_offset = (2*4 + (1 + 4 + 4)*4 + 2*net_size + 2*geo_power*4 + 2*flood_size*geo_power*4) as u64;
		let expected_file_size = flood_offset + (flood_size*4) as u64;
		assert_eq!(vpr.metadata().unwrap().len(), expected_file_size as u64);
		vpr.seek(SeekFrom::Start(flood_offset)).unwrap();
		(0..flood_size).map(|_|
			vpr.read_u32::<BE>().unwrap()
		).collect()
	};
	
	info!("Loading vmc...");
	{
		let mut vpc = File::open(format!("{}.vmc", path)).unwrap();
		info!("\tLoading compression tables...");
		let mut st_table = Vec::<i32>::with_capacity(size.1 as usize);
		let mut sz_table = Vec::<i16>::with_capacity(size.1 as usize);
		for _ in 0 .. size.1 {
			st_table.push(vpc.read_i32::<BE>().unwrap());
			sz_table.push(vpc.read_i16::<BE>().unwrap());
		}
		let splay = Splay::new(&mut vpc);
	}

	info!("Done.");
	Level {
		size: size,
		flood_map: flood,
	}
}

fn main() {
	env_logger::init().unwrap();
    let _lev = load("D:/gog/Vangers/thechain/fostral/output", (11, 14), 5, 7);
}
