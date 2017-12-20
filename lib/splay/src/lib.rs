extern crate byteorder;

use byteorder::{LittleEndian as E, ReadBytesExt};
use std::io::Read;

pub struct Splay {
    tree1: [i32; 512],
    tree2: [i32; 512],
}

impl Splay {
    pub fn new<I: ReadBytesExt>(input: &mut I) -> Splay {
        let mut splay = Splay {
            tree1: [0; 512],
            tree2: [0; 512],
        };
        for i in 0 .. 512 {
            let v = input.read_i32::<E>().unwrap();
            splay.tree1[i] = v;
        }
        for i in 0 .. 512 {
            let v = input.read_i32::<E>().unwrap();
            splay.tree2[i] = v;
        }
        splay
    }

    fn decompress<I: Read, F: Fn(u8, u8) -> u8>(
        tree: &[i32],
        input: &mut I,
        output: &mut [u8],
        fun: F,
    ) {
        let mut last_char = 0u8;
        let mut bit = 0;
        let mut cur = 0u8;
        for out in output.iter_mut() {
            let mut code = 1i32;
            while code > 0 {
                bit = if bit == 0 {
                    cur = input.read_u8().unwrap();
                    7
                } else {
                    bit - 1
                };
                let i = ((code as usize) << 1) + ((cur >> bit) as usize & 1);
                code = tree[i];
            }
            last_char = fun(last_char, -code as u8);
            *out = last_char;
        }
    }

    #[allow(dead_code)]
    fn decompress_orig<I: Read, F: Fn(u8, u8) -> u8>(
        tree: &[i32],
        input: &mut I,
        output: &mut [u8],
        fun: F,
    ) {
        let mut last_char = 0u8;
        let mut c_index = 1usize;
        let mut cur_size = 0;
        loop {
            let cur = input.read_u8().unwrap();
            for bit in (0 .. 8).rev() {
                let i = (c_index << 1) + ((cur >> bit) as usize & 1);
                let code = tree[i];
                c_index = if code <= 0 {
                    last_char = fun(last_char, -code as u8);
                    output[cur_size] = last_char;
                    cur_size += 1;
                    if cur_size == output.len() {
                        return;
                    }
                    1
                } else {
                    code as usize
                };
            }
        }
    }

    pub fn expand1<I: Read>(
        &self,
        input: &mut I,
        output: &mut [u8],
    ) {
        Splay::decompress(&self.tree1, input, output, |b, c| b.wrapping_add(c));
    }
    pub fn expand2<I: Read>(
        &self,
        input: &mut I,
        output: &mut [u8],
    ) {
        Splay::decompress(&self.tree2, input, output, |b, c| b ^ c);
    }
}
