use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use std::ops::Range;

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Transform {
    pub scale: f32,
    pub rot: Quat,
    pub disp: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    pub const IDENTITY: Self = Transform {
        scale: 1.0,
        rot: Quat::IDENTITY,
        disp: Vec3::ZERO,
    };

    /// Hard-snap `|Δpos|` for 20 Hz `WorldState` (world units).
    ///
    /// Open-loop 20-vs-60 Hz driving stays well under 1 unit per tick; a
    /// mechos is ~20 units long. Contact jitter blends; spawn / teleport snap.
    pub const WORLD_STATE_SNAP_POS: f32 = 8.0;
    /// Optional rotation gate (radians, ~29°). Small contact tilt blends;
    /// a flipped / desynced pose still snaps.
    pub const WORLD_STATE_SNAP_ROT: f32 = 0.5;
    /// Blend factor per 20 Hz snapshot (~76% correction in 200 ms).
    pub const WORLD_STATE_BLEND: f32 = 0.25;

    pub fn concat(&self, other: &Transform) -> Transform {
        Transform {
            scale: self.scale * other.scale,
            rot: self.rot * other.rot,
            disp: self.rot * (other.disp * self.scale) + self.disp,
        }
    }

    pub fn inverse(&self) -> Transform {
        let inv_scale = 1.0 / self.scale;
        let inv_rot = self.rot.inverse();
        Transform {
            scale: inv_scale,
            rot: inv_rot,
            disp: inv_rot * (-self.disp * inv_scale),
        }
    }

    pub fn transform_point(&self, p: Vec3) -> Vec3 {
        self.rot * (p * self.scale) + self.disp
    }

    pub fn transform_vector(&self, v: Vec3) -> Vec3 {
        self.rot * (v * self.scale)
    }

    pub fn to_mat4(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(Vec3::splat(self.scale), self.rot, self.disp)
    }

    /// Soft-correct toward `server` for local-player WorldState reconcile.
    ///
    /// Blends translation / rotation when the error is small so contact
    /// physics is not slammed back into geometry every tick. Hard-snaps
    /// when `|Δpos|` or rotation exceeds the 20 Hz thresholds.
    /// Returns `true` if a hard snap was applied (caller should copy dynamo).
    pub fn reconcile_world_state(&mut self, server: &Transform) -> bool {
        let delta_pos = self.disp.distance(server.disp);
        let delta_rot = self.rot.angle_between(server.rot);
        if delta_pos > Self::WORLD_STATE_SNAP_POS || delta_rot > Self::WORLD_STATE_SNAP_ROT {
            *self = *server;
            true
        } else {
            let t = Self::WORLD_STATE_BLEND;
            self.disp = self.disp.lerp(server.disp, t);
            self.rot = self.rot.slerp(server.rot, t);
            self.scale += (server.scale - self.scale) * t;
            false
        }
    }
}

#[derive(Copy, Clone)]
pub struct OrthoParams {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
    pub near: f32,
    pub far: f32,
}

#[derive(Copy, Clone)]
pub struct PerspectiveParams {
    pub fovy: f32,
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
    /// If `Some`, [`Projection::update`] recomputes `fovy` from this
    /// focal length (in screen pixels) and the new viewport height,
    /// matching the original Vangers `focus_flt = 512` formula in
    /// `road.cpp`. `None` keeps `fovy` fixed across resizes.
    pub focal_px: Option<f32>,
}

impl PerspectiveParams {
    /// Original-Vangers FOV: vertical FOV that places `focal_px` screen
    /// pixels at the same angular size as one world unit at distance
    /// `focal_px`. `focus_flt = 512` in the 1998 source.
    pub fn fov_from_focal_px(focal_px: f32, height_px: f32) -> f32 {
        2.0 * (0.5 * height_px / focal_px).atan()
    }
}

/// Default focal length for camera setup, matching `focus_flt = 512` in
/// the original Vangers `road.cpp`. Together with the window height
/// this gives a vertical FOV close to what the 1998 game showed at
/// equivalent resolutions, which feels less "telephoto" than the prior
/// fixed 45°.
pub const DEFAULT_FOCAL_PX: f32 = 512.0;

#[derive(Copy, Clone)]
pub enum Projection {
    Ortho {
        p: OrthoParams,
        original: (u16, u16),
    },
    Perspective(PerspectiveParams),
}

impl Projection {
    pub fn ortho(w: u16, h: u16, depth: Range<f32>) -> Self {
        Projection::Ortho {
            p: OrthoParams {
                left: -0.5 * w as f32,
                right: 0.5 * w as f32,
                top: -0.5 * h as f32,
                bottom: 0.5 * h as f32,
                near: depth.start,
                far: depth.end,
            },
            original: (w, h),
        }
    }

    pub fn update(&mut self, w: u16, h: u16) {
        match *self {
            Projection::Ortho {
                ref mut p,
                ref mut original,
            } => {
                let scale_x = w as f32 / original.0 as f32;
                let scale_y = h as f32 / original.1 as f32;
                let center_x = 0.5 * p.left + 0.5 * p.right;
                let center_y = 0.5 * p.top + 0.5 * p.bottom;
                *original = (w, h);
                p.left = center_x - scale_x * (center_x - p.left);
                p.right = center_x - scale_x * (center_x - p.right);
                p.top = center_y - scale_y * (center_y - p.top);
                p.bottom = center_y - scale_y * (center_y - p.bottom);
            }
            Projection::Perspective(ref mut p) => {
                p.aspect = w as f32 / h as f32;
                if let Some(focal) = p.focal_px {
                    p.fovy = PerspectiveParams::fov_from_focal_px(focal, h as f32);
                }
            }
        }
    }

    pub fn to_matrix(&self) -> Mat4 {
        match *self {
            Projection::Ortho { p, .. } => {
                // GL-style orthographic projection (depth [-1, 1])
                let rml = p.right - p.left;
                let tmb = p.top - p.bottom;
                let fmn = p.far - p.near;
                Mat4::from_cols(
                    Vec4::new(2.0 / rml, 0.0, 0.0, 0.0),
                    Vec4::new(0.0, 2.0 / tmb, 0.0, 0.0),
                    Vec4::new(0.0, 0.0, -2.0 / fmn, 0.0),
                    Vec4::new(
                        -(p.right + p.left) / rml,
                        -(p.top + p.bottom) / tmb,
                        -(p.far + p.near) / fmn,
                        1.0,
                    ),
                )
            }
            Projection::Perspective(p) => {
                // GL-style perspective projection (depth [-1, 1])
                let f = 1.0 / (p.fovy * 0.5).tan();
                let nf = 1.0 / (p.near - p.far);
                Mat4::from_cols(
                    Vec4::new(f / p.aspect, 0.0, 0.0, 0.0),
                    Vec4::new(0.0, f, 0.0, 0.0),
                    Vec4::new(0.0, 0.0, (p.far + p.near) * nf, -1.0),
                    Vec4::new(0.0, 0.0, 2.0 * p.far * p.near * nf, 0.0),
                )
            }
        }
    }
}

/// How far under a ledge top a car straddling it can sag, for
/// [`Camera::ground_under`].
const GROUND_LEDGE_SAG: f32 = 4.0;
/// How far around the chase eye [`Camera::keep_eye_clear`] keeps rock
/// away: about the near plane's half width at the 10u near distance
/// `settings.ron` sets, so a wall this close would fill the screen.
const EYE_FOOTPRINT: f32 = 8.0;
/// How far from a slab's edge [`Camera::chase`] starts dropping toward
/// its under-slab pose, world units.
pub const UNDER_SLAB_REACH: f32 = 96.0;
/// How quickly the under-slab blend eases, per second.
const UNDER_SLAB_RATE: f32 = 6.0;
/// Under-slab pose for a car of [`DEFAULT_SUBJECT_REACH`]: this far
/// behind it at most (see [`Camera::under_slab_follow`] for how it
/// scales with the car)...
const UNDER_SLAB_BACK: f32 = 40.0;
/// ...this far above it at most (less when the gap is tight)...
const UNDER_SLAB_UP: f32 = 20.0;
/// ...with the lens trailing the car's tail by this much per unit of
/// the body's reach, so every car keeps the same on-screen size...
const UNDER_SLAB_TAIL_PER_REACH: f32 = 1.5;
/// ...and by this much at the least...
const UNDER_SLAB_TAIL_GAP: f32 = 20.0;
/// ...rising at most this much per unit back...
const UNDER_SLAB_RISE: f32 = UNDER_SLAB_UP / UNDER_SLAB_BACK;
/// ...looking this far past it...
const UNDER_SLAB_LOOK_AHEAD: f32 = 10.0;
/// ...and following at least this briskly.
const UNDER_SLAB_SPEED: f32 = 3.0;
/// Fastest the blend into (or out of) the under-slab pose moves the
/// chase offset, world units/second.
const UNDER_SLAB_GLIDE_SPEED: f32 = 250.0;
/// Fastest the view turns while blending under a slab, radians/second.
const UNDER_SLAB_TURN_RATE: f32 = std::f32::consts::PI;
/// A slab counts only if its cave floor is within this of the floor
/// under the car: the car drives in that cave, not on a roof or ledge
/// beside a taller overhang whose cave is a pit far below.
pub const SLAB_FLOOR_STEP: f32 = 16.0;
/// A two-level column is a slab only with at least this much gap
/// under it: thinner is a sliver in the floor, not a cave.
const SLAB_MIN_GAP: f32 = 8.0;
/// Body reach [`Camera::chase`] frames when it is not told the car's:
/// the size [`UNDER_SLAB_BACK`] was tuned for.
pub const DEFAULT_SUBJECT_REACH: f32 = 16.0;

/// The car [`Camera::chase_anchored`] frames, beyond its transform.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Subject {
    /// Height the authored follow offset hangs from: the car's own z,
    /// or for `road`'s ground-anchored chase [`Camera::ground_under`].
    pub ground: f32,
    /// How far the body reaches from its centre horizontally, world
    /// units: see [`Subject::reach_of`].
    pub reach: f32,
}

impl Subject {
    /// A chase hung off the car itself.
    pub fn unanchored(car: &Transform, reach: f32) -> Self {
        Subject {
            ground: car.disp.z,
            reach,
        }
    }

    /// The farthest a model's box sticks out from its origin across the
    /// ground plane, scaled to the world: nose or tail for a car.
    pub fn reach_of(bbox: &crate::model::BoundingBox, scale: f32) -> f32 {
        let r = (0..2)
            .map(|i| bbox.min[i].abs().max(bbox.max[i].abs()))
            .fold(0.0, f32::max);
        r * scale
    }
}

#[derive(Copy, Clone)]
pub struct Camera {
    pub loc: Vec3,
    pub rot: Quat,
    // this non-uniform scale is used to make the camera left-handed
    pub scale: Vec3,
    pub proj: Projection,
    /// Heading (yaw about +Z, radians) the chase camera is following,
    /// carried between frames by [`Camera::follow`]. `None` until the
    /// first follow, which adopts the target's heading outright.
    pub chase_yaw: Option<f32>,
    /// How far [`Camera::chase`] has blended from the authored follow
    /// toward the low under-slab pose: 0 in the open, 1 under an
    /// overhang. Eased between frames so entering and leaving glide.
    pub under_slab: f32,
}

