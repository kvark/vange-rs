#![feature(test)]

extern crate byteorder;
extern crate test;
extern crate splay;

use std::io::{Read, Seek, SeekFrom, Cursor};
use std::fs::File;
use byteorder::{LittleEndian as E, ReadBytesExt};

const VMC_PATH: &'static str = "/mnt/data/gog/Vangers/game/thechain/fostral/output.vmc";
const SIZE: [usize; 2] = [1<<11, 1<<14];


#[bench]
fn load_level(bench: &mut test::Bencher) {
    let mut file = File::open(VMC_PATH).unwrap();
    let table: Vec<_> = (0 .. SIZE[1]).map(|_| {
        let offset = file.read_i32::<E>().unwrap();
        file.read_i16::<E>().unwrap();
        offset
    }).collect();

    let splay = splay::Splay::new(&mut file);
    let mut height = vec![0u8; SIZE[0]];
    let mut meta = vec![0u8; SIZE[0]];
    let base = file.seek(std::io::SeekFrom::Current(0)).unwrap();
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).unwrap();

    bench.iter(|| {
        let mut cursor = Cursor::new(&mut buffer);
        for &offset in table[..0x100].iter() {
            cursor.seek(SeekFrom::Start(offset as u64 - base)).unwrap();
            splay.expand(&mut cursor, &mut height, &mut meta);
        }
    });
}
