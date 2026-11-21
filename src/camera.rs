//! Deterministic CPU free-flight camera, resolution-scale math, and camera-centered
//! terrain block streaming window selection.

use glam::{Mat4, Vec3};

use crate::settings::{MAX_RESOLUTION_SCALE_PCT, MIN_RESOLUTION_SCALE_PCT};

pub const MAX_PITCH_RAD: f32 = 1.50;

#[derive(Debug, Clone, Copy, Default)]
pub struct CameraInput {
    pub forward: bool,  // W
    pub backward: bool, // S
    pub left: bool,     // A
    pub right: bool,    // D
    pub ascend: bool,   // Space or E
    pub descend: bool,  // Q or Left Ctrl
    pub fast: bool,     // Shift
    pub look_left: bool,
    pub look_right: bool,
    pub look_up: bool,
    pub look_down: bool,
    pub mouse_dx: f32,
    pub mouse_dy: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FreeCamera {
    pub position: Vec3,
    /// Radians around +Y (0 faces +Z).
    pub yaw: f32,
    /// Radians above/below horizon (clamped to `[-MAX_PITCH_RAD, MAX_PITCH_RAD]`).
    pub pitch: f32,
    pub fov_y_rad: f32,
    pub near: f32,
    pub far: f32,
    pub move_speed: f32,
    pub fast_multiplier: f32,
    pub key_turn_speed: f32,
    pub mouse_sensitivity: f32,
}

impl Default for FreeCamera {
    fn default() -> Self {
        Self {
            position: Vec3::new(128.0, 160.0, 128.0),
            yaw: 0.75,
            pitch: -0.35,
            fov_y_rad: 60.0f32.to_radians(),
            near: 0.5,
            far: 4096.0,
            move_speed: 48.0,
            fast_multiplier: 3.5,
            key_turn_speed: 1.8,
            mouse_sensitivity: 0.003,
        }
    }
}

impl FreeCamera {
    pub fn forward_vec(&self) -> Vec3 {
        let cos_p = self.pitch.cos();
        Vec3::new(
            self.yaw.sin() * cos_p,
            self.pitch.sin(),
            self.yaw.cos() * cos_p,
        )
        .normalize()
    }

    pub fn right_vec(&self) -> Vec3 {
        self.forward_vec().cross(Vec3::Y).normalize_or_zero()
    }

    /// Advance the camera deterministically for fixed timestep `dt` (seconds).
    pub fn step(&mut self, input: &CameraInput, dt: f32) {
        if dt <= 0.0 || !dt.is_finite() {
            return;
        }
        let mut yaw_delta = 0.0;
        let mut pitch_delta = 0.0;
        if input.look_left {
            yaw_delta += self.key_turn_speed * dt;
        }
        if input.look_right {
            yaw_delta -= self.key_turn_speed * dt;
        }
        if input.look_up {
            pitch_delta += self.key_turn_speed * dt;
        }
        if input.look_down {
            pitch_delta -= self.key_turn_speed * dt;
        }
        yaw_delta -= input.mouse_dx * self.mouse_sensitivity;
        pitch_delta -= input.mouse_dy * self.mouse_sensitivity;

        self.yaw = (self.yaw + yaw_delta).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch + pitch_delta).clamp(-MAX_PITCH_RAD, MAX_PITCH_RAD);

        let fwd = self.forward_vec();
        let right = self.right_vec();
        let mut move_dir = Vec3::ZERO;
        if input.forward {
            move_dir += fwd;
        }
        if input.backward {
            move_dir -= fwd;
        }
        if input.right {
            move_dir += right;
        }
        if input.left {
            move_dir -= right;
        }
        if input.ascend {
            move_dir += Vec3::Y;
        }
        if input.descend {
            move_dir -= Vec3::Y;
        }
        if move_dir.length_squared() > 0.0 {
            let speed = if input.fast {
                self.move_speed * self.fast_multiplier
            } else {
                self.move_speed
            };
            self.position += move_dir.normalize() * speed * dt;
        }
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let safe_aspect = if aspect.is_finite() && aspect > 0.01 {
            aspect
        } else {
            16.0 / 9.0
        };
        let target = self.position + self.forward_vec();
        let view = Mat4::look_at_rh(self.position, target, Vec3::Y);
        let proj = Mat4::perspective_rh(self.fov_y_rad, safe_aspect, self.near, self.far);
        proj * view
    }
}

/// Compute the 3D offscreen render target size for a given window size and resolution
/// scale percentage (`10..=200`, default `100`). Never returns 0 on either axis.
pub fn scaled_render_target(window_w: u32, window_h: u32, scale_pct: u16) -> (u32, u32) {
    let clamped = scale_pct.clamp(MIN_RESOLUTION_SCALE_PCT, MAX_RESOLUTION_SCALE_PCT) as u64;
    let w = ((window_w.max(1) as u64) * clamped / 100).max(1) as u32;
    let h = ((window_h.max(1) as u64) * clamped / 100).max(1) as u32;
    (w, h)
}