#[derive(Debug, Copy, Clone)]
pub struct Follow {
    /// Angle in radians
    pub angle_x: f32,
    pub offset: Vec3,
    pub speed: f32,
    /// World units ahead of the target the camera looks at. Zero keeps
    /// the authored pitch in `angle_x`; a positive value points the
    /// view at a point in front of the vehicle so the body sits lower
    /// in the frame (GTA-style chase cam).
    pub look_ahead: f32,
}

impl Follow {
    /// The look-ahead that points a look-ahead chase from `offset` along
    /// the same view as this follow's authored pitch, so a blend toward
    /// [`Camera::chase`]'s under-slab pose starts from the current view.
    /// Zero when the authored view does not look down.
    pub fn equivalent_look_ahead(&self) -> f32 {
        if self.look_ahead.abs() > 1e-4 {
            return self.look_ahead;
        }
        let heading = Quat::from_rotation_z(std::f32::consts::PI);
        let pos = heading * self.offset;
        let dir = heading * Quat::from_rotation_x(self.angle_x) * -Vec3::Z;
        if dir.z > -1e-3 {
            return 0.0;
        }
        let t = -pos.z / dir.z;
        (pos + dir * t).y.max(0.0)
    }

    pub fn draw_ui(&mut self, ui: &mut egui::Ui) {
        let mut angle_deg = self.angle_x.to_degrees();
        ui.add(egui::Slider::new(&mut angle_deg, -105.0..=0.0).text("Angle"));
        self.angle_x = angle_deg.to_radians();
        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(&mut self.offset.x)
                    .speed(1.0)
                    .prefix("x:"),
            );
            ui.add(
                egui::DragValue::new(&mut self.offset.y)
                    .speed(1.0)
                    .prefix("y:"),
            );
            ui.add(
                egui::DragValue::new(&mut self.offset.z)
                    .speed(1.0)
                    .prefix("z:"),
            );
        });
        ui.add(egui::Slider::new(&mut self.speed, 0.1..=16.0).text("Speed"));
        ui.add(egui::Slider::new(&mut self.look_ahead, 0.0..=80.0).text("Look-ahead"));
    }
}

#[derive(Copy, Clone)]
pub struct Direction {
    pub view: Vec3,
    pub height: f32,
}

impl Camera {
    pub fn dir(&self) -> Vec3 {
        self.rot * -Vec3::Z
    }

    pub fn depth_range(&self) -> Range<f32> {
        match self.proj {
            Projection::Ortho { p, .. } => p.near..p.far,
            Projection::Perspective(p) => p.near..p.far,
        }
    }

    fn get_proj_matrix(&self) -> Mat4 {
        let mut proj = self.proj.to_matrix();
        // convert from GL's depth of [-1,1] to wgpu/gfx-rs [0,1]
        let col = proj.col_mut(0);
        let w0 = col[3];
        col[2] = 0.5 * (col[2] + w0);
        let col = proj.col_mut(1);
        let w1 = col[3];
        col[2] = 0.5 * (col[2] + w1);
        let col = proj.col_mut(2);
        let w2 = col[3];
        col[2] = 0.5 * (col[2] + w2);
        let col = proj.col_mut(3);
        let w3 = col[3];
        col[2] = 0.5 * (col[2] + w3);
        proj
    }

    fn view_transform(&self) -> Transform {
        Transform {
            scale: 1.0,
            rot: self.rot,
            disp: self.loc,
        }
    }

    fn scale_matrix(&self) -> Mat4 {
        Mat4::from_scale(self.scale)
    }

    pub fn get_view_proj(&self) -> Mat4 {
        let view = self.view_transform();
        let view_mx = view.inverse().to_mat4();
        self.get_proj_matrix() * self.scale_matrix() * view_mx
    }

    fn intersect_ray_height(&self, dir: Vec3, height: f32) -> Vec3 {
        let t_raw = (height - self.loc.z) / dir.z;
        let range = self.depth_range();
        let t = range.start.max(t_raw).min(range.end);
        self.loc + t * dir
    }

    pub fn intersect_height(&self, height: f32) -> Vec3 {
        let dir = self.dir();
        self.intersect_ray_height(dir, height)
    }

    pub fn visible_bounds_at(&self, height: f32) -> Range<Vec2> {
        let center = self.intersect_height(height).truncate();
        let mut bounds = center..center;

        let proj = self.get_proj_matrix();
        let view = self.view_transform();
        let mx = view.to_mat4() * self.scale_matrix() * proj.inverse();
        // Scale vectors in a way that makes their Z footprint to be -1 in local space.
        let scaler = 1.0 / self.depth_range().end;
        let ndc_points = [
            Vec2::new(-1.0, -1.0),
            Vec2::new(1.0, -1.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(-1.0, 1.0),
        ];
        for ndc in &ndc_points {
            let v4 = mx * Vec4::new(ndc.x, ndc.y, 1.0, 1.0);
            let wp = Vec3::new(v4.x / v4.w, v4.y / v4.w, v4.z / v4.w);
            let pt = if wp.z < self.loc.z {
                let dir = scaler * (wp - self.loc);
                self.intersect_ray_height(dir, height)
            } else {
                wp
            };
            bounds.start.x = bounds.start.x.min(pt.x);
            bounds.start.y = bounds.start.y.min(pt.y);
            bounds.end.x = bounds.end.x.max(pt.x);
            bounds.end.y = bounds.end.y.max(pt.y);
        }
        bounds
    }

    pub fn visible_bounds(&self) -> Range<Vec2> {
        let lo = self.visible_bounds_at(0.0);
        let min = Vec2::new(self.loc.x.min(lo.start.x), self.loc.y.min(lo.start.y));
        let max = Vec2::new(self.loc.x.max(lo.end.x), self.loc.y.max(lo.end.y));
        min..max
    }

    pub fn bound_points(&self, height: f32) -> [Vec3; 4] {
        let vb = self.visible_bounds_at(height);
        [
            Vec3::new(vb.start.x, vb.start.y, height),
            Vec3::new(vb.end.x, vb.start.y, height),
            Vec3::new(vb.end.x, vb.end.y, height),
            Vec3::new(vb.start.x, vb.end.y, height),
        ]
    }

    /// Heading of a car for the chase camera: the yaw of its forward
    /// axis, and how far that yaw can be trusted (0..=1).
    ///
    /// The flattened forward axis alone degenerates and flips when the
    /// car pitches through vertical; the flattened right axis (turned a
    /// quarter to the front) does the same through a full roll. Their sum
    /// is the car's yaw for any upright pose and survives any pitch or
    /// any roll, shrinking to zero only when the car is upside down - the
    /// one pose whose "forward" depends on how it got there.
    pub fn chase_heading(rot: Quat) -> (f32, f32) {
        let front = rot * Vec3::Y;
        let right = rot * Vec3::X;
        let d = Vec2::new(front.x - right.y, front.y + right.x);
        // |d| is 2 upright, 1 on the side or nose-up, 0 upside down.
        let trust = ((0.5 * d.length() - 0.3) / 0.4).clamp(0.0, 1.0);
        let trust = trust * trust * (3.0 - 2.0 * trust);
        (f32::atan2(-d.x, d.y), trust)
    }

    /// Advance [`Self::chase_yaw`] toward the target's heading.
    ///
    /// Upright the camera takes the car's yaw as is. While the car is
    /// tumbling (upside down, or flipping through it) the heading is held,
    /// eased in again as the car rights itself, and never turned faster
    /// than `CHASE_YAW_RATE`, so an in-air tumble cannot spin the view.
    pub fn update_chase_yaw(&mut self, target: &Transform, dt: f32) -> Quat {
        /// Fastest the chase heading may turn, radians per second: half a
        /// turn a second, twice a hard driving turn.
        const CHASE_YAW_RATE: f32 = std::f32::consts::PI;
        let (yaw, trust) = Self::chase_heading(target.rot);
        let next = match self.chase_yaw {
            Some(prev) if prev.is_finite() => {
                let pi = std::f32::consts::PI;
                let delta = (yaw - prev + pi).rem_euclid(2.0 * pi) - pi;
                let max = CHASE_YAW_RATE * dt.max(0.0);
                let next = prev + (delta * trust).clamp(-max, max);
                (next + pi).rem_euclid(2.0 * pi) - pi
            }
            _ => yaw,
        };
        self.chase_yaw = Some(next);
        Quat::from_rotation_z(next)
    }

    /// The heading [`Self::follow`] last used, as a rotation about +Z.
    pub fn chase_twist(&self) -> Quat {
        Quat::from_rotation_z(self.chase_yaw.unwrap_or(0.0))
    }

    pub fn follow(&mut self, target: &Transform, dt: f32, follow: &Follow) {
        let swing = Quat::from_rotation_x(follow.angle_x);
        let twist = self.update_chase_yaw(target, dt);

        let patch = Quat::from_rotation_z(std::f32::consts::PI);
        let heading = patch * twist;
        let location = target.disp + heading * follow.offset;
        let k = (dt * -follow.speed).exp();

        if follow.look_ahead.abs() > 1e-4 {
            // Move first, then look from the actual (possibly lagged)
            // position at a point ahead of the car. Position lag then
            // cannot lift the car toward the top of the frame: the
            // body stays below the look-at no matter how far behind
            // the camera is still catching up.
            self.loc = location * (1.0 - k) + self.loc * k;
            let look_at = target.disp + twist * Vec3::new(0.0, follow.look_ahead, 0.0);
            let rotation = Self::look_rotation(look_at - self.loc);
            self.rot = rotation.slerp(self.rot, k);
        } else {
            let rotation = heading * swing;
            self.rot = rotation.slerp(self.rot, k);
            self.loc = location * (1.0 - k) + self.loc * k;
        }
    }

    /// The whole chase-camera step `road` and `web` run each frame:
    /// [`Self::follow`], then [`Self::avoid_terrain`], then the
    /// look-ahead re-aim, with the follow blended toward a low,
    /// close pose while the target is under (or about to go under) an
    /// overhanging slab.
    ///
    /// Under a slab the authored follow (`settings.ron` puts it 240u up)
    /// would sit above the slab looking down through it; the ray pull
    /// then parks it just under the ceiling, straight over the car,
    /// staring down a cave a hundred units deep. Here it drops behind
    /// and below the slab underside instead and looks past the car.
    pub fn chase(
        &mut self,
        level: &crate::level::Level,
        target: &Transform,
        dt: f32,
        follow: &Follow,
        clearance: f32,
    ) {
        let subject = Subject::unanchored(target, DEFAULT_SUBJECT_REACH);
        self.chase_anchored(level, target, subject, dt, follow, clearance);
    }

    /// [`Self::chase`] for a car of known size, optionally with `road`'s
    /// ground-anchored follow: the authored offset hangs off the car's
    /// (x, y) at height `subject.ground` (see [`Self::ground_under`]) so
    /// jumps don't yank the view, while the
    /// slab blend, the pull out of terrain and the sight line all use the
    /// car itself. On top of an arch, or flying over one, the ground
    /// under the car can be the floor of the gap far below; judging
    /// "under a slab" from there dropped the camera into that gap,
    /// looking up through the arch at the car. The under-slab pose
    /// trails the car by its `subject.reach`.
    pub fn chase_anchored(
        &mut self,
        level: &crate::level::Level,
        car: &Transform,
        subject: Subject,
        dt: f32,
        follow: &Follow,
        clearance: f32,
    ) {
        let mut anchored = *car;
        anchored.disp.z = subject.ground;
        let target = &anchored;
        let (cover, ceiling) = Self::slab_cover(level, car.disp, clearance);
        let rate = 1.0 - (-dt * UNDER_SLAB_RATE).exp();
        let slab = (cover > 0.0 || self.under_slab > 0.0).then(|| {
            let slab =
                Self::under_slab_follow(follow, target.disp.z, ceiling, clearance, subject.reach);
            let yaw = self
                .chase_yaw
                .unwrap_or_else(|| Self::chase_heading(target.rot).0);
            let offset = Self::unblocked_slab_offset(
                level,
                target.disp,
                yaw,
                slab.offset,
                ceiling,
                clearance,
            );
            (slab, offset)
        });
        // Eased toward the cover, but no faster than moves the blended
        // offset `UNDER_SLAB_GLIDE_SPEED`.
        let span = slab.map_or(0.0, |(_, offset)| (offset - follow.offset).length());
        let max_step = dt * UNDER_SLAB_GLIDE_SPEED / span.max(1.0);
        self.under_slab += ((cover - self.under_slab) * rate).clamp(-max_step, max_step);
        if self.under_slab < 1e-3 {
            self.under_slab = 0.0;
        }
        let b = self.under_slab;
        let authored_look_ahead = follow.look_ahead.abs() > 1e-4;
        let follow = match slab {
            Some((slab, offset)) if b > 0.0 => Follow {
                angle_x: follow.angle_x,
                offset: follow.offset.lerp(offset, b),
                speed: follow.speed + (slab.speed - follow.speed) * b,
                look_ahead: follow.equivalent_look_ahead()
                    + (slab.look_ahead - follow.equivalent_look_ahead()) * b,
            },
            _ => *follow,
        };
        let last_rot = self.rot;
        self.follow(target, dt, &follow);
        self.avoid_terrain(level, car.disp, clearance);
        if follow.look_ahead.abs() > 1e-4 {
            // The damped heading `follow` used, not the raw car yaw,
            // which flips while the car tumbles.
            let look_at = target.disp + self.chase_twist() * Vec3::new(0.0, follow.look_ahead, 0.0);
            let aim = Self::look_rotation(look_at - self.loc);
            self.rot = if b > 0.0 {
                // Blending in from the authored pitch, or re-aiming
                // after a pull under the slab: turn, don't snap. A chase
                // that re-aims outdoors too has the cap loosen as the
                // blend fades, so leaving does not snap back to it; an
                // authored-pitch chase eases back by its own slerp.
                let angle = last_rot.angle_between(aim);
                let max = if authored_look_ahead {
                    UNDER_SLAB_TURN_RATE * dt / b
                } else {
                    UNDER_SLAB_TURN_RATE * dt
                };
                if angle > max {
                    last_rot.slerp(aim, max / angle)
                } else {
                    aim
                }
            } else {
                aim
            };
        }
    }

    /// The pose [`Self::chase`] settles into under a slab whose
    /// underside is at `ceiling`: behind the car and low, in the gap
    /// between the floor and the slab, looking just past the car.
    ///
    /// The arm scales with the car's `reach` (nose or tail from its
    /// centre): the lens trails the tail by 1.5 reaches, at least 20u,
    /// so a long car is not framed the size of the screen with the lens
    /// over its boot. A closer authored chase stays that close, but not
    /// closer than 20u past the tail. The rise keeps the authored pitch
    /// of the tuned pose (1 up per 2 back), and never more than half the
    /// headroom left under the slab.
    pub fn under_slab_follow(
        follow: &Follow,
        car_z: f32,
        ceiling: f32,
        clearance: f32,
        reach: f32,
    ) -> Follow {
        let sized = reach + (reach * UNDER_SLAB_TAIL_PER_REACH).max(UNDER_SLAB_TAIL_GAP);
        let back = if follow.offset.y > 1.0 {
            follow.offset.y.min(sized).max(reach + UNDER_SLAB_TAIL_GAP)
        } else {
            sized
        };
        // Half the headroom left after clearing the roof, so the view
        // does not graze the underside.
        let room = ((ceiling - car_z - 2.0 * clearance) * 0.5).max(0.0);
        let up = follow
            .offset
            .z
            .clamp(8.0, (back * UNDER_SLAB_RISE).max(8.0))
            .min(room);
        let look_ahead = if follow.look_ahead > 1e-4 {
            follow.look_ahead.min(UNDER_SLAB_LOOK_AHEAD)
        } else {
            UNDER_SLAB_LOOK_AHEAD
        };
        Follow {
            angle_x: follow.angle_x,
            offset: Vec3::new(follow.offset.x, back, up),
            speed: follow.speed.max(UNDER_SLAB_SPEED),
            look_ahead,
        }
    }

    /// The under-slab `offset` (x, back, up), swung up over the car
    /// when rock or a rising floor stands right behind it.
    ///
    /// Backed up to a cave wall, the low pose behind the car is inside
    /// the wall; pulling that arm in toward the car puts the lens on its
    /// roof, the car filling the screen behind the see-through veil.
    /// Tilting the arm up, toward a view from above the car still under
    /// the slab, keeps it at length in the open gap instead. The first
    /// elevation whose arm is clear wins, else the longest clear arm;
    /// the follow eases the camera between them.
    pub fn unblocked_slab_offset(
        level: &crate::level::Level,
        car: Vec3,
        yaw: f32,
        offset: Vec3,
        ceiling: f32,
        clearance: f32,
    ) -> Vec3 {
        let heading = Quat::from_rotation_z(std::f32::consts::PI + yaw);
        let eye = Vec3::new(car.x, car.y, car.z + clearance);
        let top = (ceiling - car.z - 2.0 * clearance).max(offset.z);
        let length = Vec2::new(offset.y, offset.z).length();
        let start = offset.z.atan2(offset.y);
        let steps = 8;
        let mut best = (0.0, offset);
        for i in 0..=steps {
            let e = start + (std::f32::consts::FRAC_PI_2 - start) * i as f32 / steps as f32;
            let mut arm = Vec2::new(e.cos(), e.sin()) * length;
            if arm.y > top {
                arm *= top / arm.y;
            }
            let candidate = Vec3::new(offset.x, arm.x, arm.y);
            let free = Self::free_arm(level, eye, car + heading * candidate, clearance, car.z);
            // The whole arm: one ending in the wall puts the lens there,
            // and the pull out of terrain folds it back onto the car.
            if free >= 1.0 {
                return candidate;
            }
            let reach = free * arm.length();
            if reach > best.0 + 1.0 {
                best = (reach, candidate);
            }
        }
        best.1
    }

    /// How much of the segment `from`..`to` (0..1) is open air for a
    /// camera, by the under-slab rule of [`Self::point_hits_terrain`].
    fn free_arm(
        level: &crate::level::Level,
        from: Vec3,
        to: Vec3,
        clearance: f32,
        car_z: f32,
    ) -> f32 {
        let dist = (to - from).length();
        // Every 3u: a steep ramp can rise through a longer step unseen.
        let steps = ((dist / 3.0) as usize).clamp(6, 60);
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            if Self::point_hits_terrain(level, from.lerp(to, t), clearance, Some(car_z)) {
                return (i - 1) as f32 / steps as f32;
            }
        }
        1.0
    }

