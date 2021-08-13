#![deny(
    trivial_casts,
    trivial_numeric_casts,
    unused,
    unused_qualifications,
    rust_2018_compatibility,
    rust_2018_idioms,
    future_incompatible,
    nonstandard_style,
    missing_copy_implementations
)]
#![allow(missing_debug_implementations, clippy::new_without_default)]

use byteorder::{LittleEndian as E, WriteBytesExt};

use std::io::{Result as IoResult, Seek, SeekFrom};

const TY_ASCII: u16 = 2;
const TY_SHORT: u16 = 3;
const TY_LONG: u16 = 4;
const TAG_IMAGE_WIDTH: u16 = 0x100;
const TAG_IMAGE_LENGTH: u16 = 0x101;
const TAG_BITS_PER_SAMPLE: u16 = 0x102;
const TAG_IMAGE_DESCRIPTION: u16 = 0x10E;
const TAG_STRIP_OFFSETS: u16 = 0x111;
const TAG_ROWS_PER_STRIP: u16 = 0x116;
const TAG_STRIP_BYTE_COUNTS: u16 = 0x117;

struct Field {
    tag: u16,
    ty: u16,
    count: u32,
    value: u32,
}

pub struct Image<'a> {
    pub width: u32,
    pub height: u32,
    pub bpp: u16,
    pub name: &'a str,
    pub data: &'a [u8],
}

pub fn save<W: Seek + WriteBytesExt>(mut tiff: W, images: &[Image<'_>]) -> IoResult<()> {
    // header
    tiff.write_u16::<E>(0x4949)?; // little endian
    tiff.write_u16::<E>(42)?; // magic
    let data_start = (images.len() * 0x80) as u32;
    let mut data_offset = data_start;
    let mut cur_offset = 4;
    // image file directory
    for im in images {
        tiff.write_u32::<E>(cur_offset + 4)?; // IFD offset
        let total_bytes = (im.width * im.height) as usize * im.bpp as usize / 8;
        assert_eq!(total_bytes, im.data.len());
        let description: u32 = im
            .name
            .chars()
            .take(3)
            .enumerate()
            .map(|(i, c)| (c as u32) << (i * 8))
            .sum();
        let fields = [
            Field {
                tag: TAG_IMAGE_WIDTH,
                ty: TY_LONG,
                count: 1,
                value: im.width,
            },
            Field {
                tag: TAG_IMAGE_LENGTH,
                ty: TY_LONG,
                count: 1,
                value: im.height,
            },
            Field {
                tag: TAG_BITS_PER_SAMPLE,
                ty: TY_SHORT,
                count: 1,
                value: im.bpp as u32,
            },
            Field {
                tag: TAG_IMAGE_DESCRIPTION,
                ty: TY_ASCII,
                count: im.name.len().min(3) as u32 + 1,
                value: description,
            },
            Field {
                tag: TAG_STRIP_OFFSETS,
                ty: TY_LONG,
                count: 1,
                value: data_offset,
            },
            Field {
                tag: TAG_ROWS_PER_STRIP,
                ty: TY_SHORT,
                count: 1,
                value: im.height,
            },
            Field {
                tag: TAG_STRIP_BYTE_COUNTS,
                ty: TY_LONG,
                count: 1,
                value: total_bytes as u32,
            },
        ];
        tiff.write_u16::<E>(fields.len() as u16)?;
        for &Field {
            tag,
            ty,
            count,
            value,
        } in &fields
        {
            tiff.write_u16::<E>(tag)?;
            tiff.write_u16::<E>(ty)?;
            tiff.write_u32::<E>(count)?;
            tiff.write_u32::<E>(value)?;
        }
        cur_offset += 4 + 2 + fields.len() as u32 * 12;
        data_offset += total_bytes as u32;
        assert_eq!(tiff.seek(SeekFrom::Current(0)).unwrap(), cur_offset as u64);
    }
    // gap
    assert!(cur_offset < data_start);
    tiff.write_u32::<E>(0)?; // next IFD offset
    for _ in cur_offset + 4..data_start {
        tiff.write_u8(0)?;
    }
    // image data
    for im in images {
        tiff.write_all(im.data)?;
    }
    Ok(())
}
