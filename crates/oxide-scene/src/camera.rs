//! The orbit camera and the picking ray.
//!
//! Both projections share one orbit state (target, yaw, pitch, distance), so
//! switching between them never moves what the user is looking at. Picking uses
//! the same view-projection the renderer draws with: a click and the 3D image
//! cannot disagree about where a pixel is.

use glam::{Mat4, Vec2, Vec3};

use crate::manifest::Aabb;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Projection {
    #[default]
    Perspective,
    Orthographic,
}

impl Projection {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Projection::Perspective => "透视",
            Projection::Orthographic => "正交",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub direction: Vec3,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub target: Vec3,
    /// Radians, around +Y.
    pub yaw: f32,
    /// Radians above the horizon, clamped to keep the up vector stable.
    pub pitch: f32,
    pub distance: f32,
    pub projection: Projection,
    /// Vertical field of view, radians (perspective).
    pub fov_y: f32,
    /// Visible height in world units (orthographic).
    pub ortho_height: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            target: Vec3::new(0.0, 1.5, 0.0),
            yaw: 0.72,
            pitch: 0.62,
            distance: 46.0,
            projection: Projection::Perspective,
            fov_y: 0.72,
            ortho_height: 30.0,
        }
    }
}

const MIN_PITCH: f32 = 0.08;
const MAX_PITCH: f32 = 1.45;
const MIN_DISTANCE: f32 = 3.0;
const MAX_DISTANCE: f32 = 120.0;

impl Camera {
    #[must_use]
    pub fn eye(&self) -> Vec3 {
        let horizontal = self.distance * self.pitch.cos();
        self.target
            + Vec3::new(
                horizontal * self.yaw.sin(),
                self.distance * self.pitch.sin(),
                horizontal * self.yaw.cos(),
            )
    }

    #[must_use]
    pub fn view(&self) -> Mat4 {
        Mat4::look_at_rh(self.eye(), self.target, Vec3::Y)
    }

    #[must_use]
    pub fn projection_matrix(&self, aspect: f32) -> Mat4 {
        match self.projection {
            Projection::Perspective => {
                Mat4::perspective_rh(self.fov_y, aspect.max(0.05), 0.1, 400.0)
            }
            Projection::Orthographic => {
                let half_h = self.ortho_height * 0.5;
                let half_w = half_h * aspect.max(0.05);
                Mat4::orthographic_rh(-half_w, half_w, -half_h, half_h, 0.1, 400.0)
            }
        }
    }

    #[must_use]
    pub fn view_projection(&self, aspect: f32) -> Mat4 {
        self.projection_matrix(aspect) * self.view()
    }

    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * 0.008;
        self.pitch = (self.pitch + dy * 0.008).clamp(MIN_PITCH, MAX_PITCH);
    }

    /// Drag in points; the world moves with the pointer.
    pub fn pan(&mut self, dx: f32, dy: f32, viewport_height: f32) {
        let world_per_point = match self.projection {
            Projection::Orthographic => self.ortho_height / viewport_height.max(1.0),
            Projection::Perspective => {
                2.0 * self.distance * (self.fov_y * 0.5).tan() / viewport_height.max(1.0)
            }
        };
        // Pan across the floor: a vertical drag moves the target away from the
        // camera along the ground, never up into the air, so the factory stays
        // on its floor plane however the camera is tilted.
        let forward = (self.target - self.eye()).normalize_or_zero();
        let right = forward.cross(Vec3::Y).normalize_or_zero();
        let ground_forward = Vec3::Y.cross(right).normalize_or_zero();
        self.target += (-right * dx + ground_forward * dy) * world_per_point;
        self.target.x = self.target.x.clamp(-60.0, 60.0);
        self.target.z = self.target.z.clamp(-60.0, 60.0);
    }

    /// `delta` is in wheel notches; positive zooms in.
    pub fn zoom(&mut self, delta: f32) {
        let factor = (-delta * 0.12).exp();
        self.distance = (self.distance * factor).clamp(MIN_DISTANCE, MAX_DISTANCE);
        self.ortho_height = (self.ortho_height * factor).clamp(4.0, 90.0);
    }

    pub fn toggle_projection(&mut self) {
        self.projection = match self.projection {
            Projection::Perspective => Projection::Orthographic,
            Projection::Orthographic => Projection::Perspective,
        };
    }

    /// Frame a box: centre the target and pick a distance that fits its radius.
    pub fn focus(&mut self, box_: Aabb) {
        self.target = box_.center();
        let radius = box_.radius().max(1.0);
        self.distance = (radius / (self.fov_y * 0.5).tan() * 2.1).clamp(MIN_DISTANCE, MAX_DISTANCE);
        self.ortho_height = (radius * 2.4).max(4.0);
    }

    /// A ray through a normalized device coordinate (-1..1, y up).
    #[must_use]
    pub fn ray(&self, ndc: Vec2, aspect: f32) -> Ray {
        let inverse = self.view_projection(aspect).inverse();
        let near = inverse * glam::Vec4::new(ndc.x, ndc.y, 0.0, 1.0);
        let far = inverse * glam::Vec4::new(ndc.x, ndc.y, 1.0, 1.0);
        let near = near.truncate() / near.w;
        let far = far.truncate() / far.w;
        Ray {
            origin: near,
            direction: (far - near).normalize_or_zero(),
        }
    }

    /// Where a world point lands on screen: `None` behind the camera.
    #[must_use]
    pub fn project(&self, point: Vec3, aspect: f32) -> Option<Vec2> {
        let clip = self.view_projection(aspect) * point.extend(1.0);
        if clip.w <= 0.0001 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(Vec2::new(ndc.x, ndc.y))
    }
}

