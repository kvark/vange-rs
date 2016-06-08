use byteorder::{BigEndian as BE, ReadBytesExt};

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
			let v = input.read_i32::<BE>().unwrap();
			splay.tree1[i] = v;
			splay.tree1u[i] = (!v as u8).wrapping_add(1);
		}
		for i in 0..512 {
			let v = input.read_i32::<BE>().unwrap();
			splay.tree2[i] = v;
			splay.tree2u[i] = (!v as u8).wrapping_add(1);
		}
		splay
	}

	fn decompress<'a, 'b>(tree: &[i32], tree_char: &[u8], input: &'a [u8], output: &'b mut [u8]) -> (&'a [u8], &'b mut [u8]) {
		let mut char_count = 1<<11;
		let (mut ni, mut no) = (0, 0);
		let mut last_char = 0u8;
		let mut c_index = 1usize;
		loop {
			let cur = input[ni];
			ni += 1;
			for bit in (0..8).rev() {
				c_index = (c_index << 1) + ((cur >> bit) as usize & 1);
				if tree[c_index] <= 0 {
					last_char = last_char.wrapping_add(tree_char[c_index]);
					output[no] = last_char;
					no += 1;
					char_count -= 1;
					if char_count == 0 {
						let i = input.split_at(ni).1;
						let o = output.split_at_mut(no).1;
						return (i, o)
					}
					c_index = 1;
				}else {
					c_index = tree[c_index] as usize;
				}
			}
		}
	}

	pub fn expand<'a, 'b>(&self, input: &'a [u8], output: &'b mut [u8]) -> (&'a [u8], &'b mut [u8]) {
		let (i,o) = Splay::decompress(&self.tree1, &self.tree1u, input, output);
		Splay::decompress(&self.tree2, &self.tree2u, i, o)
	}
}
