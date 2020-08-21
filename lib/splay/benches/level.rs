#![feature(test)]

use byteorder::{LittleEndian as E, ReadBytesExt};
use std::{
    fs::File,
    io::{Read, Seek},
};

const VMC_PATH: &'static str = "/hub/gog/Vangers/game/thechain/fostral/output.vmc";
const SIZE: [usize; 2] = [1 << 11, 1 << 14];

#[bench]
fn load_level(bench: &mut test::Bencher) {
    let mut file = File::open(VMC_PATH).unwrap();
    let table: Vec<_> = (0..SIZE[1])
        .map(|_| {
            let offset = file.read_i32::<E>().unwrap();
            let size = file.read_i16::<E>().unwrap();
            (offset, size)
        })
        .collect();

    let splay = splay::Splay::new(&mut file);
    let mut height = vec![0u8; SIZE[0]];
    let mut meta = vec![0u8; SIZE[0]];
    let data_offset = file.seek(std::io::SeekFrom::Current(0)).unwrap();
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).unwrap();

    bench.iter(|| {
        for &(offset, size) in table[..0x100].iter() {
            let off = offset as usize - data_offset as usize;
            splay.expand(&buffer[off..off + size as usize], &mut height, &mut meta);
        }
    });
}
