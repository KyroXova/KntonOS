//! Gyroscopic Aiming Subsystem & Natural Sensitivity Scale Integrator.
//!
//! Provides high-precision gyro aiming for handheld gaming PCs (Steam Deck, ROG Ally, Legion Go):
//! - Pitch / Yaw angular velocity integration with sub-pixel delta accumulation
//! - Natural Sensitivity Scale (NSS - 1:1 real-world angular to in-game pixel ratio)
//! - Roll-to-yaw translation and axis inversion options
//! - Sub-pixel fraction preservation eliminating micro-movement truncation

use crate::drivers::input::imu::PI;
pub const TWO_PI: f32 = PI * 2.0;

// -----------------------------------------------------------------------------
// Natural Sensitivity Scale & Configuration
// -----------------------------------------------------------------------------

/// Default calibration: 3600 mouse counts per full 360° revolution (10 counts per degree).
pub const DEFAULT_DOTS_PER_REVOLUTION: f32 = 3600.0;

/// Conversion factor: dots per radian at default calibration (3600 / 2*PI).
pub const DEFAULT_DOTS_PER_RADIAN: f32 = DEFAULT_DOTS_PER_REVOLUTION / TWO_PI;

/// Configuration for gyroscopic aiming and sensitivity scaling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GyroAimConfig {
    /// Natural sensitivity multiplier (1.0 = 1:1 real-to-virtual angular rotation).
    pub natural_sensitivity: f32,
    /// Calibration factor: virtual mouse pixels/dots per full 360° revolution.
    pub dots_per_revolution: f32,
    /// Invert vertical pitch axis.
    pub invert_pitch: bool,
    /// Invert horizontal yaw axis.
    pub invert_yaw: bool,
    /// Yaw-roll blend factor (0.0 = pure yaw, 1.0 = pure roll, 0.25 = default handheld hybrid).
    pub roll_to_yaw_blend: f32,
}

impl GyroAimConfig {
    pub const fn default_handheld() -> Self {
        Self {
            natural_sensitivity: 1.0,
            dots_per_revolution: DEFAULT_DOTS_PER_REVOLUTION,
            invert_pitch: false,
            invert_yaw: false,
            roll_to_yaw_blend: 0.0, // Pure yaw by default
        }
    }

    #[inline(always)]
    pub fn dots_per_radian(&self) -> f32 {
        self.dots_per_revolution / TWO_PI
    }
}

// -----------------------------------------------------------------------------
// Gyro Angular Velocity Integrator with Sub-Pixel Accumulator
// -----------------------------------------------------------------------------

/// High-precision angular velocity integrator with sub-pixel residual tracking.
#[derive(Debug, Clone)]
pub struct GyroAimIntegrator {
    pub config: GyroAimConfig,
    /// Sub-pixel accumulator for horizontal X-axis (yaw).
    pub subpixel_x: f32,
    /// Sub-pixel accumulator for vertical Y-axis (pitch).
    pub subpixel_y: f32,
    /// Cumulative integrated yaw angle in radians.
    pub total_yaw_rad: f32,
    /// Cumulative integrated pitch angle in radians.
    pub total_pitch_rad: f32,
}

impl GyroAimIntegrator {
    pub const fn new(config: GyroAimConfig) -> Self {
        Self {
            config,
            subpixel_x: 0.0,
            subpixel_y: 0.0,
            total_yaw_rad: 0.0,
            total_pitch_rad: 0.0,
        }
    }

    /// Resets sub-pixel accumulators and integrated angle history.
    pub fn reset(&mut self) {
        self.subpixel_x = 0.0;
        self.subpixel_y = 0.0;
        self.total_yaw_rad = 0.0;
        self.total_pitch_rad = 0.0;
    }