/// Nearest positive hit of `ray` on the box, if any.
#[must_use]
pub fn ray_aabb(ray: &Ray, box_: Aabb) -> Option<f32> {
    let mut t_min = 0.0_f32;
    let mut t_max = f32::INFINITY;
    for axis in 0..3 {
        let origin = ray.origin[axis];
        let direction = ray.direction[axis];
        if direction.abs() < 1e-8 {
            if origin < box_.min[axis] || origin > box_.max[axis] {
                return None;
            }
            continue;
        }
        let inv = 1.0 / direction;
        let mut t1 = (box_.min[axis] - origin) * inv;
        let mut t2 = (box_.max[axis] - origin) * inv;
        if t1 > t2 {
            std::mem::swap(&mut t1, &mut t2);
        }
        t_min = t_min.max(t1);
        t_max = t_max.min(t2);
        if t_min > t_max {
            return None;
        }
    }
    (t_max >= 0.0).then_some(t_min.max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> Camera {
        Camera {
            target: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.5,
            distance: 20.0,
            ..Camera::default()
        }
    }

    #[test]
    fn the_eye_orbits_around_the_target_at_the_set_distance() {
        let camera = camera();
        assert!((camera.eye().distance(Vec3::ZERO) - 20.0).abs() < 0.001);
        let mut zoomed = camera;
        zoomed.zoom(3.0);
        assert!(zoomed.eye().distance(Vec3::ZERO) < 20.0);
    }

    #[test]
    fn panning_moves_the_target_and_stays_on_the_floor_plane() {
        let mut camera = camera();
        let before = camera.target;
        camera.pan(10.0, 10.0, 800.0);
        assert_ne!(camera.target, before);
        assert_eq!(camera.target.y, before.y);
    }

    #[test]
    fn a_ray_through_the_centre_hits_a_box_at_the_target() {
        let camera = camera();
        let box_ = Aabb::new([-1.0, -1.0, -1.0], [1.0, 1.0, 1.0]);
        let hit = ray_aabb(&camera.ray(Vec2::ZERO, 1.6), box_);
        assert!(hit.is_some(), "centre ray misses the box at the target");
    }

    #[test]
    fn a_ray_away_from_the_box_misses() {
        let camera = camera();
        let box_ = Aabb::new([-1.0, -1.0, -1.0], [1.0, 1.0, 1.0]);
        assert!(ray_aabb(&camera.ray(Vec2::new(0.99, 0.99), 1.6), box_).is_none());
    }

    #[test]
    fn both_projections_project_the_target_to_the_centre() {
        let mut camera = camera();
        for projection in [Projection::Perspective, Projection::Orthographic] {
            camera.projection = projection;
            let ndc = camera.project(Vec3::ZERO, 1.6).unwrap();
            assert!(ndc.length() < 0.001, "{projection:?}: {ndc:?}");
        }
    }

    #[test]
    fn focusing_frames_a_tool_with_a_closer_camera() {
        let mut camera = Camera::default();
        camera.focus(Aabb::new([-1.0, 0.0, -1.0], [1.0, 3.0, 1.0]));
        assert!(camera.distance < Camera::default().distance);
        assert!((camera.target.y - 1.5).abs() < 0.001);
    }
}
