//! Reader for the Vangers `*.vot` files - the "mobile locations" (a.k.a.
//! moving land) that animate a rectangular patch of the level surface.
//!
//! Mirrors `MobileLocation::load`/`MLFrame::load` of `src/units/moveland.cpp`
//! in the original game. Compressed payloads are expanded eagerly here, so a
//! loaded [`Frame`] always exposes plain `sx * sy` byte planes.

use byteorder::{LittleEndian as E, ReadBytesExt};
use std::io;

pub mod rle;

/// `MLSign` of the original.
const SIGNATURE: &[u8; 3] = b"ML3";
/// `MLNAMELEN + 1`.
const NAME_LEN: usize = 16;
/// `MAX_KEYPHASE`.
pub const MAX_KEY_PHASE: usize = 4;

/// How the frame payload is meant to be applied to the surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// `delta` holds unsigned altitude *offsets*, with the direction taken
    /// from the per-texel sign bits.
    Relative = 0,
    /// `delta` holds the target altitudes directly. Zero means "leave alone".
    Absolute = 1,
    /// Authored as relative, converted to absolute by the map editor on the
    /// first playthrough. Only the editor ever runs the relative path for it;
    /// the game plays it back exactly like `Absolute`, and files in this mode
    /// carry no sign bits.
    Rel2Abs = 2,
}

impl Mode {
    /// Whether the game interpolates the frame instead of writing altitudes
    /// straight through - `mode == MLM_RELATIVE` of the original.
    pub fn is_relative(&self) -> bool {
        matches!(*self, Mode::Relative)
    }
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    BadSignature([u8; 3]),
    BadMode(u8),
    /// A packed plane did not expand into the expected number of bytes.
    BadCompression,
    /// The frame dimensions don't fit into memory.
    BadFrameSize(i32, i32),
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Error::Io(ref e) => write!(f, "I/O error: {}", e),
            Error::BadSignature(sig) => write!(f, "not a VOT file, signature is {:?}", sig),
            Error::BadMode(m) => write!(f, "unexpected mode {}", m),
            Error::BadCompression => write!(f, "unable to expand a packed frame plane"),
            Error::BadFrameSize(x, y) => write!(f, "invalid frame size {}x{}", x, y),
        }
    }
}

impl std::error::Error for Error {}

/// One step of the animation.
///
/// The `delta` and `terrain` planes are stored row-major over an `sx * sy`
/// rectangle anchored at `pos`, and always in expanded form.
pub struct Frame {
    /// `(x0, y0)` - top-left corner in level texels, before wrapping.
    pub pos: (i32, i32),
    /// `(sx, sy)` - extent of the affected rectangle.
    pub size: (i32, i32),
    /// Number of quants this frame is spread over. Altitude changes are
    /// interpolated across them; the terrain type lands on the last one.
    pub period: i32,
    /// Terrain type index applied to the whole frame, or a value `>= terrain
    /// count` to signal that `terrain` carries a per-texel plane instead.
    /// Negative means "don't touch the terrain type".
    pub surface_type: i32,
    /// Altitude offsets (relative modes) or absolute altitudes.
    pub delta: Vec<u8>,
    /// Per-texel terrain bits, already shifted into place. Empty when
    /// `surface_type` addresses a single terrain.
    pub terrain: Vec<u8>,
    /// One bit per texel, LSB first, giving the sign of `delta`.
    /// Empty in [`Mode::Absolute`].
    pub sign_bits: Vec<u32>,
}

impl Frame {
    /// Number of texels covered by the frame.
    pub fn area(&self) -> usize {
        self.size.0 as usize * self.size.1 as usize
    }

    /// Sign of the delta at a linear texel index: `true` means "downwards".
    pub fn is_negative(&self, index: usize) -> bool {
        match self.sign_bits.get(index >> 5) {
            Some(word) => word & (1 << (index & 31)) != 0,
            None => false,
        }
    }