    /// A slab overhangs `p` at `(x, y)`: a two-level column whose cave
    /// is open at `p`'s height (floor near or below it, underside
    /// above it, and some rock on top), and whose cave floor is
    /// level with `floor`, the floor under `p` - so `p` is in (or at the
    /// mouth of) that cave. Single-level holes and degenerate
    /// `low == mid == high` columns are not slabs, nor are slivers with
    /// next to no gap under them (`low` 7, `mid` 7, `high` 9 litters
    /// Fostral's underpass floor); nor is a taller
    /// overhang next to a roof or ledge, whose cave floor is far below.
    fn slab_ceiling_at(
        level: &crate::level::Level,
        x: f32,
        y: f32,
        z: f32,
        floor: f32,
    ) -> Option<f32> {
        match level.get((x as i32, y as i32)) {
            crate::level::Texel::Dual { low, mid, high }
                if mid > z
                    && mid > low.0 + SLAB_MIN_GAP
                    && low.0 <= z + 8.0
                    && (low.0 - floor).abs() <= SLAB_FLOOR_STEP
                    && high.0 > mid + 1.0 =>
            {
                Some(mid)
            }
            _ => None,
        }
    }

    /// The ground a car at `p` rests on, for the native chase's
    /// ground anchor: the highest floor at or below the car within a
    /// few units, not just under its centre. Fostral's underpass floor
    /// (10) is pitted with single texels down at 0-4 that the hull rides
    /// straight over; anchored in one, the camera's arm started below
    /// the surrounding floor, its first sample hit that floor, and the
    /// arm collapsed onto the car with the near plane in the rock.
    pub fn ground_under(level: &crate::level::Level, p: Vec3) -> f32 {
        let mut ground = level.floor_below(p);
        for dy in -1..=1 {
            for dx in -1..=1 {
                let q = Vec3::new(p.x + dx as f32 * 4.0, p.y + dy as f32 * 4.0, p.z);
                let floor = level.floor_below(q);
                // A hull straddling a ledge can sag with its centre a
                // little under the ledge top; a roof hole's slab top is
                // far above.
                if floor <= p.z + GROUND_LEDGE_SAG {
                    ground = ground.max(floor);
                }
            }
        }
        ground
    }

