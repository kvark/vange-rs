//! Extract one scene row from a full `compare-terrain.py` grid.
//!
//! The source grids are intentionally publication-resolution and very large.
//! This decodes one grid once, downsamples its six method cells, and writes
//! compact PNGs referenced by `paper/figures/teaser.svg`.

use std::{
    env,
    fs::{self, File},
    io::{BufReader, BufWriter},
    path::Path,
};

const METHODS: usize = 6;
const CELL_WIDTH: usize = 1280;
const CELL_HEIGHT: usize = 800;
const PAD: usize = 5;
const HEADER: usize = 30;
const LABEL: usize = 18;
const OUT_WIDTH: usize = 448;
const OUT_HEIGHT: usize = 280;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let input = args
        .next()
        .unwrap_or_else(|| "remote/cmp-k6.png".to_owned());
    let output = args.next().unwrap_or_else(|| "paper/figures".to_owned());
    let row: usize = args.next().as_deref().unwrap_or("1").parse()?;

    let decoder = png::Decoder::new(BufReader::new(File::open(&input)?));
    let mut reader = decoder.read_info()?;
    let mut pixels = vec![0; reader.output_buffer_size().ok_or("PNG is too large")?];
    let frame = reader.next_frame(&mut pixels)?;
    if frame.color_type != png::ColorType::Rgb || frame.bit_depth != png::BitDepth::Eight {
        return Err("comparison grid must be 8-bit RGB".into());
    }
    let width = frame.width as usize;
    let height = frame.height as usize;
    let expected_width = METHODS * (CELL_WIDTH + PAD) + PAD;
    let cell_y = HEADER + row * (CELL_HEIGHT + LABEL + PAD) + LABEL;
    if width != expected_width || cell_y + CELL_HEIGHT > height {
        return Err(format!(
            "unexpected grid geometry {width}x{height}; expected width {expected_width} and row {row}"
        )
        .into());
    }
    pixels.truncate(frame.buffer_size());
    fs::create_dir_all(&output)?;

    for method in 0..METHODS {
        let cell_x = PAD + method * (CELL_WIDTH + PAD);
        let mut downsampled = vec![0u8; OUT_WIDTH * OUT_HEIGHT * 3];
        for out_y in 0..OUT_HEIGHT {
            let source_y0 = cell_y + out_y * CELL_HEIGHT / OUT_HEIGHT;
            let source_y1 = cell_y + (out_y + 1) * CELL_HEIGHT / OUT_HEIGHT;
            for out_x in 0..OUT_WIDTH {
                let source_x0 = cell_x + out_x * CELL_WIDTH / OUT_WIDTH;
                let source_x1 = cell_x + (out_x + 1) * CELL_WIDTH / OUT_WIDTH;
                let mut sum = [0u32; 3];
                let mut count = 0u32;
                for source_y in source_y0..source_y1 {
                    for source_x in source_x0..source_x1 {
                        let offset = 3 * (source_y * width + source_x);
                        for channel in 0..3 {
                            sum[channel] += pixels[offset + channel] as u32;
                        }
                        count += 1;
                    }
                }
                let offset = 3 * (out_y * OUT_WIDTH + out_x);
                for channel in 0..3 {
                    downsampled[offset + channel] = (sum[channel] / count) as u8;
                }
            }
        }

        let path = Path::new(&output).join(format!("teaser-{method}.png"));
        let mut encoder = png::Encoder::new(
            BufWriter::new(File::create(path)?),
            OUT_WIDTH as u32,
            OUT_HEIGHT as u32,
        );
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Balanced);
        encoder.write_header()?.write_image_data(&downsampled)?;
    }

    Ok(())
}
