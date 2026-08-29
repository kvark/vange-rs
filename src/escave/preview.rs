//! Rotating item mesh for the shop, plus original names and comments.
//!
//! The 1998 preview was an AVI of a spinning model plus a text AVI.
//! Names and comments come from `actint/items.inc`; the mesh is the
//! `game.lst` `.m3d`, turned on a turntable each frame.

use glam::{Quat, Vec3};
use std::cell::RefCell;
use std::path::Path;

/// English names from `ITM*_NAME1`. Ids that already match stay as-is.
pub fn display_name(id: &str) -> &str {
    match id {
        "LightLaser" => "MacHOTine Gun",
        "LightMissile" => "Speetle System",
        "HeavyLaser" => "MacHOTine Gun",
        "HeavyMissile" => "Speetle System",
        other => other,
    }
}

/// `game.lst` NameID for a shop id.
pub fn mesh_id(id: &str) -> &str {
    match id {
        "Nymbos" => "Eggs",
        "Phlegma" => "Slime",
        "Shrub" => "Shurub",
        "Poponka" => "ClayTabl",
        "Toxick" => "Toxic",
        other => other,
    }
}

/// `ITM*_COMMENTS1` from `items.inc`. Weapon prompts are appended.
pub fn description_for(id: &str) -> &'static str {
    match id {
        "Nymbos" => "Some eleepods' stuff from Podish",
        "Phlegma" => "Some eleepods' stuff from Incubator",
        "Heroin" => "Some beeboorats' stuff from VigBoo",
        "Shrub" => "Some beeboorats' stuff from VigBoo",
        "Poponka" => "Some zeexens' stuff from ZeePa",
        "Toxick" => "Some zeexens' stuff from B-Zone",
        "LightLaser" => "Softie's weapon (light model). Put in an arms slot to use",
        "HeavyLaser" => "Softie's weapon (heavy model). Put in an arms slot to use",
        "LightMissile" => "Softie's weapon (light model). Put in an arms slot to use",
        "HeavyMissile" => "Softie's weapon (heavy model). Put in an arms slot to use",
        _ => "A trade good from the belts.",
    }
}

struct Face {
    verts: [usize; 3],
    normal: [f32; 3],
}

/// CPU triangles of an item `.m3d`, for a turntable in the shop.
pub struct SpinMesh {
    positions: Vec<[f32; 3]>,
    faces: Vec<Face>,
    radius: f32,
    /// Last raster, kept alive so egui does not free the texture mid-frame.
    tex: RefCell<Option<egui::TextureHandle>>,
}

impl SpinMesh {
    pub fn from_draw(mesh: &m3d::DrawMesh) -> Self {
        let geo = &mesh.geometry;
        let positions = geo
            .positions
            .iter()
            .map(|p| [p[0] as f32, p[1] as f32, p[2] as f32])
            .collect::<Vec<_>>();
        let faces = geo
            .polygons
            .iter()
            .map(|poly| Face {
                verts: [
                    poly.vertices[0].pos as usize,
                    poly.vertices[1].pos as usize,
                    poly.vertices[2].pos as usize,
                ],
                normal: [
                    poly.flat_normal[0] as f32 / m3d::NORMALIZER,
                    poly.flat_normal[1] as f32 / m3d::NORMALIZER,
                    poly.flat_normal[2] as f32 / m3d::NORMALIZER,
                ],
            })
            .filter(|f| {
                f.verts[0] < positions.len()
                    && f.verts[1] < positions.len()
                    && f.verts[2] < positions.len()
            })
            .collect();
        let radius = (mesh.max_radius as f32).max(1.0);
        SpinMesh {
            positions,
            faces,
            radius,
            tex: RefCell::new(None),
        }
    }

    pub fn load_bytes(bytes: &[u8]) -> Option<Self> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let model = m3d::FullModel::load(std::io::Cursor::new(bytes));
            Self::from_draw(&model.body)
        }))
        .ok()
    }

    pub fn load_path(path: &Path) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        Self::load_bytes(&bytes)
    }

    /// One face pointing at the camera, for shop-preview tests.
    #[cfg(test)]
    pub fn triangle() -> Self {
        SpinMesh {
            positions: vec![[-8.0, 0.0, -8.0], [8.0, 0.0, -8.0], [0.0, 0.0, 8.0]],
            faces: vec![Face {
                verts: [0, 1, 2],
                normal: [0.0, -1.0, 0.0],
            }],
            radius: 12.0,
            tex: RefCell::new(None),
        }
    }

    /// Turntable: Z-up, spin around Z, look down a bit, Lambert faces.
    ///
    /// egui's painter has no depth attachment, so this is a software
    /// z-buffer (closer pixel wins) rather than GPU `depth_compare`.
    /// Triangle order does not matter.
    pub fn paint(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        angle: f32,
        color: egui::Color32,
    ) {
        if self.positions.is_empty() || rect.width() < 8.0 || rect.height() < 8.0 {
            return;
        }
        let w = rect.width().round().max(1.0) as i32;
        let h = rect.height().round().max(1.0) as i32;
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        let mut zbuf = vec![f32::INFINITY; (w * h) as usize];
        let rot = Quat::from_rotation_z(angle) * Quat::from_rotation_x(-0.85);
        let light = Vec3::new(0.35, -0.8, 0.45).normalize();
        let dist = self.radius * 2.8;
        let scale = rect.height().min(rect.width()) * 0.46 / self.radius;
        let cx = w as f32 * 0.5;
        let cy = h as f32 * 0.5;
        for face in &self.faces {
            let n = rot * Vec3::from(face.normal);
            if n.y >= 0.0 {
                continue;
            }
            let n = n.normalize_or(Vec3::NEG_Y);
            let lambert = (0.22 + 0.78 * n.dot(light).clamp(0.0, 1.0)).clamp(0.0, 1.0);
            let fill = [
                (color.r() as f32 * lambert) as u8,
                (color.g() as f32 * lambert) as u8,
                (color.b() as f32 * lambert) as u8,
                255,
            ];
            let pts = face.verts.map(|vi| {
                let p = rot * Vec3::from(self.positions[vi]);
                let z = (p.y + dist).max(1.0);
                let px = cx + p.x * scale * dist / z;
                let py = cy - p.z * scale * dist / z;
                [px, py, z]
            });
            raster_ztest(&mut pixels, &mut zbuf, w, h, pts, fill);
        }
        let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &pixels);
        let options = egui::TextureOptions::LINEAR;
        let tex_id = {
            let mut slot = self.tex.borrow_mut();
            match slot.as_mut() {
                Some(handle) => {
                    handle.set(image, options);
                    handle.id()
                }
                None => {
                    let handle = painter.ctx().load_texture("shop-spin", image, options);
                    let id = handle.id();
                    *slot = Some(handle);
                    id
                }
            }
        };
        painter.image(
            tex_id,
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }
}