    /// The car's own column, or any within a few units, is under a slab.
    /// Fostral's underpasses are speckled with single-level holes; the
    /// car's texel alone flips in and out of "cave" as it rolls across
    /// them, and each flip used to treat the whole gap as rock and yank
    /// the camera onto the car's roof.
    pub fn under_slab_at(level: &crate::level::Level, p: Vec3) -> Option<f32> {
        let floor = level.floor_below(p);
        let mut ceiling: Option<f32> = None;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (x, y) = (p.x + dx as f32 * 4.0, p.y + dy as f32 * 4.0);
                if let Some(c) = Self::slab_ceiling_at(level, x, y, p.z, floor) {
                    ceiling = Some(ceiling.map_or(c, |m| m.min(c)));
                }
            }
        }
        ceiling
    }

    /// How much `p` is under an overhang (1 under it, easing to 0
    /// [`UNDER_SLAB_REACH`] away from its edge) and the lowest slab
    /// underside near it.
    fn slab_cover(level: &crate::level::Level, p: Vec3, clearance: f32) -> (f32, f32) {
        let step = 8.0;
        let n = (UNDER_SLAB_REACH / step) as i32;
        let mut nearest = f32::INFINITY;
        let mut ceiling = f32::INFINITY;
        let floor = level.floor_below(p);
        for j in -n..=n {
            for i in -n..=n {
                let off = glam::Vec2::new(i as f32, j as f32) * step;
                let d = off.length();
                if d > UNDER_SLAB_REACH {
                    continue;
                }
                if let Some(c) = Self::slab_ceiling_at(level, p.x + off.x, p.y + off.y, p.z, floor)
                {
                    nearest = nearest.min(d);
                    if d <= UNDER_SLAB_BACK + step {
                        ceiling = ceiling.min(c);
                    }
                }
            }
        }
        if let Some(c) = Self::under_slab_at(level, p) {
            nearest = 0.0;
            ceiling = ceiling.min(c);
        }
        if !nearest.is_finite() {
            return (0.0, ceiling);
        }
        if !ceiling.is_finite() {
            // Only distant slabs: plan for the lowest one in reach.
            ceiling = p.z + 2.0 * clearance + 2.0 * UNDER_SLAB_UP;
        }
        let t = ((nearest - step) / (UNDER_SLAB_REACH - step)).clamp(0.0, 1.0);
        (1.0 - t * t * (3.0 - 2.0 * t), ceiling)
    }

    /// Push the camera up out of the ground.
    ///
    /// A chase camera trails the car by a fixed offset, which puts it
    /// inside the hillside whenever the car drives up a slope or into a
    /// dip - the view then ends up under the terrain looking at its
    /// backfaces. Raising it to sit `clearance` above whatever surface is
    /// below keeps the framing while never letting that happen.
    ///
    /// Only ever raises: a camera legitimately under a slab, in a tunnel
    /// or cave, is above that space's own floor and is left alone.
    pub fn keep_above_ground(&mut self, level: &crate::level::Level, clearance: f32) {
        let floor = level.floor_below(self.loc);
        self.loc.z = self.loc.z.max(floor + clearance);
    }

    /// Pull the chase camera out of intervening terrain toward `anchor`
    /// (usually the car), then apply [`keep_above_ground`].
    ///
    /// Z-only lifts park the camera on a ridge while the see-through
    /// focus cone still punches a hole. Sampling the car→camera segment
    /// and shortening that arm clears the hill for real; the focus veil
    /// can then stay off when the line of sight is open.
    pub fn avoid_terrain(
        &mut self,
        level: &crate::level::Level,
        anchor: Vec3,
        clearance: f32,
    ) {
        // `floor_below` returns the cave floor when z is under a slab, which
        // makes a buried outdoor chase cam look "clear" inside a hill. Use
        // the top surface outdoors; only trust floor_below when the car
        // itself is in a cave, or the chase is already down under a
        // slab (`under_slab`) and on its way back out.
        let cave_mode = Self::under_slab_at(level, anchor).is_some();
        let gap_ok = cave_mode || self.under_slab > 0.0;
        if gap_ok {
            self.duck_under_slab(level, anchor, clearance);
        }
        let eye_start = Vec3::new(anchor.x, anchor.y, anchor.z + clearance);
        let delta = self.loc - eye_start;
        let dist = delta.length();
        if dist > 1.0 {
            let dir = delta / dist;
            let steps = ((dist / 6.0) as usize).clamp(6, 40);
            let mut hit_t = 1.0_f32;
            for i in 1..=steps {
                let t = i as f32 / steps as f32;
                let p = eye_start + dir * (dist * t);
                if Self::point_hits_terrain(level, p, clearance, gap_ok.then_some(anchor.z)) {
                    hit_t = (i - 1) as f32 / steps as f32;
                    // Outdoors keep a minimum arm so a hill right behind
                    // the car does not put the lens on its roof. Under a
                    // slab the first sample can already be at the
                    // ceiling: stopping short of it is the only free
                    // spot, never a fixed fraction up inside the rock.
                    if !gap_ok {
                        hit_t = hit_t.max(0.12);
                    }
                    break;
                }
            }
            if hit_t < 0.999 {
                self.loc = eye_start + dir * (dist * hit_t);
            }
        }
        self.keep_above_ground(level, clearance);
        if gap_ok {
            self.keep_below_slab(level, clearance);
        } else {
            // Under a slab the lens belongs in the gap below the
            // underside, next to rock by design; see `keep_below_slab`.
            self.keep_eye_clear(level, clearance);
        }
    }

    /// The last word on where the eye may be: never in the rock, nor
    /// with rock within [`EYE_FOOTPRINT`] of it at its own height.
    ///
    /// `keep_above_ground` only reads the column under the lens. In a pit
    /// or a crevice the lens can sit over the open floor with a wall a
    /// few units away, and that wall fills the near plane. Then the eye
    /// goes up over every column around it, `clearance` above the highest
    /// top: the rule the original game keeps everywhere. Its eye is `ViewZ = focus * TurnSecX /
    /// xsize` above sea level (KranX/Vangers `road.cpp:1841`, `focus` 512
    /// at `road.cpp:1711`), and `TurnSecX` never drops below
    /// `xsize * MIN_ZOOM >> 8` (`mechos.cpp:3808`, `MIN_ZOOM` 128 at
    /// `mechos.cpp:3659`), so the eye is at least 256 up (221 tilted by
    /// `SLOPE_MAX`, pi/6), over every voxel (heights are 0-255).
    pub fn keep_eye_clear(&mut self, level: &crate::level::Level, clearance: f32) {
        let column = |p: &Vec3| level.get((p.x.floor() as i32, p.y.floor() as i32));
        let ring = Self::eye_footprint(self.loc);
        if !ring.iter().any(|&p| level.solid_at(p)) {
            return;
        }
        let top = ring
            .iter()
            .map(|p| column(p).high())
            .fold(f32::MIN, f32::max);
        self.loc.z = self.loc.z.max(top + clearance);
    }

    /// The eye and eight points [`EYE_FOOTPRINT`] around it, level with it.
    fn eye_footprint(eye: Vec3) -> [Vec3; 9] {
        let r = EYE_FOOTPRINT;
        let d = r * std::f32::consts::FRAC_1_SQRT_2;
        [
            (0.0, 0.0),
            (r, 0.0),
            (-r, 0.0),
            (0.0, r),
            (0.0, -r),
            (d, d),
            (d, -d),
            (-d, d),
            (-d, -d),
        ]
        .map(|(x, y)| Vec3::new(eye.x + x, eye.y + y, eye.z))
    }

    /// Lower the camera just enough that the arm from the car's eye
    /// passes under every slab underside on the way: rising back out
    /// from under a slab, the trailing camera meets the underside before
    /// the car does, and the arm test would otherwise pull it a dozen
    /// units toward the car in one frame.
    fn duck_under_slab(&mut self, level: &crate::level::Level, anchor: Vec3, clearance: f32) {
        let eye = Vec3::new(anchor.x, anchor.y, anchor.z + clearance);
        let delta = self.loc - eye;
        let steps = ((delta.length() / 6.0) as usize).clamp(6, 40);
        let mut top = self.loc.z;
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let p = eye + delta * t;
            if let Some(ceiling) = Self::under_slab_at(level, Vec3::new(p.x, p.y, anchor.z)) {
                let limit = ceiling - clearance - 0.5;
                if limit > eye.z {
                    top = top.min(eye.z + (limit - eye.z) / t);
                }
            }
        }
        self.loc.z = top;
    }

    /// Under a slab, hold the camera `clearance` below its underside
    /// (but never below the floor's own clearance).
    fn keep_below_slab(&mut self, level: &crate::level::Level, clearance: f32) {
        if let crate::level::Texel::Dual { low, mid, high } =
            level.get((self.loc.x as i32, self.loc.y as i32))
            && self.loc.z < mid
            && high.0 > mid + 1.0
        {
            let top = (mid - clearance).max(low.0 + clearance);
            self.loc.z = self.loc.z.min(top);
        }
    }

    /// `under`: the car's height when it is under (or the camera is
    /// leaving) a slab, so the gap between floor and underside is air.
    fn point_hits_terrain(
        level: &crate::level::Level,
        p: Vec3,
        clearance: f32,
        under: Option<f32>,
    ) -> bool {
        let texel = level.get((p.x as i32, p.y as i32));
        let Some(car_z) = under else {
            return p.z < texel.high() + clearance;
        };
        // Only rock stops the arm at the floor: a cave floor rising behind
        // the car would otherwise fold the camera onto its roof, where
        // `keep_above_ground` lifts it clear just as well.
        if p.z < level.floor_below(p) {
            return true;
        }
        // Brushing the slab underside, or rising through it - including
        // through the single-level holes that speckle Fostral's roofs.
        matches!(
            Self::under_slab_at(level, Vec3::new(p.x, p.y, car_z)),
            Some(ceiling) if p.z > ceiling - clearance
        ) || matches!(
            texel,
            crate::level::Texel::Dual { mid, high, .. }
                if high.0 > mid + 1.0 && p.z > mid - clearance && p.z < high.0 + clearance
        )
    }

    /// True when height samples along cam→car rise through the sight line.
    /// Cheap CPU gate for the GPU focus veil.
    pub fn terrain_blocks_view(
        level: &crate::level::Level,
        cam: Vec3,
        car: Vec3,
        clearance: f32,
    ) -> bool {
        let delta = car - cam;
        let dist = delta.length();
        if dist < 1.0 {
            return false;
        }
        let cave_mode = Self::under_slab_at(level, car).is_some();
        let dir = delta / dist;
        let steps = ((dist / 8.0) as usize).clamp(4, 24);
        // Stop short of the car body so the mechos itself is not a "hit".
        let end = 0.85;
        for i in 1..steps {
            let t = end * (i as f32 / steps as f32);
            let p = cam + dir * (dist * t);
            let surface = if cave_mode {
                level.floor_below(p)
            } else {
                match level.get((p.x as i32, p.y as i32)) {
                    // Threading the open gap under an arch or slab is
                    // clear sight, not a view through its top.
                    crate::level::Texel::Dual { low, mid, high }
                        if high.0 > mid + 1.0 && p.z < mid - clearance * 0.35 =>
                    {
                        low.0
                    }
                    texel => texel.high(),
                }
            };
            if surface > p.z + clearance * 0.35 {
                return true;
            }
        }
        false
    }

    pub fn look_by(&mut self, target: &Transform, dir: &Direction) {
        debug_assert!(dir.view.z < 0.0);
        let k = (target.disp.z - self.loc.z) / -dir.view.z;
        self.loc = target.disp + dir.view * k;
        self.rot = Quat::from_rotation_x(30.0f32.to_radians());
    }

    pub fn focus_on(&mut self, target: &Transform) {
        self.loc = target.disp + Vec3::new(0.0, -64.0, 40.0);
        self.rot = Quat::from_rotation_x(std::f32::consts::FRAC_PI_3);
        // A jump cut: the next follow takes the target's heading outright.
        self.chase_yaw = None;
    }

    pub fn rotate_focus(&mut self, target: &Transform, hor: f32, ver: f32) {
        let mut view = Transform {
            scale: 1.0,
            rot: self.rot,
            disp: self.loc,
        };
        if hor != 0.0 {
            let pre = Transform {
                scale: 1.0,
                rot: Quat::IDENTITY,
                disp: -target.disp,
            };
            let post = Transform {
                scale: 1.0,
                rot: Quat::from_rotation_z(hor),
                disp: target.disp,
            };
            view = post.concat(&pre.concat(&view));
        }
        if ver != 0.0 {
            let target_inv = target.inverse();
            let axis_local = target_inv.rot * self.rot * Vec3::X;
            let mid = Transform {
                scale: 1.0,
                rot: Quat::from_axis_angle(axis_local, -ver),
                disp: Vec3::ZERO,
            };
            view = target.concat(&mid.concat(&target_inv.concat(&view)));
        }
        self.loc = view.disp;
        self.rot = view.rot;
    }

    /// Rotation that looks along `forward` with the horizon level.
    ///
    /// The engine folds a `scale` of (1, -1, 1) into the view matrix to make
    /// the camera left-handed (see `get_view_proj`), so what ends up pointing
    /// up on screen is `-(rot * Y)`, not `rot * Y`. The basis is built
    /// accordingly; negating two columns keeps the rotation proper.
    pub fn look_rotation(forward: Vec3) -> Quat {
        let forward = forward.normalize_or(Vec3::Y);
        // World-up is +Z, except when forward is parallel to it.
        let up_ref = if forward.cross(Vec3::Z).length_squared() > 1e-6 {
            Vec3::Z
        } else {
            Vec3::Y
        };
        let right = forward.cross(up_ref).normalize();
        let up = right.cross(forward).normalize();
        Quat::from_mat3(&glam::Mat3::from_cols(-right, -up, -forward))
    }

    /// Places the camera `distance` from `target`, `elevation` radians above
    /// the horizontal, looking at it.
    pub fn orbit_to(&mut self, target: Vec3, distance: f32, elevation: f32) {
        let offset = distance * Vec3::new(0.0, -elevation.cos(), elevation.sin());
        self.loc = target + offset;
        self.rot = Self::look_rotation(-offset);
    }

    pub fn front_face(&self) -> wgpu::FrontFace {
        if self.scale.x * self.scale.y > 0.0 {
            wgpu::FrontFace::Cw
        } else {
            wgpu::FrontFace::Ccw
        }
    }

    /// The horizontal unit vector that moves toward the top of the
    /// screen: the heading projected onto the ground plane. Looking
    /// straight down or up the heading has no ground component, and
    /// "forward" is the screen's up direction instead.
    ///
    /// This is the axis W/S travel along. It has to switch because the
    /// camera's Y axis - the old axis - is horizontal only in the
    /// top-down view; once pitched to the horizon its ground projection
    /// vanishes, and normalizing it turns the position into NaN.
    pub fn ground_forward(&self) -> Vec3 {
        let mut v = self.dir();
        v.z = 0.0;
        if v.length_squared() < 1e-6 {
            v = -(self.rot * Vec3::Y);
            v.z = 0.0;
        }
        v.normalize()
    }

    /// Heading and elevation, in degrees. Yaw is 0 along +Y and grows
    /// clockwise; pitch is 0 at the horizon and positive looking up.
    pub fn angles(&self) -> (f32, f32) {
        let fwd = self.dir();
        (
            fwd.x.atan2(fwd.y).to_degrees().rem_euclid(360.0),
            fwd.z.clamp(-1.0, 1.0).asin().to_degrees(),
        )
    }

    /// Point the camera at a heading and elevation, keeping the horizon
    /// level. Matches the basis `--fp-yaw`/`--fp-pitch` build, so a view
    /// set here reproduces exactly under the snapshot binary.
    pub fn set_angles(&mut self, yaw_deg: f32, pitch_deg: f32) {
        let (yaw, pitch) = (yaw_deg.to_radians(), pitch_deg.to_radians());
        let fwd = Vec3::new(
            yaw.sin() * pitch.cos(),
            yaw.cos() * pitch.cos(),
            pitch.sin(),
        )
        .normalize();
        // Looking straight up or down leaves the heading undefined, so
        // pick a reference that is not parallel to it.
        let up_ref = if fwd.cross(Vec3::Z).length_squared() > 1e-6 {
            Vec3::Z
        } else {
            Vec3::Y
        };
        let right = fwd.cross(up_ref).normalize();
        let up = right.cross(fwd).normalize();
        // The engine folds a `scale` of (1, -1, 1) into the view matrix to
        // make the camera left-handed, so what points up on *screen* is
        // `-(rot * Y)`, not `rot * Y`. Negating two columns keeps the
        // rotation proper while accounting for it. Building the naive
        // right-handed basis instead renders the world upside down - and
        // `dir()` cannot tell, because it reads the third column, which is
        // identical either way.
        self.rot = Quat::from_mat3(&glam::Mat3::from_cols(-right, -up, -fwd));
    }

    pub fn draw_ui(&mut self, ui: &mut egui::Ui) {
        // Read the angles back from the rotation every frame so flying
        // with the keyboard keeps these in step, and only write when the
        // user actually drags one - otherwise the round trip through
        // degrees would slowly rewrite the orientation.
        let (mut yaw, mut pitch) = self.angles();
        let mut aimed = false;
        ui.horizontal(|ui| {
            ui.label("Yaw");
            aimed |= ui
                .add(
                    egui::DragValue::new(&mut yaw)
                        .speed(0.5)
                        .range(0.0..=360.0)
                        .suffix("°"),
                )
                .changed();
            ui.label("Pitch");
            aimed |= ui
                .add(
                    egui::DragValue::new(&mut pitch)
                        .speed(0.5)
                        .range(-90.0..=90.0)
                        .suffix("°"),
                )
                .changed();
        });
        // Quarter turns, since that is the interval a bug is most likely
        // to be described in.
        ui.horizontal(|ui| {
            for step in [-90.0f32, -15.0, 15.0, 90.0] {
                if ui.button(format!("{step:+.0}°")).clicked() {
                    yaw = (yaw + step).rem_euclid(360.0);
                    aimed = true;
                }
            }
        });
        if aimed {
            self.set_angles(yaw, pitch);
        }

        match self.proj {
            Projection::Ortho {
                ref mut p,
                original: _,
            } => {
                ui.add(egui::Slider::new(&mut p.near, 0.1..=50.0).text("Depth near"));
                ui.add(egui::Slider::new(&mut p.far, 50.0..=10000.0).text("Depth far"));
            }
            Projection::Perspective(ref mut p) => {
                ui.add(egui::Slider::new(&mut p.near, 0.1..=50.0).text("Depth near"));
                ui.add(egui::Slider::new(&mut p.far, 50.0..=10000.0).text("Depth far"));
            }
        }
    }

    /// Tweaks + web Settings. Free-cam knobs always; chase sliders when
    /// `follow` is `Some`. `ground_anchor` is native-only.
    pub fn draw_controls(
        &mut self,
        ui: &mut egui::Ui,
        follow: Option<&mut Follow>,
        ground_anchor: Option<&mut bool>,
    ) {
        self.draw_ui(ui);
        if let Some(follow) = follow {
            follow.draw_ui(ui);
        }
        if let Some(anchor) = ground_anchor {
            ui.checkbox(anchor, "Ground anchor");
        }
        ui.label(format!(
            "Pos: ({:.0}, {:.0}, {:.0})",
            self.loc.x, self.loc.y, self.loc.z
        ));
    }
}