    /// Integrates angular velocity (rad/s) over timestep `dt_sec` and outputs integer mouse deltas (dx, dy).
    ///
    /// `gyro_rad_per_sec`: `(pitch_x, yaw_y, roll_z)`.
    pub fn integrate(&mut self, gyro_rad_per_sec: (f32, f32, f32), dt_sec: f32) -> (i32, i32) {
        let (raw_pitch, raw_yaw, raw_roll) = gyro_rad_per_sec;

        // Apply roll-to-yaw blend for handheld posture comfort
        let blended_yaw = raw_yaw * (1.0 - self.config.roll_to_yaw_blend) - raw_roll * self.config.roll_to_yaw_blend;

        let mut pitch_vel = raw_pitch;
        let mut yaw_vel = blended_yaw;

        if self.config.invert_pitch {
            pitch_vel = -pitch_vel;
        }
        if self.config.invert_yaw {
            yaw_vel = -yaw_vel;
        }

        // Calculate angular delta over timestep: dTheta = omega * dt
        let delta_yaw_rad = yaw_vel * dt_sec;
        let delta_pitch_rad = pitch_vel * dt_sec;

        self.total_yaw_rad += delta_yaw_rad;
        self.total_pitch_rad += delta_pitch_rad;

        // Calculate continuous pixel delta using Natural Sensitivity Scale:
        // pixels = delta_rad * dots_per_radian * sensitivity
        let scale = self.config.dots_per_radian() * self.config.natural_sensitivity;
        let delta_pixels_x = delta_yaw_rad * scale;
        let delta_pixels_y = delta_pitch_rad * scale;

        // Add to sub-pixel accumulator
        self.subpixel_x += delta_pixels_x;
        self.subpixel_y += delta_pixels_y;

        // Truncate whole integer pixels for cursor/camera event dispatch
        let dx = self.subpixel_x as i32;
        let dy = self.subpixel_y as i32;

        // Preserve fractional sub-pixel remainder for subsequent updates
        self.subpixel_x -= dx as f32;
        self.subpixel_y -= dy as f32;

        (dx, dy)
    }
}

// -----------------------------------------------------------------------------
// Unit Tests & Verification
// -----------------------------------------------------------------------------

/// Unit test verifying gyro angular velocity integration, sub-pixel residual preservation, and Natural Sensitivity Scale.
pub fn verify_gyro_angular_velocity_and_natural_sensitivity() -> bool {
    let config = GyroAimConfig::default_handheld();
    let mut integrator = GyroAimIntegrator::new(config);

    // 1. Verify 1:1 Natural Sensitivity Scale
    // Rotating at 1.0 rad/s yaw for 1.0 second with 3600 dots/rev (3600 / 2*PI dots/rad):
    // Expected pixels = 1.0 * (3600 / 2*PI) * 1.0 = ~572.9578 pixels
    let (dx1, dy1) = integrator.integrate((0.0, 1.0, 0.0), 1.0);
    if dx1 != 572 || dy1 != 0 {
        return false;
    }
    // Subpixel remainder should be ~0.9578
    if integrator.subpixel_x < 0.95 || integrator.subpixel_x > 0.97 {
        return false;
    }

    // 2. Verify sub-pixel accumulation across fine micro-movements
    // Small rotational speed: 0.005 rad/s over 0.01s (10ms frame):
    // delta_rad = 0.00005 rad. scale = ~572.96 -> delta_pixels = ~0.0286 pixels/frame
    integrator.reset();
    let mut accumulated_dx = 0;
    // Over 100 frames (1 second total at 0.05 rad/s):
    // Total pixels = 0.05 * 572.9578 = ~28.6479 pixels
    for _ in 0..100 {
        let (dx, _) = integrator.integrate((0.0, 0.05, 0.0), 0.01);
        accumulated_dx += dx;
    }
    if accumulated_dx != 28 {
        return false; // Subpixel accumulation preserved all fractional deltas!
    }
    if integrator.subpixel_x < 0.60 || integrator.subpixel_x > 0.70 {
        return false;
    }

    // 3. Verify Pitch & Yaw separation with axis inversion
    let mut inv_config = GyroAimConfig::default_handheld();
    inv_config.invert_pitch = true;
    inv_config.invert_yaw = true;
    let mut inv_integrator = GyroAimIntegrator::new(inv_config);

    let (dx_inv, dy_inv) = inv_integrator.integrate((1.0, 1.0, 0.0), 1.0);
    // Inverted: positive angular velocities yield negative pixel deltas
    if dx_inv != -572 || dy_inv != -572 {
        return false;
    }

    // 4. Verify full 360° revolution: 2*PI radians
    integrator.reset();
    let (dx_rev, _) = integrator.integrate((0.0, TWO_PI, 0.0), 1.0);
    if dx_rev != 3600 {
        return false; // Exactly 3600 counts per 360° revolution
    }

    true
}
