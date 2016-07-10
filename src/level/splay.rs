use std::io::Read;
use byteorder::{LittleEndian as E, ReadBytesExt};


pub struct Splay {
    tree1: [i32; 512],
    tree1u: [u8; 512],
    tree2: [i32; 512],
    tree2u: [u8; 512],
}

impl Splay {
    pub fn new<I: ReadBytesExt>(input: &mut I) -> Splay {
        let mut splay = Splay {
            tree1: [0; 512],
            tree1u: [0; 512],
            tree2: [0; 512],
            tree2u: [0; 512],
        };
        for i in 0..512 {
            let v = input.read_i32::<E>().unwrap();
            splay.tree1[i] = v;
            splay.tree1u[i] = (!v as u8).wrapping_add(1);
        }
        for i in 0..512 {
            let v = input.read_i32::<E>().unwrap();
            splay.tree2[i] = v;
            splay.tree2u[i] = (!v as u8).wrapping_add(1);
        }
        splay
    }

    fn decompress<I: Read, F: Fn(u8, u8)->u8>(tree: &[i32], tree_char: &[u8],
                  input: &mut I, output: &mut [u8], fun: F) {
        let mut cur_size = 0;
        let mut last_char = 0u8;
        let mut c_index = 1usize;
        loop {
            let cur = input.read_u8().unwrap();
            for bit in (0..8).rev() {
                let i = (c_index << 1) + ((cur >> bit) as usize & 1);
                let code = tree[i];
                c_index = if code <= 0 {
                    last_char = fun(last_char, tree_char[i]);
                    output[cur_size] = last_char;
                    cur_size += 1;
                    if cur_size == output.len() {
                        return
                    }
                    1
                }else {
                    code as usize
                };
            }
        }
    }

    pub fn expand1<I: Read>(&self, input: &mut I, output: &mut [u8]) {
        Splay::decompress(&self.tree1, &self.tree1u, input, output, |b, c| b.wrapping_add(c));
    }
    pub fn expand2<I: Read>(&self, input: &mut I, output: &mut [u8]) {
        Splay::decompress(&self.tree2, &self.tree2u, input, output, |b, c| b^c);
    }
}
