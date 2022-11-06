use vangers::{config, level, model, space};

use cgmath::prelude::*;

#[derive(Debug)]
pub struct CollisionPoint {
    pub pos: cgmath::Vector3<f32>,
    pub depth: f32,
}

#[derive(Debug)]
pub struct CollisionData {
    pub soft: Option<CollisionPoint>,
    pub hard_dominant: bool,
    pub hard: Option<CollisionPoint>,
}

struct HitAccumulator {
    pos: cgmath::Vector3<f32>,
    depth: f32,
    count: f32,
}

impl HitAccumulator {
    fn new() -> Self {
        HitAccumulator {
            pos: cgmath::vec3(0.0, 0.0, 0.0),
            depth: 0.0,
            count: 0.0,
        }
    }
    fn add(&mut self, pos: cgmath::Vector3<f32>, depth: f32) {
        self.pos += pos;
        self.depth += depth;
        self.count += 1.0;
    }
    fn finish(&self, min: f32) -> Option<CollisionPoint> {
        if self.count > min {
            Some(CollisionPoint {
                pos: self.pos / self.count,
                depth: self.depth / self.count,
            })
        } else {
            None
        }
    }
}

pub fn get_distance_to_terrain(level: &level::Level, point: cgmath::Point3<f32>) -> f32 {
    point.z
        - match level.get((point.x as i32, point.y as i32)) {
            level::Texel::Single(p) => p.0,
            level::Texel::Dual { high, low, mid } => {
                if point.z > mid {
                    high.0
                } else {
                    low.0
                }
            }
        }
}

impl CollisionData {
    pub fn collide_low(
        poly: &model::Polygon,
        samples: &[model::RawVertex],
        scale: f32,
        transform: &space::Transform,
        level: &level::Level,
        terraconf: &config::common::Terrain,
    ) -> Self {
        let (mut soft, mut hard) = (HitAccumulator::new(), HitAccumulator::new());
        for s in samples[poly.samples.clone()].iter() {
            let sp = cgmath::Point3::from(*s).cast::<f32>().unwrap();
            let pos = transform.transform_point(sp * scale).to_vec();
            let texel = level.get((pos.x as i32, pos.y as i32));
            let height = match texel {
                level::Texel::Single(point) => point.0,
                level::Texel::Dual { high, low, mid } => {
                    if pos.z > mid {
                        if pos.z - mid > high.0 - pos.z {
                            high.0
                        } else {
                            continue;
                        }
                    } else {
                        low.0
                    }
                }
            };
            let dz = height - pos.z;
            log::trace!("\t\t\tSample h={:?} at {:?}, dz={}", height, pos, dz);
            if dz > terraconf.min_wall_delta {
                //log::debug!("\t\t\tHard touch of {} at {:?}", dz, pos);
                hard.add(pos, dz);
            } else if dz > 0.0 {
                //log::debug!("\t\t\tSoft touch of {} at {:?}", dz, pos);
                soft.add(pos, dz);
            }
        }

        let total = (poly.samples.end - poly.samples.start) as f32;
        // This is tricky: original code was doing pixel collisions and had
        // a hard-coded constants of 4 pixels to be the threshold.
        // See `VariablePolygon::lower_average` implementation.
        let threshold = 0.05 * total;
        CollisionData {
            soft: (if soft.count > 0.0 { &soft } else { &hard }).finish(0.0),
            hard_dominant: hard.count * 2.0 >= total,
            hard: hard.finish(threshold),
        }
    }
}