    fn load<I: io::Read>(input: &mut I, mode: Mode, terrain_count: i32) -> Result<Self, Error> {
        let x0 = input.read_i32::<E>()?;
        let y0 = input.read_i32::<E>()?;
        let sx = input.read_i32::<E>()?;
        let sy = input.read_i32::<E>()?;
        let period = input.read_i32::<E>()?;
        let surface_type = input.read_i32::<E>()?;
        // Sizes of the packed planes, zero when they are stored raw.
        let csd = input.read_i32::<E>()?;
        let cst = input.read_i32::<E>()?;
        let _reserved = [input.read_i32::<E>()?, input.read_i32::<E>()?];

        if sx < 0 || sy < 0 || sx.checked_mul(sy).is_none() {
            return Err(Error::BadFrameSize(sx, sy));
        }
        let total = (sx * sy) as usize;

        let delta = read_plane(input, csd, total)?;
        let terrain = if surface_type >= terrain_count {
            read_plane(input, cst, total)?
        } else {
            Vec::new()
        };

        // `ss = sz/32 + 1` of the original - note the unconditional `+ 1`,
        // so an exact multiple of 32 still gets a trailing word.
        let sign_bits = if mode.is_relative() {
            let words = total / 32 + 1;
            let mut sb = Vec::with_capacity(words);
            for _ in 0..words {
                sb.push(input.read_u32::<E>()?);
            }
            sb
        } else {
            Vec::new()
        };

        Ok(Frame {
            pos: (x0, y0),
            size: (sx, sy),
            // `setPeriod` of the original clamps this on the way in.
            period: period.max(1),
            surface_type,
            delta,
            terrain,
            sign_bits,
        })
    }
}

/// Reads one `total`-byte plane, expanding it when `packed_size` is non-zero.
fn read_plane<I: io::Read>(
    input: &mut I,
    packed_size: i32,
    total: usize,
) -> Result<Vec<u8>, Error> {
    let mut plane = vec![0u8; total];
    if packed_size == 0 {
        input.read_exact(&mut plane)?;
    } else {
        let mut packed = vec![0u8; packed_size as usize];
        input.read_exact(&mut packed)?;
        rle::decode(&packed, &mut plane).ok_or(Error::BadCompression)?;
    }
    Ok(plane)
}

/// A whole moving-land animation: a looping sequence of [`Frame`]s.
pub struct MobileLocation {
    /// Name as stored in the file, used to address the location from scripts.
    pub name: String,
    pub mode: Mode,
    /// Terrain type the affected rectangle is re-registered with after each
    /// frame - `DryTerrain` of the original.
    pub dry_terrain: i32,
    /// Physical impulse imparted on whatever stands on the moving surface.
    pub impulse: i32,
    /// Phases the location can be told to stop at. Slot 0 is always the start.
    pub key_phases: [i32; MAX_KEY_PHASE],
    pub frames: Vec<Frame>,
}

impl MobileLocation {
    /// `terrain_count` is the level's `TERRAIN_MAX`: a frame whose
    /// `surface_type` reaches it carries a per-texel terrain plane.
    pub fn load<I: io::Read>(input: &mut I, terrain_count: i32) -> Result<Self, Error> {
        let mut signature = [0u8; 3];
        input.read_exact(&mut signature)?;
        if &signature != SIGNATURE {
            return Err(Error::BadSignature(signature));
        }

        let mut raw_name = [0u8; NAME_LEN];
        input.read_exact(&mut raw_name)?;
        let name_len = raw_name.iter().position(|&c| c == 0).unwrap_or(NAME_LEN);
        let name = String::from_utf8_lossy(&raw_name[..name_len]).into_owned();

        let frame_count = input.read_i32::<E>()?;
        let dry_terrain = input.read_i32::<E>()?;
        let impulse = input.read_i32::<E>()?;

        let _ = input.read_u8()?;
        let raw_mode = input.read_u8()?;
        let mode = match raw_mode {
            0 => Mode::Relative,
            1 => Mode::Absolute,
            2 => Mode::Rel2Abs,
            other => return Err(Error::BadMode(other)),
        };
        let _ = input.read_u16::<E>()?;

        let mut key_phases = [0i32; MAX_KEY_PHASE];
        for key in key_phases[1..].iter_mut() {
            *key = input.read_i32::<E>()?;
        }
        let _ = input.read_i32::<E>()?;

        let mut frames = Vec::with_capacity(frame_count.max(0) as usize);
        for _ in 0..frame_count.max(0) {
            frames.push(Frame::load(input, mode, terrain_count)?);
        }

        Ok(MobileLocation {
            name,
            mode,
            dry_terrain,
            impulse,
            key_phases,
            frames,
        })
    }