#[cfg(test)]
mod ground_tests {
    use super::*;
    use crate::{config::settings, level};

    fn test_level() -> level::Level {
        level::load(
            &level::LevelConfig::new_test(),
            &settings::Geometry::default(),
        )
    }

    fn cam_at(loc: Vec3) -> Camera {
        Camera {
            chase_yaw: None,
            under_slab: 0.0,
            loc,
            rot: Quat::IDENTITY,
            scale: Vec3::ONE,
            proj: Projection::Perspective(PerspectiveParams {
                fovy: 45f32.to_radians(),
                aspect: 1.0,
                near: 1.0,
                far: 1000.0,
                focal_px: None,
            }),
        }
    }

    /// `set_angles` and `angles` have to be exact inverses, and have to
    /// agree with the basis `--fp-yaw`/`--fp-pitch` build in the snapshot
    /// binary. If they drift, an angle dialled in the viewer renders as a
    /// different view under the harness, which is the one thing the
    /// control exists to prevent.
    #[test]
    fn angles_round_trip() {
        let mut cam = cam_at(Vec3::new(100.0, 100.0, 50.0));
        for yaw_step in 0..24 {
            for pitch_step in -4..=4 {
                let (yaw, pitch) = (yaw_step as f32 * 15.0, pitch_step as f32 * 20.0);
                cam.set_angles(yaw, pitch);
                let (got_yaw, got_pitch) = cam.angles();
                assert!(
                    (got_pitch - pitch).abs() < 1e-3,
                    "pitch {pitch} came back as {got_pitch}"
                );
                // Yaw is meaningless looking straight up or down, and 0
                // and 360 are the same heading.
                if pitch.abs() < 89.0 {
                    let d = (got_yaw - yaw).rem_euclid(360.0);
                    let d = d.min(360.0 - d);
                    assert!(d < 1e-2, "yaw {yaw} came back as {got_yaw}");
                }
            }
        }
    }

    /// `set_angles` must build the whole basis the snapshot binary does,
    /// not just aim in the same direction.
    ///
    /// Comparing `dir()` alone is not enough and was the hole in the first
    /// version of this test: `dir()` reads the third column of the
    /// rotation, which is the same whether or not the other two are
    /// negated for the engine's left-handed `scale`. A camera built the
    /// naive right-handed way points exactly where you asked and renders
    /// the world upside down, and `dir()` reports success throughout.
    #[test]
    fn angles_match_the_snapshot_camera() {
        let mut cam = cam_at(Vec3::ZERO);
        for (yaw, pitch) in [
            (0.0f32, 0.0f32),
            (122.0, -12.0),
            (275.0, 10.0),
            (150.0, -1.0),
            (30.0, 89.0),
            (30.0, -89.0),
        ] {
            cam.set_angles(yaw, pitch);
            let (y, p) = (yaw.to_radians(), pitch.to_radians());
            let fwd = Vec3::new(y.sin() * p.cos(), y.cos() * p.cos(), p.sin()).normalize();
            assert!(
                (cam.dir() - fwd).length() < 1e-5,
                "yaw {yaw} pitch {pitch}: aimed at {:?}, wanted {fwd:?}",
                cam.dir()
            );

            // Rebuild the basis exactly as `make_camera` does, including
            // the two negations that account for the left-handed scale.
            let up_ref = if fwd.cross(Vec3::Z).length_squared() > 1e-6 {
                Vec3::Z
            } else {
                Vec3::Y
            };
            let right = fwd.cross(up_ref).normalize();
            let up = right.cross(fwd).normalize();
            let want = Quat::from_mat3(&glam::Mat3::from_cols(-right, -up, -fwd));
            assert!(
                (cam.rot * Vec3::X - want * Vec3::X).length() < 1e-4
                    && (cam.rot * Vec3::Y - want * Vec3::Y).length() < 1e-4,
                "yaw {yaw} pitch {pitch}: basis differs from the snapshot camera"
            );

            // And the thing a player notices: with the engine's scale
            // folded in, screen-up has to have a positive world Z whenever
            // the camera is not pointing straight at the zenith.
            if pitch < 89.0 {
                let screen_up = -(cam.rot * Vec3::Y);
                assert!(
                    screen_up.z > 0.0,
                    "yaw {yaw} pitch {pitch}: screen-up is {screen_up:?}, \
                     the view is upside down"
                );
            }
        }
    }

    /// `ground_forward` is the axis W/S travel along, and it has to
    /// survive the rotation to the horizon: the previous axis (the
    /// camera's Y projected onto the ground) vanishes there, and
    /// normalizing it turned the position into NaN.
    #[test]
    fn ground_forward_survives_the_horizon() {
        let mut cam = cam_at(Vec3::ZERO);
        // Top-down: forward is the top of the screen.
        assert!((cam.ground_forward() - Vec3::new(0.0, -1.0, 0.0)).length() < 1e-4);
        for pitch in [-89.0f32, -45.0, -1.0, 0.0, 30.0, 89.0] {
            for yaw in [0.0f32, 90.0, 195.0, 270.0] {
                cam.set_angles(yaw, pitch);
                let fwd = cam.ground_forward();
                assert!(
                    fwd.z.abs() < 1e-4 && fwd.length() > 0.999,
                    "yaw {yaw} pitch {pitch}: {fwd:?} is not a horizontal unit vector"
                );
                // It points where the camera looks, horizontally.
                let mut want = cam.dir();
                want.z = 0.0;
                assert!(
                    (fwd - want.normalize()).length() < 1e-3,
                    "yaw {yaw} pitch {pitch}: forward {fwd:?} disagrees with the heading"
                );
            }
        }
        // Straight down, the heading degenerates and the screen top takes over.
        cam.set_angles(0.0, -90.0);
        assert!((cam.ground_forward() - Vec3::new(0.0, 1.0, 0.0)).length() < 1e-4);
    }

