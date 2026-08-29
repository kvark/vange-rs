//! CPU snapshot of the shop overlay, so layout can be iterated without
//! booting the game. Library tests write `target/shop-preview.png`.

use egui::epaint::{ClippedPrimitive, ImageDelta, Primitive, Vertex};
use egui::{ColorImage, ImageData, TextureId};
use std::collections::HashMap;
use std::io::BufWriter;
use std::path::Path;

struct Atlas {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

impl Atlas {
    fn color(image: &ColorImage) -> Self {
        let mut rgba = Vec::with_capacity(image.pixels.len() * 4);
        for c in &image.pixels {
            rgba.extend_from_slice(&c.to_array());
        }
        Atlas {
            width: image.width(),
            height: image.height(),
            rgba,
        }
    }

    fn sample(&self, uv: [f32; 2]) -> [f32; 4] {
        if self.width == 0 || self.height == 0 {
            return [1.0, 1.0, 1.0, 1.0];
        }
        let x = (uv[0] * self.width as f32).clamp(0.0, (self.width - 1) as f32) as usize;
        let y = (uv[1] * self.height as f32).clamp(0.0, (self.height - 1) as f32) as usize;
        let i = (y * self.width + x) * 4;
        [
            self.rgba[i] as f32 / 255.0,
            self.rgba[i + 1] as f32 / 255.0,
            self.rgba[i + 2] as f32 / 255.0,
            self.rgba[i + 3] as f32 / 255.0,
        ]
    }

    fn patch_color(&mut self, img: &ColorImage, pos: Option<[usize; 2]>) {
        let (ox, oy) = pos.map(|p| (p[0], p[1])).unwrap_or((0, 0));
        for y in 0..img.height() {
            for x in 0..img.width() {
                let c = img.pixels[y * img.width() + x].to_array();
                let dx = ox + x;
                let dy = oy + y;
                if dx < self.width && dy < self.height {
                    let i = (dy * self.width + dx) * 4;
                    self.rgba[i..i + 4].copy_from_slice(&c);
                }
            }
        }
    }
}

fn apply_delta(textures: &mut HashMap<TextureId, Atlas>, id: TextureId, delta: &ImageDelta) {
    #[allow(clippy::infallible_destructuring_match, clippy::pattern_type_mismatch)]
    let img = match &delta.image {
        ImageData::Color(img) => img.as_ref(),
    };
    if let Some(pos) = delta.pos
        && let Some(atlas) = textures.get_mut(&id)
    {
        atlas.patch_color(img, Some(pos));
        return;
    }
    textures.insert(id, Atlas::color(img));
}

fn blend(dst: &mut [u8], src_premul: [f32; 4]) {
    let sa = src_premul[3].clamp(0.0, 1.0);
    let da = dst[3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        dst.copy_from_slice(&[0, 0, 0, 0]);
        return;
    }
    for c in 0..3 {
        let s = src_premul[c];
        let d = dst[c] as f32 / 255.0 * da;
        dst[c] = ((s + d * (1.0 - sa)) / out_a * 255.0).clamp(0.0, 255.0) as u8;
    }
    dst[3] = (out_a * 255.0).clamp(0.0, 255.0) as u8;
}

fn vertex_rgba(v: Vertex) -> [f32; 4] {
    let c = v.color.to_array();
    [
        c[0] as f32 / 255.0,
        c[1] as f32 / 255.0,
        c[2] as f32 / 255.0,
        c[3] as f32 / 255.0,
    ]
}

fn raster_triangle(
    pixels: &mut [u8],
    width: i32,
    height: i32,
    clip: egui::Rect,
    verts: [Vertex; 3],
    atlas: Option<&Atlas>,
) {
    let pts = verts.map(|v| v.pos);
    let min_x = pts[0]
        .x
        .min(pts[1].x)
        .min(pts[2].x)
        .max(clip.left())
        .max(0.0);
    let max_x = pts[0]
        .x
        .max(pts[1].x)
        .max(pts[2].x)
        .min(clip.right())
        .min(width as f32);
    let min_y = pts[0]
        .y
        .min(pts[1].y)
        .min(pts[2].y)
        .max(clip.top())
        .max(0.0);
    let max_y = pts[0]
        .y
        .max(pts[1].y)
        .max(pts[2].y)
        .min(clip.bottom())
        .min(height as f32);
    if min_x >= max_x || min_y >= max_y {
        return;
    }
    let area = (pts[1].x - pts[0].x) * (pts[2].y - pts[0].y)
        - (pts[2].x - pts[0].x) * (pts[1].y - pts[0].y);
    if area.abs() < 0.25 {
        return;
    }
    let x0 = min_x.floor() as i32;
    let x1 = max_x.ceil() as i32;
    let y0 = min_y.floor() as i32;
    let y1 = max_y.ceil() as i32;
    let colors = verts.map(vertex_rgba);
    for y in y0..y1 {
        for x in x0..x1 {
            if x < 0 || y < 0 || x >= width || y >= height {
                continue;
            }
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let w0 = ((pts[1].x - px) * (pts[2].y - py) - (pts[2].x - px) * (pts[1].y - py)) / area;
            let w1 = ((pts[2].x - px) * (pts[0].y - py) - (pts[0].x - px) * (pts[2].y - py)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let mut src = [
                colors[0][0] * w0 + colors[1][0] * w1 + colors[2][0] * w2,
                colors[0][1] * w0 + colors[1][1] * w1 + colors[2][1] * w2,
                colors[0][2] * w0 + colors[1][2] * w1 + colors[2][2] * w2,
                colors[0][3] * w0 + colors[1][3] * w1 + colors[2][3] * w2,
            ];
            if let Some(atlas) = atlas {
                let uv = [
                    verts[0].uv[0] * w0 + verts[1].uv[0] * w1 + verts[2].uv[0] * w2,
                    verts[0].uv[1] * w0 + verts[1].uv[1] * w1 + verts[2].uv[1] * w2,
                ];
                let texel = atlas.sample(uv);
                src[0] *= texel[0];
                src[1] *= texel[1];
                src[2] *= texel[2];
                src[3] *= texel[3];
            }
            if src[3] <= 0.001 {
                continue;
            }
            let i = ((y * width + x) * 4) as usize;
            blend(&mut pixels[i..i + 4], src);
        }
    }
}

/// Paint tessellated egui output into an RGBA buffer.
pub fn rasterize(
    width: u32,
    height: u32,
    output: &egui::FullOutput,
    primitives: &[ClippedPrimitive],
) -> Vec<u8> {
    let mut textures = HashMap::new();
    for pair in &output.textures_delta.set {
        apply_delta(&mut textures, pair.0, &pair.1);
    }
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    for prim in primitives {
        let mesh = match prim.primitive {
            Primitive::Mesh(ref mesh) => mesh,
            Primitive::Callback(_) => continue,
        };
        let atlas = textures.get(&mesh.texture_id);
        for tri in mesh.indices.as_chunks::<3>().0 {
            let verts = [
                mesh.vertices[tri[0] as usize],
                mesh.vertices[tri[1] as usize],
                mesh.vertices[tri[2] as usize],
            ];
            raster_triangle(
                &mut pixels,
                width as i32,
                height as i32,
                prim.clip_rect,
                verts,
                atlas,
            );
        }
    }
    pixels
}

pub fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(rgba)?;
    Ok(())
}