    /// Largest frame extent, which is what the interpolation scratch buffer
    /// has to be sized for.
    pub fn max_frame_size(&self) -> (i32, i32) {
        self.frames
            .iter()
            .fold((0, 0), |acc, f| (acc.0.max(f.size.0), acc.1.max(f.size.1)))
    }

    /// Total number of quants in one full loop - `maxStage` of the original.
    pub fn max_stage(&self) -> i32 {
        self.frames.iter().map(|f| f.period).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::{rle, Error, MobileLocation, Mode};
    use byteorder::{LittleEndian as E, WriteBytesExt};

    /// Builds a file the way the original `MobileLocation::save` does.
    struct Builder {
        data: Vec<u8>,
        mode: Mode,
    }

    impl Builder {
        fn new(name: &str, mode: Mode, frame_count: i32) -> Self {
            let mut data = Vec::new();
            data.extend_from_slice(b"ML3");
            let mut raw_name = [0u8; super::NAME_LEN];
            raw_name[..name.len()].copy_from_slice(name.as_bytes());
            data.extend_from_slice(&raw_name);
            data.write_i32::<E>(frame_count).unwrap();
            data.write_i32::<E>(3).unwrap(); // DryTerrain
            data.write_i32::<E>(42).unwrap(); // Impulse
            data.write_u8(0).unwrap();
            data.write_u8(mode as u8).unwrap();
            data.write_u16::<E>(0).unwrap();
            for key in 1..4 {
                data.write_i32::<E>(key * 10).unwrap();
            }
            data.write_i32::<E>(0).unwrap();
            Builder { data, mode }
        }

        /// `pack` mirrors the `csd`/`cst` fields being non-zero.
        fn frame(
            mut self,
            size: (i32, i32),
            period: i32,
            surface_type: i32,
            delta: &[u8],
            terrain: Option<&[u8]>,
            signs: &[u32],
            pack: bool,
        ) -> Self {
            let mut packed_delta = Vec::new();
            let mut packed_terrain = Vec::new();
            if pack {
                rle::encode(delta, &mut packed_delta);
                if let Some(t) = terrain {
                    rle::encode(t, &mut packed_terrain);
                }
            }

            self.data.write_i32::<E>(1).unwrap(); // x0
            self.data.write_i32::<E>(2).unwrap(); // y0
            self.data.write_i32::<E>(size.0).unwrap();
            self.data.write_i32::<E>(size.1).unwrap();
            self.data.write_i32::<E>(period).unwrap();
            self.data.write_i32::<E>(surface_type).unwrap();
            self.data.write_i32::<E>(packed_delta.len() as i32).unwrap();
            self.data
                .write_i32::<E>(packed_terrain.len() as i32)
                .unwrap();
            self.data.write_i32::<E>(0).unwrap();
            self.data.write_i32::<E>(0).unwrap();

            if pack {
                self.data.extend_from_slice(&packed_delta);
            } else {
                self.data.extend_from_slice(delta);
            }
            if let Some(t) = terrain {
                if pack {
                    self.data.extend_from_slice(&packed_terrain);
                } else {
                    self.data.extend_from_slice(t);
                }
            }
            if self.mode == Mode::Relative {
                for &word in signs {
                    self.data.write_u32::<E>(word).unwrap();
                }
            }
            self
        }

        fn load(&self, terrain_count: i32) -> Result<MobileLocation, Error> {
            MobileLocation::load(&mut self.data.as_slice(), terrain_count)
        }
    }

    #[test]
    fn header_layout() {
        let ml = Builder::new("carrier", Mode::Relative, 0).load(8).unwrap();
        assert_eq!(ml.name, "carrier");
        assert_eq!(ml.mode, Mode::Relative);
        assert_eq!(ml.dry_terrain, 3);
        assert_eq!(ml.impulse, 42);
        assert_eq!(ml.key_phases, [0, 10, 20, 30]);
        assert!(ml.frames.is_empty());
    }

    #[test]
    fn relative_frame_with_sign_bits() {
        let delta = [1u8, 2, 3, 4, 5, 6];
        // Texels 1 and 4 move downwards.
        let signs = [(1 << 1) | (1 << 4)];
        let ml = Builder::new("lift", Mode::Relative, 1)
            .frame((3, 2), 4, -1, &delta, None, &signs, false)
            .load(8)
            .unwrap();

        let frame = &ml.frames[0];
        assert_eq!(frame.pos, (1, 2));
        assert_eq!(frame.size, (3, 2));
        assert_eq!(frame.period, 4);
        assert_eq!(frame.delta, delta);
        assert!(frame.terrain.is_empty());
        let negative = (0..frame.area())
            .filter(|&i| frame.is_negative(i))
            .collect::<Vec<_>>();
        assert_eq!(negative, [1, 4]);
        assert_eq!(ml.max_stage(), 4);
        assert_eq!(ml.max_frame_size(), (3, 2));
    }

    #[test]
    fn per_texel_terrain_is_read_past_the_count() {
        let delta = [9u8; 4];
        let terrain = [1u8, 2, 3, 4];
        // `surface_type >= terrain_count` is the marker for the extra plane.
        let ml = Builder::new("mud", Mode::Absolute, 1)
            .frame((2, 2), 1, 8, &delta, Some(&terrain), &[], false)
            .load(8)
            .unwrap();
        assert_eq!(ml.frames[0].terrain, terrain);
        // Absolute frames carry no sign bits.
        assert!(ml.frames[0].sign_bits.is_empty());

        // Below the count, the plane is absent and the whole frame shares one
        // terrain type.
        let ml = Builder::new("mud", Mode::Absolute, 1)
            .frame((2, 2), 1, 5, &delta, None, &[], false)
            .load(8)
            .unwrap();
        assert!(ml.frames[0].terrain.is_empty());
        assert_eq!(ml.frames[0].surface_type, 5);
    }

    #[test]
    fn packed_planes_expand_to_the_frame_size() {
        let delta = {
            let mut d = vec![7u8; 40];
            d[13] = 1;
            d
        };
        let terrain = vec![2u8; 40];
        let ml = Builder::new("packed", Mode::Relative, 1)
            .frame((8, 5), 2, 9, &delta, Some(&terrain), &[0, 0], true)
            .load(8)
            .unwrap();
        assert_eq!(ml.frames[0].delta, delta);
        assert_eq!(ml.frames[0].terrain, terrain);
        // `ss = sz/32 + 1`, so 40 texels need two words.
        assert_eq!(ml.frames[0].sign_bits.len(), 2);
    }

    #[test]
    fn multiple_frames_keep_their_periods() {
        let ml = Builder::new("stairs", Mode::Relative, 3)
            .frame((2, 1), 1, -1, &[1, 2], None, &[0], false)
            .frame((2, 1), 5, -1, &[3, 4], None, &[0], false)
            .frame((2, 1), 2, -1, &[5, 6], None, &[0], false)
            .load(8)
            .unwrap();
        assert_eq!(
            ml.frames.iter().map(|f| f.period).collect::<Vec<_>>(),
            [1, 5, 2]
        );
        assert_eq!(ml.max_stage(), 8);
    }

    #[test]
    fn period_is_clamped_up() {
        let ml = Builder::new("zero", Mode::Relative, 1)
            .frame((1, 1), 0, -1, &[1], None, &[0], false)
            .load(8)
            .unwrap();
        assert_eq!(ml.frames[0].period, 1);
    }

    #[test]
    fn bad_signature_is_rejected() {
        let mut builder = Builder::new("nope", Mode::Relative, 0);
        builder.data[0] = b'X';
        assert!(matches!(builder.load(8), Err(Error::BadSignature(_))));
    }

    #[test]
    fn bad_mode_is_rejected() {
        let mut builder = Builder::new("nope", Mode::Relative, 0);
        // The mode byte follows the signature, the name and three ints.
        builder.data[3 + super::NAME_LEN + 3 * 4 + 1] = 7;
        assert!(matches!(builder.load(8), Err(Error::BadMode(7))));
    }

    #[test]
    fn truncated_file_is_rejected() {
        let mut builder = Builder::new("cut", Mode::Relative, 1).frame(
            (4, 4),
            1,
            -1,
            &[1; 16],
            None,
            &[0],
            false,
        );
        builder.data.truncate(builder.data.len() - 8);
        assert!(matches!(builder.load(8), Err(Error::Io(_))));
    }
}