    /// A camera below the surface is pushed back up to the clearance.
    #[test]
    fn raises_a_camera_that_is_under_the_ground() {
        let level = test_level();
        let (x, y) = (10.0f32, 10.0f32);
        let floor = level.get((x as i32, y as i32)).low();
        let mut cam = cam_at(Vec3::new(x, y, floor - 30.0));
        cam.keep_above_ground(&level, 4.0);
        assert!(
            (cam.loc.z - (floor + 4.0)).abs() < 1e-3,
            "expected {}, got {}",
            floor + 4.0,
            cam.loc.z
        );
    }

    /// ...and one already in the open is left where it is. `keep_above_ground`
    /// only ever raises, so it cannot fight the follow camera.
    #[test]
    fn leaves_a_camera_in_the_open_alone() {
        let level = test_level();
        let (x, y) = (10.0f32, 10.0f32);
        let floor = level.get((x as i32, y as i32)).low();
        let mut cam = cam_at(Vec3::new(x, y, floor + 200.0));
        cam.keep_above_ground(&level, 4.0);
        assert_eq!(cam.loc.z, floor + 200.0);
    }

    /// A camera buried in a far ridge is pulled toward the car, not just
    /// lifted onto the ridge top.
    #[test]
    fn avoid_terrain_pulls_toward_the_anchor() {
        let level = test_level();
        let car_floor = level.get((10, 10)).high();
        let ridge = level.get((10, 80)).high();
        let car = Vec3::new(10.0, 10.0, car_floor + 8.0);
        // Deep under the ridge top so a Z-only lift would perch on it.
        let mut cam = cam_at(Vec3::new(10.0, 80.0, ridge - 40.0));
        let before = cam.loc;
        cam.avoid_terrain(&level, car, 4.0);
        assert!(
            (cam.loc - car).length() + 1.0 < (before - car).length(),
            "expected a pull toward the car, got {before:?} -> {:?} (car_floor={car_floor}, ridge={ridge})",
            cam.loc
        );
        assert!(
            cam.loc.z >= level.floor_below(cam.loc) + 4.0 - 1e-3,
            "still needs floor clearance"
        );
    }

    #[test]
    fn open_line_of_sight_is_not_blocked() {
        let level = test_level();
        let floor = level.get((10, 10)).low();
        let cam = Vec3::new(10.0, 10.0, floor + 80.0);
        let car = Vec3::new(10.0, 40.0, floor + 8.0);
        assert!(!Camera::terrain_blocks_view(&level, cam, car, 4.0));
    }

    /// The case that makes this more than a `max`: under a slab, the
    /// surface below is the cave floor, not the slab top. Clamping to the
    /// slab top would fling the camera through the roof every time the car
    /// drove into a tunnel.
    #[test]
    fn a_camera_inside_a_cave_keeps_the_cave_floor() {
        let level = test_level();
        let dual = (0..level.size.1)
            .flat_map(|y| (0..level.size.0).map(move |x| (x, y)))
            .find_map(|(x, y)| match level.get((x, y)) {
                level::Texel::Dual { low, mid, high } => Some((x, y, low.0, mid, high.0)),
                level::Texel::Single(_) => None,
            })
            .expect("the test level is built with a double-level region");
        let (x, y, low, mid, high) = dual;
        assert!(mid < high, "the cave has to have a ceiling below the slab");

        // Sitting in the cave, comfortably under the ceiling.
        let inside = low + (mid - low) * 0.5;
        let mut cam = cam_at(Vec3::new(x as f32, y as f32, inside));
        cam.keep_above_ground(&level, 4.0);
        assert_eq!(cam.loc.z, inside, "a camera in the cave should not move");

        // Below the cave floor: raised to the floor, still under the slab.
        let mut cam = cam_at(Vec3::new(x as f32, y as f32, low - 10.0));
        cam.keep_above_ground(&level, 4.0);
        assert!((cam.loc.z - (low + 4.0)).abs() < 1e-3);
        assert!(cam.loc.z < mid, "should not be pushed through the ceiling");
    }

    /// A look-ahead chase camera sits behind the car and looks at a
    /// point in front of it, so the body is below the centre of the
    /// frame rather than filling it.
    #[test]
    fn look_ahead_points_past_the_car() {
        let target = Transform {
            scale: 1.0,
            rot: Quat::IDENTITY,
            disp: Vec3::ZERO,
        };
        let follow = Follow {
            angle_x: -std::f32::consts::FRAC_PI_2,
            offset: Vec3::new(0.0, 50.0, 20.0),
            speed: 20.0,
            look_ahead: 40.0,
        };
        let mut cam = cam_at(Vec3::new(0.0, 0.0, 200.0));
        for _ in 0..120 {
            cam.follow(&target, 1.0 / 60.0, &follow);
        }
        assert!(
            (cam.loc - Vec3::new(0.0, -50.0, 20.0)).length() < 0.1,
            "settled at {:?}, expected behind the car",
            cam.loc
        );
        let want = Vec3::new(0.0, 90.0, -20.0).normalize();
        let got = cam.dir();
        assert!(
            (got - want).length() < 0.05,
            "view {got:?} should look at the point ahead of the car ({want:?})"
        );
        // The car itself is below that look-at: the view ray's pitch is
        // shallower than the pitch down to the car's centre.
        let to_car = (target.disp - cam.loc).normalize();
        assert!(
            got.z > to_car.z,
            "looking at the car {to_car:?} would pitch down more than {got:?}"
        );
    }

    /// Wide-FOV chase used by the web build: one body-length back so the
    /// car does not shrink into the distance. Live play then adds one
    /// body-height on top of `offset.z`.
    #[test]
    fn a_close_chase_keeps_the_car_nearby() {
        let target = Transform {
            scale: 1.0,
            rot: Quat::IDENTITY,
            disp: Vec3::ZERO,
        };
        let follow = Follow {
            angle_x: 12f32.to_radians() - std::f32::consts::FRAC_PI_2,
            offset: Vec3::new(0.0, 24.0, 9.0),
            speed: 8.0,
            look_ahead: 10.0,
        };
        let mut cam = cam_at(Vec3::new(0.0, 0.0, 200.0));
        for _ in 0..120 {
            cam.follow(&target, 1.0 / 60.0, &follow);
        }
        let dist = (cam.loc - target.disp).length();
        assert!(
            dist < 30.0,
            "close chase settled {dist} away at {:?}",
            cam.loc
        );
        assert!(
            dist > 18.0,
            "should still sit behind the hull, got {dist} at {:?}",
            cam.loc
        );
    }

    /// The settings.ron overhead chase (240u up, tilted 30°) looks at a
    /// point 240·tan 30° ahead of the car; a look-ahead chase from the
    /// same spot aimed there sees exactly the same view, which is what
    /// lets [`Camera::chase`] blend from one into the other.
    #[test]
    fn equivalent_look_ahead_matches_the_authored_pitch() {
        let overhead = Follow {
            angle_x: 60f32.to_radians() - std::f32::consts::FRAC_PI_2,
            offset: Vec3::new(0.0, 0.0, 240.0),
            speed: 1.0,
            look_ahead: 0.0,
        };
        let l = overhead.equivalent_look_ahead();
        assert!((l - 240.0 * 30f32.to_radians().tan()).abs() < 1e-2, "{l}");
        let heading = Quat::from_rotation_z(std::f32::consts::PI);
        let pitched = heading * Quat::from_rotation_x(overhead.angle_x) * -Vec3::Z;
        let aimed =
            Camera::look_rotation(Vec3::new(0.0, l, 0.0) - heading * overhead.offset) * -Vec3::Z;
        assert!(
            (pitched - aimed).length() < 1e-4,
            "{pitched:?} vs {aimed:?}"
        );
        let close = Follow {
            look_ahead: 10.0,
            ..overhead
        };
        assert_eq!(close.equivalent_look_ahead(), 10.0);
    }

    /// The under-slab pose: behind and low in a tall gap, lower still
    /// in a tight one. The arm grows with the car, so the lens trails
    /// the tail by the same share of the car (the car keeps one size on
    /// screen) and never by less than 20u; a close chase stays close
    /// but not over the tail.
    #[test]
    fn under_slab_pose_fits_the_gap() {
        let overhead = Follow {
            angle_x: 60f32.to_radians() - std::f32::consts::FRAC_PI_2,
            offset: Vec3::new(0.0, 0.0, 240.0),
            speed: 1.0,
            look_ahead: 0.0,
        };
        let close = Follow {
            angle_x: 12f32.to_radians() - std::f32::consts::FRAC_PI_2,
            offset: Vec3::new(0.0, 24.0, 9.0),
            speed: 8.0,
            look_ahead: 10.0,
        };
        // The size the pose was tuned for keeps it.
        let tall = Camera::under_slab_follow(&overhead, 13.6, 130.0, 4.0, DEFAULT_SUBJECT_REACH);
        assert_eq!(tall.offset, Vec3::new(0.0, 40.0, 20.0));
        assert_eq!(tall.look_ahead, 10.0);
        assert_eq!(tall.speed, 3.0);
        let tuned_share = DEFAULT_SUBJECT_REACH / tall.offset.y;
        // A small buggy up to Fostral's OxidizeMonk (27u nose to centre)
        // and beyond.
        for reach in [8.0, 16.0, 27.2, 40.0] {
            let tall = Camera::under_slab_follow(&overhead, 13.6, 130.0, 4.0, reach);
            let tail_gap = tall.offset.y - reach;
            assert!(tail_gap >= 20.0 - 1e-4, "reach {reach}: {:?}", tall.offset);
            if reach >= DEFAULT_SUBJECT_REACH {
                // Same angular size of the car as the tuned pose.
                let share = reach / tall.offset.y;
                assert!((share - tuned_share).abs() < 1e-4, "reach {reach}: {share}");
            }
            // The tuned pitch, within half of the 108u of headroom.
            assert!((tall.offset.z / tall.offset.y - 0.5).abs() < 1e-4 || tall.offset.z == 8.0);
            assert!(tall.offset.z <= 54.0);
            // 20u of headroom: half of what is left after both clearances.
            let tight = Camera::under_slab_follow(&overhead, 10.0, 30.0, 4.0, reach);
            assert!((tight.offset.z - 6.0).abs() < 1e-5, "{:?}", tight.offset);
            assert_eq!(tight.offset.y, tall.offset.y);
            let near = Camera::under_slab_follow(&close, 13.6, 130.0, 4.0, reach);
            assert_eq!(near.offset.y, (reach + 20.0).max(24.0), "reach {reach}");
            assert!(near.offset.y <= tall.offset.y.max(24.0));
            assert_eq!(near.speed, 8.0);
        }
    }

