// Morton codes
// https://fgiesen.wordpress.com/2009/12/13/decoding-morton-codes/

// "Insert" a 0 bit after each of the 16 low bits of x
fn morton_part_1by1(arg: u32) -> u32 {
  var x = arg & 0x0000ffffu;         // x = ---- ---- ---- ---- fedc ba98 7654 3210
  x = (x ^ (x <<  8u)) & 0x00ff00ffu; // x = ---- ---- fedc ba98 ---- ---- 7654 3210
  x = (x ^ (x <<  4u)) & 0x0f0f0f0fu; // x = ---- fedc ---- ba98 ---- 7654 ---- 3210
  x = (x ^ (x <<  2u)) & 0x33333333u; // x = --fe --dc --ba --98 --76 --54 --32 --10
  x = (x ^ (x <<  1u)) & 0x55555555u; // x = -f-e -d-c -b-a -9-8 -7-6 -5-4 -3-2 -1-0
  return x;
}

// "Insert" two 0 bits after each of the 10 low bits of x
fn morton_part_1by2(arg: u32) -> u32 {
  var x = arg & 0x000003ffu;          // x = ---- ---- ---- ---- ---- --98 7654 3210
  x = (x ^ (x << 16u)) & 0xff0000ffu; // x = ---- --98 ---- ---- ---- ---- 7654 3210
  x = (x ^ (x <<  8u)) & 0x0300f00fu; // x = ---- --98 ---- ---- 7654 ---- ---- 3210
  x = (x ^ (x <<  4u)) & 0x030c30c3u; // x = ---- --98 ---- 76-- --54 ---- 32-- --10
  x = (x ^ (x <<  2u)) & 0x09249249u; // x = ---- 9--8 --7- -6-- 5--4 --3- -2-- 1--0
  return x;
}

fn encode_morton2(v: vec2<u32>) -> u32 {
  return (morton_part_1by1(v.y) << 1u) + morton_part_1by1(v.x);
}

fn encode_morton3(v: vec3<u32>) -> u32 {
  return (morton_part_1by2(v.z) << 2u) + (morton_part_1by2(v.y) << 1u) + morton_part_1by2(v.x);
}