fn raster_ztest(
    pixels: &mut [u8],
    zbuf: &mut [f32],
    width: i32,
    height: i32,
    pts: [[f32; 3]; 3],
    fill: [u8; 4],
) {
    let area = (pts[1][0] - pts[0][0]) * (pts[2][1] - pts[0][1])
        - (pts[2][0] - pts[0][0]) * (pts[1][1] - pts[0][1]);
    if area.abs() < 0.5 {
        return;
    }
    let min_x = pts[0][0].min(pts[1][0]).min(pts[2][0]).max(0.0);
    let max_x = pts[0][0].max(pts[1][0]).max(pts[2][0]).min(width as f32);
    let min_y = pts[0][1].min(pts[1][1]).min(pts[2][1]).max(0.0);
    let max_y = pts[0][1].max(pts[1][1]).max(pts[2][1]).min(height as f32);
    if min_x >= max_x || min_y >= max_y {
        return;
    }
    let x0 = min_x.floor() as i32;
    let x1 = max_x.ceil() as i32;
    let y0 = min_y.floor() as i32;
    let y1 = max_y.ceil() as i32;
    for y in y0..y1 {
        for x in x0..x1 {
            if x < 0 || y < 0 || x >= width || y >= height {
                continue;
            }
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let w0 =
                ((pts[1][0] - px) * (pts[2][1] - py) - (pts[2][0] - px) * (pts[1][1] - py)) / area;
            let w1 =
                ((pts[2][0] - px) * (pts[0][1] - py) - (pts[0][0] - px) * (pts[2][1] - py)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let z = pts[0][2] * w0 + pts[1][2] * w1 + pts[2][2] * w2;
            let i = (y * width + x) as usize;
            if z >= zbuf[i] {
                continue;
            }
            zbuf[i] = z;
            let o = i * 4;
            pixels[o..o + 4].copy_from_slice(&fill);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_names_and_comments() {
        assert_eq!(display_name("Nymbos"), "Nymbos");
        assert_eq!(display_name("LightLaser"), "MacHOTine Gun");
        assert_eq!(display_name("LightMissile"), "Speetle System");
        assert_eq!(mesh_id("Nymbos"), "Eggs");
        assert_eq!(mesh_id("Phlegma"), "Slime");
        assert_eq!(mesh_id("LightLaser"), "LightLaser");
        assert_eq!(
            description_for("Nymbos"),
            "Some eleepods' stuff from Podish"
        );
        assert!(description_for("LightLaser").contains("arms slot"));
    }

    fn viewport_input() -> egui::RawInput {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 300.0));
        egui::RawInput {
            screen_rect: Some(rect),
            ..egui::RawInput::default()
        }
    }

    fn painted_images(output: &egui::FullOutput) -> usize {
        output
            .shapes
            .iter()
            .filter(|clipped| matches!(clipped.shape, egui::Shape::Mesh(_)))
            .count()
    }

    #[test]
    fn the_turntable_paints_faces() {
        let mesh = SpinMesh::triangle();
        let ctx = egui::Context::default();
        #[expect(deprecated)]
        let output = ctx.run(viewport_input(), |ctx| {
            mesh.paint(
                &ctx.layer_painter(egui::LayerId::background()),
                ctx.content_rect(),
                0.0,
                egui::Color32::from_rgb(200, 140, 48),
            );
        });
        assert!(
            painted_images(&output) > 0,
            "a visible face must hit the painter"
        );
    }

    #[test]
    fn a_near_face_wins_the_z_buffer() {
        let mut pixels = vec![0u8; 64 * 64 * 4];
        let mut zbuf = vec![f32::INFINITY; 64 * 64];
        let near = [[20.0, 20.0, 2.0], [44.0, 20.0, 2.0], [32.0, 44.0, 2.0]];
        let far = [[8.0, 8.0, 9.0], [56.0, 8.0, 9.0], [32.0, 56.0, 9.0]];
        raster_ztest(&mut pixels, &mut zbuf, 64, 64, far, [10, 10, 10, 255]);
        raster_ztest(&mut pixels, &mut zbuf, 64, 64, near, [200, 140, 48, 255]);
        let i = (32 * 64 + 32) * 4;
        assert_eq!(&pixels[i..i + 3], &[200, 140, 48]);
    }
}