    /// Parked under the test level's slab, the overhead chase ends up in
    /// the gap between floor and underside, behind the car, seeing it -
    /// not pinned against the ceiling straight overhead.
    #[test]
    fn chase_drops_under_the_test_slab() {
        let level = test_level();
        let (x, y) = (128.0, 128.0);
        let (low, mid) = match level.get((x as i32, y as i32)) {
            level::Texel::Dual { low, mid, .. } => (low.0, mid),
            level::Texel::Single(_) => panic!("the test slab is at the centre"),
        };
        let car = Vec3::new(x, y, low + 3.6);
        // Facing -Y: the test slab's underside rises toward +Y, behind.
        let target = Transform {
            disp: car,
            rot: Quat::from_rotation_z(std::f32::consts::PI),
            scale: 1.0,
        };
        let follow = chase_follows()[0];
        let mut cam = cam_at(car + Vec3::new(0.0, 0.0, 240.0));
        for _ in 0..180 {
            cam.chase(&level, &target, 1.0 / 60.0, &follow, 4.0);
            assert!(
                cam.loc.z < mid,
                "camera at {:?} above the underside {mid}",
                cam.loc
            );
        }
        assert!(cam.under_slab > 0.99);
        assert!(
            cam.loc.z >= low + 4.0 - 1e-3 && cam.loc.z <= mid - 4.0 + 1e-3,
            "camera {:?} outside the gap {low}..{mid}",
            cam.loc
        );
        // The test slab's cave is a lumpy pocket with the floor rising
        // right behind the car (Fostral's underpass tests check the
        // distance on a real one): low in the gap, not up at the roof.
        let off = cam.loc - car;
        assert!(off.length() <= 60.0, "{off:?} from the car");
        assert!(
            off.y >= 0.0,
            "should not swing in front of the car: {off:?}"
        );
        assert!(
            off.z < 0.5 * (mid - car.z),
            "should sit low in the {:.0}u gap, not up at the roof: {off:?}",
            mid - car.z
        );
        assert!(!Camera::terrain_blocks_view(&level, cam.loc, car, 4.0));
    }

    /// Away from any overhang `chase` is exactly the old
    /// follow → avoid_terrain → re-aim sequence, for both chases.
    #[test]
    fn chase_in_the_open_is_follow_then_avoid() {
        let level = test_level();
        for follow in chase_follows() {
            // Well over UNDER_SLAB_REACH from the slab at the centre.
            let mut new = cam_at(Vec3::new(10.0, 236.0, 300.0));
            let mut old = new;
            for frame in 0..240 {
                let t = frame as f32 / 60.0;
                let disp = Vec3::new(10.0 + 2.0 * t, 236.0 + 4.0 * t, 0.0);
                let disp = Vec3::new(disp.x, disp.y, level.floor_below(disp) + 4.0);
                let target = Transform {
                    disp,
                    rot: Quat::from_rotation_z(0.4 * t),
                    scale: 1.0,
                };
                new.chase(&level, &target, 1.0 / 60.0, &follow, 4.0);
                old.follow(&target, 1.0 / 60.0, &follow);
                old.avoid_terrain(&level, target.disp, 4.0);
                if follow.look_ahead.abs() > 1e-4 {
                    let look_at =
                        target.disp + old.chase_twist() * Vec3::new(0.0, follow.look_ahead, 0.0);
                    old.rot = Camera::look_rotation(look_at - old.loc);
                }
                assert_eq!(new.under_slab, 0.0);
                assert_eq!(new.loc, old.loc, "frame {frame}");
                assert_eq!(new.rot, old.rot, "frame {frame}");
            }
        }
    }

    /// A ground of `Single` texels at `ground(x, y)`, with two-level
    /// columns `(low, delta, high)` (underside `low + delta * 8`) where
    /// `slab(x, y)` has one. Heights are world units (scale 1).
    fn painted_level(
        ground: impl Fn(i32, i32) -> u8,
        slab: impl Fn(i32, i32) -> Option<(u8, u8, u8)>,
    ) -> level::Level {
        let mut level = test_level();
        assert_eq!(level.altitude_scale(), 1.0);
        assert_eq!(level.geometry.delta_power, 3);
        let (w, h) = level.size;
        for y in 0..h {
            for x in (0..w).step_by(2) {
                let i = (y * w + x) as usize;
                if let Some((low, delta, high)) = slab(x, y) {
                    level.height[i] = low;
                    level.height[i + 1] = high;
                    level.meta[i] = crate::level::DOUBLE_LEVEL | (delta >> 2);
                    level.meta[i + 1] = crate::level::DOUBLE_LEVEL | (delta & 3);
                } else {
                    for (j, x) in [(i, x), (i + 1, x + 1)] {
                        level.height[j] = ground(x, y);
                        level.meta[j] = 0;
                    }
                }
            }
        }
        level
    }

    /// On a roof at 100 beside a taller overhang (underside 130) whose
    /// cave floor is a pit at 10, the car is not under a slab: the
    /// under-slab pose must not kick in. On the pit floor it is.
    #[test]
    fn a_taller_overhang_beside_a_roof_is_not_cover() {
        let slab = |x: i32, _y: i32| (x >= 140).then_some((10, 15, 200));
        let roof = painted_level(|_, _| 100, slab);
        match roof.get((150, 128)) {
            level::Texel::Dual { low, mid, high } => {
                assert_eq!((low.0, mid, high.0), (10.0, 130.0, 200.0))
            }
            level::Texel::Single(_) => panic!("the slab should be two-level"),
        }
        let car = Vec3::new(138.0, 128.0, 103.6);
        assert!(Camera::under_slab_at(&roof, car).is_none());
        assert_eq!(Camera::slab_cover(&roof, car, 4.0).0, 0.0);

        let pit = painted_level(|_, _| 10, slab);
        let car = Vec3::new(138.0, 128.0, 13.6);
        assert!(Camera::under_slab_at(&pit, car).is_some());
        assert!(Camera::slab_cover(&pit, car, 4.0).0 > 0.99);
    }

    /// An arch (rock 45-75 over a gap down to 5) between the camera and
    /// a car outside it: a sight line threading the gap is clear, one
    /// through the arch's rock is not.
    #[test]
    fn a_sight_line_through_an_arch_gap_is_clear() {
        let arch = painted_level(
            |_, _| 5,
            |x, _| (120..140).contains(&x).then_some((5, 5, 75)),
        );
        match arch.get((130, 128)) {
            level::Texel::Dual { low, mid, high } => {
                assert_eq!((low.0, mid, high.0), (5.0, 45.0, 75.0))
            }
            level::Texel::Single(_) => panic!("the arch should be two-level"),
        }
        let car = Vec3::new(150.0, 128.0, 8.6);
        assert!(Camera::under_slab_at(&arch, car).is_none());
        assert!(!Camera::terrain_blocks_view(
            &arch,
            Vec3::new(110.0, 128.0, 25.0),
            car,
            4.0
        ));
        assert!(Camera::terrain_blocks_view(
            &arch,
            Vec3::new(110.0, 128.0, 90.0),
            car,
            4.0
        ));
    }

    /// Up on the arch's top, the car is not under it: no under-slab
    /// blend, even judged from the gap floor 70u below.
    #[test]
    fn on_top_of_an_arch_is_not_cover() {
        let arch = painted_level(
            |_, _| 5,
            |x, _| (120..140).contains(&x).then_some((5, 5, 75)),
        );
        let car = Vec3::new(130.0, 128.0, 78.6);
        assert_eq!(Camera::slab_cover(&arch, car, 4.0).0, 0.0);
        let beside = Vec3::new(150.0, 128.0, 85.0);
        assert_eq!(Camera::slab_cover(&arch, beside, 4.0).0, 0.0);
        // The gap floor under that flying car is covered: the old anchor.
        let floor = Vec3::new(150.0, 128.0, 5.0);
        assert!(Camera::slab_cover(&arch, floor, 4.0).0 > 0.5);
    }

    /// Under a slab with a cliff right behind the car, the low pose
    /// behind it is in rock: the offset swings up over the car at full
    /// arm instead of folding onto it. With no wall it is unchanged.
    #[test]
    fn backed_to_a_wall_the_slab_pose_swings_up() {
        // Cave (floor 10, underside 130) everywhere; a cliff up to 200
        // for y < 120.
        let wall = painted_level(|_, _| 200, |_, y| (y >= 120).then_some((10, 15, 200)));
        let open = painted_level(|_, _| 200, |_, _| Some((10, 15, 200)));
        let car = Vec3::new(128.0, 128.0, 13.6);
        let follow = chase_follows()[0];
        let pose =
            Camera::under_slab_follow(&follow, car.z, 130.0, 4.0, DEFAULT_SUBJECT_REACH).offset;
        // Facing +Y: behind is -Y, the cliff 8u back.
        let swung = Camera::unblocked_slab_offset(&wall, car, 0.0, pose, 130.0, 4.0);
        assert!(swung.z > pose.z + 10.0, "{swung:?} from {pose:?}");
        assert!((swung.length() - pose.length()).abs() < 1e-3, "{swung:?}");
        let behind = car + Quat::from_rotation_z(std::f32::consts::PI) * swung;
        assert!(Camera::free_arm(&wall, car + Vec3::Z * 4.0, behind, 4.0, car.z) >= 1.0);
        assert_eq!(
            Camera::unblocked_slab_offset(&open, car, 0.0, pose, 130.0, 4.0),
            pose
        );

        // The whole chase keeps its distance (it used to settle ~12u off).
        let target = Transform {
            disp: car,
            rot: Quat::IDENTITY,
            scale: 1.0,
        };
        // For the tuned size and an OxidizeMonk-sized car, the lens ends
        // up clear of the car's reach by 20u or more.
        for (follow, reach) in chase_follows()
            .into_iter()
            .flat_map(|f| [(f, DEFAULT_SUBJECT_REACH), (f, 27.2)])
        {
            let mut cam = cam_at(car + Vec3::new(0.0, 0.0, 240.0));
            let subject = Subject::unanchored(&target, reach);
            for _ in 0..240 {
                cam.chase_anchored(&wall, &target, subject, 1.0 / 60.0, &follow, 4.0);
            }
            let d = (cam.loc - car).length();
            assert!(
                d - reach >= 20.0,
                "reach {reach}: camera {:?} only {d:.1} from the car",
                cam.loc
            );
            assert!(cam.loc.z < 130.0 - 4.0 + 1e-3);
            assert!(!Camera::terrain_blocks_view(&wall, cam.loc, car, 4.0));
        }
    }

    /// A lens over the floor of a crevice with a wall 6u off rises over
    /// the wall top, as the original's eye is always over every voxel;
    /// one over open ground, or 10u from the wall, is left alone.
    #[test]
    fn an_eye_next_to_a_crevice_wall_goes_over_it() {
        let crevice = painted_level(
            |x, _| if (122..134).contains(&x) { 0 } else { 60 },
            |_, _| None,
        );
        let mut cam = cam_at(Vec3::new(128.0, 128.0, 20.0));
        cam.keep_eye_clear(&crevice, 4.0);
        assert_eq!(cam.loc.z, 64.0);

        let open = painted_level(|_, _| 0, |_, _| None);
        let mut cam = cam_at(Vec3::new(128.0, 128.0, 20.0));
        cam.keep_eye_clear(&open, 4.0);
        assert_eq!(cam.loc.z, 20.0);

        let wide = painted_level(
            |x, _| if (110..146).contains(&x) { 0 } else { 60 },
            |_, _| None,
        );
        let mut cam = cam_at(Vec3::new(120.0, 128.0, 20.0));
        cam.keep_eye_clear(&wide, 4.0);
        assert_eq!(cam.loc.z, 20.0);
    }