/// Select `(row, col)` terrain blocks within `radius_blocks` of `camera_pos`
/// using Angelica centered world coordinates (`X in [-W/2, +W/2]`, `Z in [-L/2, +L/2]`,
/// row 0 at `+L/2` descending toward `-L/2`), sorted nearest-first.
pub fn streaming_blocks_angelica(
    camera_pos: Vec3,
    block_size: f32,
    rows: u32,
    cols: u32,
    radius_blocks: i32,
) -> Vec<(u32, u32)> {
    if rows == 0 || cols == 0 || block_size <= 0.0 || !block_size.is_finite() {
        return Vec::new();
    }
    let half_w = (cols as f32) * block_size * 0.5;
    let half_l = (rows as f32) * block_size * 0.5;
    let cam_col = ((camera_pos.x + half_w) / block_size).floor() as i32;
    let cam_row = ((half_l - camera_pos.z) / block_size).floor() as i32;
    let radius = radius_blocks.max(0);
    let r_min = (cam_row - radius).max(0) as u32;
    let r_max = (cam_row + radius).min(rows as i32 - 1).max(0) as u32;
    let c_min = (cam_col - radius).max(0) as u32;
    let c_max = (cam_col + radius).min(cols as i32 - 1).max(0) as u32;

    let mut selected = Vec::new();
    for r in r_min..=r_max {
        for c in c_min..=c_max {
            let dr = (r as i32 - cam_row) as i64;
            let dc = (c as i32 - cam_col) as i64;
            let dist_sq = dr * dr + dc * dc;
            selected.push((dist_sq, r, c));
        }
    }
    selected.sort_by_key(|&(d, r, c)| (d, r, c));
    selected.into_iter().map(|(_, r, c)| (r, c)).collect()
}

/// Select `(row, col)` terrain blocks within `radius_blocks` of `camera_pos`,
/// sorted nearest-first by squared distance to the camera.
pub fn streaming_blocks(
    camera_pos: Vec3,
    block_size: f32,
    rows: u32,
    cols: u32,
    radius_blocks: i32,
) -> Vec<(u32, u32)> {
    if rows == 0 || cols == 0 || block_size <= 0.0 || !block_size.is_finite() {
        return Vec::new();
    }
    let cam_col = (camera_pos.x / block_size).floor() as i32;
    let cam_row = (camera_pos.z / block_size).floor() as i32;
    let radius = radius_blocks.max(0);
    let r_min = (cam_row - radius).max(0) as u32;
    let r_max = (cam_row + radius).min(rows as i32 - 1).max(0) as u32;
    let c_min = (cam_col - radius).max(0) as u32;
    let c_max = (cam_col + radius).min(cols as i32 - 1).max(0) as u32;

    let mut selected = Vec::new();
    for r in r_min..=r_max {
        for c in c_min..=c_max {
            let dr = (r as i32 - cam_row) as i64;
            let dc = (c as i32 - cam_col) as i64;
            let dist_sq = dr * dr + dc * dc;
            selected.push((dist_sq, r, c));
        }
    }
    selected.sort_by_key(|&(d, r, c)| (d, r, c));
    selected.into_iter().map(|(_, r, c)| (r, c)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_scale_calculation_clamps_and_scales_cleanly() {
        assert_eq!(scaled_render_target(1920, 1080, 100), (1920, 1080));
        assert_eq!(scaled_render_target(1920, 1080, 50), (960, 540));
        assert_eq!(scaled_render_target(1920, 1080, 200), (3840, 2160));
        assert_eq!(scaled_render_target(1920, 1080, 10), (192, 108));
        // Out-of-range clamping
        assert_eq!(scaled_render_target(1000, 1000, 1), (100, 100));
        assert_eq!(scaled_render_target(1000, 1000, 500), (2000, 2000));
        // Zero-dimension safety
        assert_eq!(scaled_render_target(0, 0, 100), (1, 1));
    }

    #[test]
    fn fixed_timestep_camera_movement_and_look_controls() {
        let mut cam = FreeCamera {
            position: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            move_speed: 10.0,
            fast_multiplier: 3.0,
            ..Default::default()
        };
        // Forward along +Z at yaw=0, pitch=0 for 1.0s
        cam.step(
            &CameraInput {
                forward: true,
                ..Default::default()
            },
            1.0,
        );
        assert!((cam.position.z - 10.0).abs() < 1e-4);

        // Ascend with Shift boost for 0.5s -> +15.0 on Y
        cam.step(
            &CameraInput {
                ascend: true,
                fast: true,
                ..Default::default()
            },
            0.5,
        );
        assert!((cam.position.y - 15.0).abs() < 1e-4);

        // Arrow-key pitch up clamps cleanly at MAX_PITCH_RAD
        for _ in 0..20 {
            cam.step(
                &CameraInput {
                    look_up: true,
                    ..Default::default()
                },
                0.25,
            );
        }
        assert!((cam.pitch - MAX_PITCH_RAD).abs() < 1e-5);
    }

    #[test]
    fn camera_centered_streaming_selects_nearest_blocks_first() {
        let blocks = streaming_blocks(Vec3::new(160.0, 50.0, 224.0), 64.0, 16, 16, 1);
        // Camera is in col = floor(160/64) = 2, row = floor(224/64) = 3
        assert_eq!(blocks.len(), 9);
        assert_eq!(blocks[0], (3, 2));
        assert!(blocks.contains(&(2, 1)));
        assert!(blocks.contains(&(4, 3)));
    }
}
