use byteorder::{LittleEndian as E, ReadBytesExt};

const SIGNATURE: &[u8; 3] = b"ML3"; //MLSign
const NAME_LEN: usize = 16; //MLNAMELEN + 1
const MAX_KEYPHRASE: usize = 4; //MAX_KEYPHASE

#[derive(Clone, Copy, Debug)]
pub enum Mode {
    Relative = 0,
    Absolute = 1,
    Rel2Abs = 2,
}

pub struct Frame {
    pos: (i32, i32),
    size: (u32, u32),
    period: u32,
    surf_type: u32,
    csd: u32,
    cst: u32,
    delta: Vec<u8>,
    terrain: Vec<u8>,
    sign_bits: Vec<u32>,
}

impl Frame {
    pub fn load<I: ReadBytesExt>(
        input: &mut I,
        mode: Mode,
        max_suface_type: u32,
        mut temp: &mut Vec<u8>,
    ) -> Self {
        let x0 = input.read_i32::<E>().unwrap();
        let y0 = input.read_i32::<E>().unwrap();
        let sx = input.read_u32::<E>().unwrap();
        let sy = input.read_u32::<E>().unwrap();
        let period = input.read_u32::<E>().unwrap();
        let surf_type = input.read_u32::<E>().unwrap();
        let csd = input.read_u32::<E>().unwrap();
        let cst = input.read_u32::<E>().unwrap();
        let _ = input.read_u32::<E>();
        let _ = input.read_u32::<E>();
        let total = (sx * sy) as usize;

        temp.clear();
        temp.resize(total, 0u8);

        let delta = if csd == 0 {
            input.read(&mut temp).unwrap();
            let mut d = Vec::new();
            rle::decode(&temp, &mut d);
            d
        } else {
            let mut d = vec![0u8; csd as usize];
            input.read(&mut d).unwrap();
            d
        };

        let terrain = if surf_type >= max_suface_type {
            if cst == 0 {
                input.read(&mut temp).unwrap();
                let mut t = Vec::new();
                rle::decode(&temp, &mut t);
                t
            } else {
                let mut t = vec![0u8; cst as usize];
                input.read(&mut t).unwrap();
                t
            }
        } else {
            Vec::new()
        };

        let sign_bits = match mode {
            Mode::Relative => {
                let words = total / 32 + 1;
                let mut sb = Vec::with_capacity(words);
                for _ in 0..words {
                    sb.push(input.read_u32::<E>().unwrap());
                }
                sb
            }
            _ => Vec::new(),
        };

        Frame {
            pos: (x0, y0),
            size: (sx, sy),
            period,
            surf_type,
            csd,
            cst,
            delta,
            terrain,
            sign_bits,
        }
    }
}

pub struct MobileLocation {
    pub max_stage: u32,
    pub frames: Vec<Frame>,
}

impl MobileLocation {
    pub fn load<I: ReadBytesExt>(input: &mut I, max_surface: u32) -> Self {
        let mut signature = [0u8; 3];
        input.read(&mut signature).unwrap();
        assert_eq!(&signature, SIGNATURE);

        let mut raw_name = [0u8; NAME_LEN];
        input.read(&mut raw_name).unwrap();

        let max_frame = input.read_u32::<E>().unwrap();
        let _dry_terrain = input.read_u32::<E>().unwrap();
        let _impulse = input.read_u32::<E>().unwrap();

        let _ = input.read_u8();
        let mode = match input.read_u8().unwrap() {
            0 => Mode::Relative,
            1 => Mode::Absolute,
            2 => Mode::Rel2Abs,
            other => panic!("Unexpected mode {}", other),
        };
        let _ = input.read_u8();
        let _ = input.read_u8();

        let mut keyphrase = [0u32; MAX_KEYPHRASE];
        for key in keyphrase[1..].iter_mut() {
            *key = input.read_u32::<E>().unwrap();
        }
        let _ = input.read_u32::<E>();

        let mut is_alt = false;
        let mut max_stage = 0;
        let mut alt_size = (0, 0);
        let mut frames = Vec::with_capacity(max_frame as usize);
        let mut temp = Vec::new();
        for _ in 0..max_frame {
            let frame = Frame::load(input, mode, max_surface, &mut temp);
            alt_size.0 = alt_size.0.max(frame.size.0);
            alt_size.1 = alt_size.1.max(frame.size.1);
            is_alt |= frame.period > 1;
            max_stage += frame.period;
            frames.push(frame);
        }
        let _ = is_alt;

        MobileLocation { max_stage, frames }
    }
}
