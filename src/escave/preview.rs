//! Rotating item mesh for the shop, plus original names and comments.
//!
//! The 1998 preview was an AVI of a spinning model plus a text AVI.
//! Names and comments come from `actint/items.inc`; the mesh is the
//! `game.lst` `.m3d`, turned on a turntable each frame.

use glam::{Quat, Vec3};
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
        }
    }

    pub fn load_reader<R: std::io::Read>(mut reader: R) -> Self {
        let model = m3d::FullModel::load(&mut reader);
        Self::from_draw(&model.body)
    }

    pub fn load_path(path: &Path) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let model = m3d::FullModel::load(std::io::Cursor::new(&bytes));
            Self::from_draw(&model.body)
        }))
        .ok()
    }

    /// Turntable: Z-up, spin around Z, slight nod, Lambert faces.
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
        let rot = Quat::from_rotation_z(angle) * Quat::from_rotation_x(-0.55);
        let light = Vec3::new(0.35, -0.8, 0.45).normalize();
        let dist = self.radius * 2.6;
        let cx = rect.center().x;
        let cy = rect.center().y;
        let scale = rect.height().min(rect.width()) * 0.42 / self.radius;
        let mut order: Vec<(f32, usize)> = self
            .faces
            .iter()
            .enumerate()
            .filter_map(|(i, face)| {
                let n = rot * Vec3::from(face.normal);
                if n.y > 0.2 {
                    return None;
                }
                let a = rot * Vec3::from(self.positions[face.verts[0]]);
                let b = rot * Vec3::from(self.positions[face.verts[1]]);
                let c = rot * Vec3::from(self.positions[face.verts[2]]);
                Some(((a.y + b.y + c.y) / 3.0, i))
            })
            .collect();
        order.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        for &(_, i) in &order {
            let face = &self.faces[i];
            let n = (rot * Vec3::from(face.normal)).normalize_or(Vec3::Z);
            let lambert = (0.22 + 0.78 * n.dot(light).clamp(0.0, 1.0)).clamp(0.0, 1.0);
            let fill = egui::Color32::from_rgba_unmultiplied(
                (color.r() as f32 * lambert) as u8,
                (color.g() as f32 * lambert) as u8,
                (color.b() as f32 * lambert) as u8,
                255,
            );
            let pts = face.verts.map(|vi| {
                let p = rot * Vec3::from(self.positions[vi]);
                let z = (p.y + dist).max(1.0);
                let px = cx + p.x * scale * dist / z;
                let py = cy - p.z * scale * dist / z;
                egui::pos2(px, py)
            });
            painter.add(egui::Shape::convex_polygon(
                pts.to_vec(),
                fill,
                egui::Stroke::NONE,
            ));
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
}
