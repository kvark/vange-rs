extern crate byteorder;

use byteorder::{LittleEndian as E, ReadBytesExt, WriteBytesExt};
use std::io::{Read, Write};

pub struct Splay {
    tree1: [i32; 512],
    tree2: [i32; 512],
}

//TODO: use iterators

impl Splay {
    pub fn new<I: ReadBytesExt>(input: &mut I) -> Self {
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

    pub fn write_trivial<O: WriteBytesExt>(output: &mut O) {
        for _ in 0 .. 2 {
            for i in 0i32 .. 256 {
                output.write_i32::<E>(i).unwrap();
            }
            for i in 0i32 .. 256 {
                output.write_i32::<E>(-i).unwrap();
            }
        }
    }

    pub fn tree_size() -> u64 {
        512 * 2 * 4
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

    pub fn expand<I: Read>(
        &self,
        input: &mut I,
        output1: &mut [u8],
        output2: &mut [u8],
    ) {
        Self::decompress(&self.tree1, input, output1, |b, c| b.wrapping_add(c));
        Self::decompress(&self.tree2, input, output2, |b, c| b ^ c);
    }

    pub fn compress_trivial<O: Write>(
        input1: &[u8],
        input2: &[u8],
        output: &mut O,
    ) {
        let mut last_char = 0;
        for &b in input1 {
            output.write_u8(b.wrapping_sub(last_char)).unwrap();
            last_char = b;
        }
        last_char = 0;
        for &b in input2 {
            output.write_u8(b ^ last_char).unwrap();
            last_char = b;
        }
    }
}
