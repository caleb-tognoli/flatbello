//! An orbit camera around a target point, z up. Math is f64; matrices go to the GPU as f32.

use core::f64::consts::FRAC_PI_2;

use flatbello_core::{Mat4, Point3, Vec3};

/// Vertical field of view.
const FOV_Y: f64 = 35.0 * core::f64::consts::PI / 180.0;
/// Pitch stays just short of straight up or down, where the up vector degenerates.
const MAX_PITCH: f64 = FRAC_PI_2 - 0.01;

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub target: Point3,
    pub distance: f64,
    /// Rotation about +z, radians.
    pub yaw: f64,
    /// Elevation above the xy plane, radians.
    pub pitch: f64,
    /// Radius of the model, for clip planes.
    pub scene_radius: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            target: Point3::ORIGIN,
            distance: 100.0,
            yaw: -FRAC_PI_2 + 0.6,
            pitch: 0.5,
            scene_radius: 50.0,
        }
    }
}

impl Camera {
    /// A camera looking at a sphere of `radius` around `center` from the default direction.
    pub fn framing(center: Point3, radius: f64) -> Camera {
        let radius = if radius > 0.0 { radius } else { 1.0 };
        Camera {
            target: center,
            distance: radius / (FOV_Y / 2.0).sin() * 1.15,
            scene_radius: radius,
            ..Camera::default()
        }
    }

    /// Unit vector from the target towards the eye.
    fn back(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(cp * cy, cp * sy, sp)
    }

    pub fn eye(&self) -> Point3 {
        self.target + self.back() * self.distance
    }

    /// Camera right and up directions in world space.
    fn basis(&self) -> (Vec3, Vec3) {
        let forward = -self.back();
        let right = forward.cross(Vec3::Z).normalized().unwrap_or(Vec3::X);
        let up = right.cross(forward);
        (right, up)
    }

    /// Rotates by a drag of `dx`, `dy` pixels.
    pub fn orbit(&mut self, dx: f64, dy: f64) {
        self.yaw -= dx * 0.008;
        self.pitch = (self.pitch + dy * 0.008).clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// Moves the target with a drag of `dx`, `dy` pixels in a view `height` pixels tall, so the
    /// point under the cursor follows it.
    pub fn pan(&mut self, dx: f64, dy: f64, height: f64) {
        let (right, up) = self.basis();
        let per_pixel = 2.0 * self.distance * (FOV_Y / 2.0).tan() / height.max(1.0);
        self.target = self.target + (right * -dx + up * dy) * per_pixel;
    }

    /// Zooms in for positive `steps`.
    pub fn zoom(&mut self, steps: f64) {
        let min = self.scene_radius * 1e-3;
        self.distance = (self.distance * (-steps * 0.15).exp()).max(min);
    }

    /// View matrix, world to camera, with world coordinates taken relative to `origin`.
    pub fn view(&self, origin: Point3) -> Mat4 {
        let eye = self.eye() - origin;
        let f = -self.back();
        let (s, u) = self.basis();
        Mat4::from_cols([
            [s.x, u.x, -f.x, 0.0],
            [s.y, u.y, -f.y, 0.0],
            [s.z, u.z, -f.z, 0.0],
            [-s.dot(eye), -u.dot(eye), f.dot(eye), 1.0],
        ])
    }

    /// Right-handed perspective projection with depth in [0, 1], as wgpu expects.
    pub fn projection(&self, aspect: f64) -> Mat4 {
        let near = (self.distance - self.scene_radius * 2.0).max(self.distance * 1e-3);
        let far = self.distance + self.scene_radius * 2.0;
        let f = 1.0 / (FOV_Y / 2.0).tan();
        let a = if aspect > 0.0 { aspect } else { 1.0 };
        Mat4::from_cols([
            [f / a, 0.0, 0.0, 0.0],
            [0.0, f, 0.0, 0.0],
            [0.0, 0.0, far / (near - far), -1.0],
            [0.0, 0.0, near * far / (near - far), 0.0],
        ])
    }

    /// The direction light comes from: over the viewer's shoulder.
    pub fn light_dir(&self) -> Vec3 {
        let (right, up) = self.basis();
        (self.back() + up * 0.5 + right * 0.3)
            .normalized()
            .unwrap_or(Vec3::Z)
    }
}

/// `m` as f32 columns for a uniform buffer.
pub fn to_f32(m: &Mat4) -> [[f32; 4]; 4] {
    m.cols().map(|c| c.map(|v| v as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(m: &Mat4, p: Point3) -> (f64, f64, f64) {
        let c = m.cols();
        let v = [p.x, p.y, p.z, 1.0];
        let row = |r: usize| (0..4).map(|k| c[k][r] * v[k]).sum::<f64>();
        let w = row(3);
        (row(0) / w, row(1) / w, row(2) / w)
    }

    #[test]
    fn target_projects_to_the_centre() {
        let cam = Camera::framing(Point3::new(10.0, 20.0, 5.0), 30.0);
        let m = cam.projection(1.5) * cam.view(Point3::ORIGIN);
        let (x, y, z) = clip(&m, cam.target);
        assert!(x.abs() < 1e-9 && y.abs() < 1e-9);
        assert!((0.0..1.0).contains(&z));
    }

    #[test]
    fn origin_shift_does_not_change_the_image() {
        let cam = Camera::framing(Point3::new(1000.0, -500.0, 20.0), 10.0);
        let p = Point3::new(1003.0, -498.0, 25.0);
        let a = clip(&(cam.projection(1.0) * cam.view(Point3::ORIGIN)), p);
        let o = Point3::new(1000.0, -500.0, 20.0);
        let shifted = Point3::new(p.x - o.x, p.y - o.y, p.z - o.z);
        let b = clip(&(cam.projection(1.0) * cam.view(o)), shifted);
        assert!((a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9 && (a.2 - b.2).abs() < 1e-9);
    }

    #[test]
    fn whole_scene_is_inside_the_depth_range() {
        let cam = Camera::framing(Point3::ORIGIN, 10.0);
        let m = cam.projection(1.0) * cam.view(Point3::ORIGIN);
        for p in [
            Point3::new(10.0, 0.0, 0.0),
            Point3::new(0.0, -10.0, 0.0),
            Point3::new(0.0, 0.0, 10.0),
        ] {
            let (_, _, z) = clip(&m, p);
            assert!((0.0..=1.0).contains(&z), "{z}");
        }
    }

    #[test]
    fn screen_up_is_world_up_at_zero_pitch() {
        let mut cam = Camera::framing(Point3::ORIGIN, 10.0);
        cam.pitch = 0.0;
        let m = cam.projection(1.0) * cam.view(Point3::ORIGIN);
        let (_, y, _) = clip(&m, Point3::new(0.0, 0.0, 5.0));
        assert!(y > 0.0);
    }

    #[test]
    fn orbit_clamps_pitch_and_zoom_stays_positive() {
        let mut cam = Camera::default();
        cam.orbit(0.0, 1e6);
        assert!(cam.pitch <= MAX_PITCH);
        cam.zoom(1e6);
        assert!(cam.distance > 0.0);
    }

    #[test]
    fn pan_moves_the_target_sideways() {
        let mut cam = Camera::framing(Point3::ORIGIN, 10.0);
        let eye_to_target = cam.target - cam.eye();
        cam.pan(100.0, 0.0, 500.0);
        let moved = cam.target - Point3::ORIGIN;
        assert!(moved.length() > 0.0);
        assert!(moved.dot(eye_to_target).abs() < 1e-9);
    }
}