    /// A car at the bottom of a deep round pit: the chase arm hits the
    /// pit wall, and the pull used to leave the lens low against it with
    /// the wall filling the screen. It ends over the rim instead, with
    /// nothing solid around it.
    #[test]
    fn a_chase_into_a_deep_pit_stays_over_the_rim() {
        let pit = painted_level(
            |x, y| {
                let (dx, dy) = (x as f32 - 128.0, y as f32 - 128.0);
                if dx * dx + dy * dy < 20.0 * 20.0 {
                    0
                } else {
                    100
                }
            },
            |_, _| None,
        );
        let car = Vec3::new(128.0, 128.0, 3.6);
        let target = Transform {
            disp: car,
            rot: Quat::IDENTITY,
            scale: 1.0,
        };
        for follow in chase_follows() {
            let mut cam = cam_at(car + Vec3::new(0.0, 0.0, 240.0));
            for _ in 0..240 {
                cam.chase(&pit, &target, 1.0 / 60.0, &follow, 4.0);
                let ring = Camera::eye_footprint(cam.loc);
                assert!(
                    ring.iter().all(|&p| !pit.solid_at(p)),
                    "{follow:?}: rock around the lens at {:?}",
                    cam.loc
                );
            }
            let (dx, dy) = (cam.loc.x - 128.0, cam.loc.y - 128.0);
            assert!(
                dx * dx + dy * dy < 12.0 * 12.0 || cam.loc.z >= 104.0 - 1e-3,
                "{follow:?}: lens low against the wall at {:?}",
                cam.loc
            );
        }
    }

    /// The overhead chase of `config/settings.ron` (240u up, tilted 30°,
    /// speed 1) and the web build's close look-ahead chase.
    fn chase_follows() -> [Follow; 2] {
        [
            Follow {
                angle_x: 60f32.to_radians() - std::f32::consts::FRAC_PI_2,
                offset: Vec3::new(0.0, 0.0, 240.0),
                speed: 1.0,
                look_ahead: 0.0,
            },
            Follow {
                angle_x: 12f32.to_radians() - std::f32::consts::FRAC_PI_2,
                offset: Vec3::new(0.0, 24.0, 9.0),
                speed: 8.0,
                look_ahead: 10.0,
            },
        ]
    }

    fn wrap_angle(a: f32) -> f32 {
        let pi = std::f32::consts::PI;
        (a + pi).rem_euclid(2.0 * pi) - pi
    }

    /// Yaw of the camera's view direction, flattened.
    fn view_yaw(cam: &Camera) -> f32 {
        let d = cam.dir();
        f32::atan2(-d.x, d.y)
    }

    /// Follow `pose(t)` for `frames` 60 fps frames after settling on
    /// `pose(0)`; returns the worst per-frame turn of the camera view and
    /// of the chase heading, and the largest heading drift from the start.
    fn chase(follow: &Follow, frames: usize, pose: impl Fn(f32) -> Quat) -> (f32, f32, f32) {
        let dt = 1.0 / 60.0;
        let mut target = Transform {
            disp: Vec3::new(100.0, 100.0, 50.0),
            rot: pose(0.0),
            scale: 1.0,
        };
        let mut cam = cam_at(target.disp + Vec3::new(0.0, 0.0, 200.0));
        for _ in 0..600 {
            cam.follow(&target, dt, follow);
        }
        let start = cam.chase_yaw.unwrap();
        let (mut view, mut heading, mut drift) = (view_yaw(&cam), start, 0.0f32);
        let (mut worst_view, mut worst_heading) = (0.0f32, 0.0f32);
        for i in 1..=frames {
            let t = i as f32 / frames as f32;
            target.rot = pose(t);
            // A jump arc: the car keeps moving while it tumbles.
            target.disp += Vec3::new(0.0, 0.5, 1.0 - 2.0 * t);
            cam.follow(&target, dt, follow);
            let (v, h) = (view_yaw(&cam), cam.chase_yaw.unwrap());
            worst_view = worst_view.max(wrap_angle(v - view).abs());
            worst_heading = worst_heading.max(wrap_angle(h - heading).abs());
            drift = drift.max(wrap_angle(h - start).abs());
            (view, heading) = (v, h);
        }
        (worst_view, worst_heading, drift)
    }

    /// A car flipping end over end, or rolling over, in a 2 s jump: the
    /// raw forward axis turns through 180° in a frame at the vertical,
    /// which used to spin the chase camera round. The heading now holds.
    #[test]
    fn chase_heading_survives_a_flip_and_a_roll() {
        let yaw = 0.7;
        let tau = std::f32::consts::TAU;
        for follow in chase_follows() {
            for (name, axis) in [("pitch", Vec3::X), ("roll", Vec3::Y)] {
                let (view, heading, drift) = chase(&follow, 120, |t| {
                    Quat::from_rotation_z(yaw) * Quat::from_axis_angle(axis, tau * t)
                });
                assert!(
                    drift < 1f32.to_radians(),
                    "{name}: heading drifted {}°",
                    drift.to_degrees()
                );
                assert!(heading < 1f32.to_radians(), "{name}: heading jumped");
                assert!(
                    view < 3f32.to_radians(),
                    "{name}: view turned {}° in one frame",
                    view.to_degrees()
                );
            }
        }
    }

    /// Tumbling about a skewed axis, spinning as well, passes through
    /// upside-down poses whose heading is meaningless: the camera may
    /// come round, but never by more than a few degrees a frame.
    #[test]
    fn chase_heading_stays_continuous_through_a_tumble() {
        let tau = std::f32::consts::TAU;
        for follow in chase_follows() {
            for (axis, spin) in [
                (Vec3::new(1.0, 1.0, 0.0), 0.0),
                (Vec3::new(1.0, 0.3, 0.2), 0.0),
                (Vec3::new(0.2, 1.0, 0.5), 1.5),
                (Vec3::new(1.0, -0.6, 0.0), -2.0),
            ] {
                let axis = axis.normalize();
                let (view, heading, _) = chase(&follow, 120, |t| {
                    Quat::from_rotation_z(0.3 + spin * t)
                        * Quat::from_axis_angle(axis, 1.5 * tau * t)
                });
                // `CHASE_YAW_RATE` is half a turn a second: 3° a 60 fps frame.
                assert!(
                    heading <= 3.01f32.to_radians(),
                    "axis {axis:?}: heading turned {}° in one frame",
                    heading.to_degrees()
                );
                assert!(
                    view < 3.5f32.to_radians(),
                    "axis {axis:?}: view turned {}° in one frame",
                    view.to_degrees()
                );
            }
        }
    }

    /// Normal driving is untouched: a car turning on a bumpy road gets
    /// the camera of the old flattened-forward heading, frame by frame,
    /// within the fraction of a degree by which the two headings differ
    /// when the car is pitched and rolled at once.
    #[test]
    fn chase_heading_follows_a_normal_turn() {
        let dt = 1.0 / 60.0;
        for follow in chase_follows() {
            let mut target = Transform {
                disp: Vec3::new(100.0, 100.0, 5.0),
                rot: Quat::IDENTITY,
                scale: 1.0,
            };
            let mut cam = cam_at(Vec3::new(100.0, 100.0, 200.0));
            let mut old = cam_at(Vec3::new(100.0, 100.0, 200.0));
            for i in 0..240 {
                let t = i as f32 * dt;
                // 86°/s, swinging to -52°/s, over 10° pitch and roll bumps.
                let yaw = 1.5 * t - 0.3 * t * t;
                target.rot = Quat::from_rotation_z(yaw)
                    * Quat::from_rotation_x(0.17 * (7.0 * t).sin())
                    * Quat::from_rotation_y(0.17 * (5.0 * t).cos());
                target.disp += target.rot * Vec3::new(0.0, 0.4, 0.0);
                cam.follow(&target, dt, &follow);
                old_follow(&mut old, &target, dt, &follow);
                let heading = cam.chase_yaw.unwrap();
                let (want, _) = Camera::chase_heading(target.rot);
                assert!(
                    wrap_angle(heading - want).abs() < 1e-4,
                    "frame {i}: heading lags"
                );
                assert!(
                    cam.rot.angle_between(old.rot) < 1f32.to_radians()
                        && (cam.loc - old.loc).length() < 0.5,
                    "frame {i}: {:?} / {:?} strayed from the old camera {:?} / {:?}",
                    cam.rot,
                    cam.loc,
                    old.rot,
                    old.loc
                );
            }
        }
    }

    /// `Camera::follow` before the chase heading: the flattened forward.
    fn old_follow(cam: &mut Camera, target: &Transform, dt: f32, follow: &Follow) {
        let mut front = target.rot * Vec3::Y;
        front.z = 0.0;
        let twist = Quat::from_rotation_arc(Vec3::Y, front.normalize());
        let heading = Quat::from_rotation_z(std::f32::consts::PI) * twist;
        let location = target.disp + heading * follow.offset;
        let k = (dt * -follow.speed).exp();
        cam.loc = location * (1.0 - k) + cam.loc * k;
        let rotation = if follow.look_ahead.abs() > 1e-4 {
            let look_at = target.disp + twist * Vec3::new(0.0, follow.look_ahead, 0.0);
            Camera::look_rotation(look_at - cam.loc)
        } else {
            heading * Quat::from_rotation_x(follow.angle_x)
        };
        cam.rot = rotation.slerp(cam.rot, k);
    }
}

#[cfg(test)]
mod reconcile_tests {
    use super::*;

    fn at(disp: Vec3, yaw: f32) -> Transform {
        Transform {
            disp,
            rot: Quat::from_rotation_z(yaw),
            scale: 1.0,
        }
    }

    #[test]
    fn blends_small_position_error() {
        let server = at(Vec3::new(10.0, 0.0, 0.0), 0.0);
        let mut local = at(Vec3::new(10.0 + 2.0, 0.0, 0.0), 0.0);
        assert!(
            !local.reconcile_world_state(&server),
            "2u is under SNAP_POS"
        );
        let expected = 2.0 * (1.0 - Transform::WORLD_STATE_BLEND);
        assert!((local.disp.x - (10.0 + expected)).abs() < 1e-5);
        assert_eq!(local.disp.y, 0.0);
    }

    #[test]
    fn snaps_large_position_error() {
        let server = at(Vec3::new(10.0, 0.0, 0.0), 0.0);
        let mut local = at(Vec3::new(10.0 + 20.0, 0.0, 0.0), 0.0);
        assert!(local.reconcile_world_state(&server));
        assert_eq!(local, server);
    }

    #[test]
    fn snaps_large_rotation_error() {
        let server = at(Vec3::ZERO, 0.0);
        let mut local = at(Vec3::ZERO, 1.2); // ~69°
        assert!(local.reconcile_world_state(&server));
        assert_eq!(local, server);
    }

    #[test]
    fn blends_small_rotation_error() {
        let server = at(Vec3::ZERO, 0.0);
        let yaw = 0.2; // ~11°
        let mut local = at(Vec3::ZERO, yaw);
        assert!(!local.reconcile_world_state(&server));
        let got = local.rot.angle_between(server.rot);
        let expect = yaw * (1.0 - Transform::WORLD_STATE_BLEND);
        assert!((got - expect).abs() < 1e-4, "got {got}, expect {expect}");
    }
}
