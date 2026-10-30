//! Motion Assist Dynamic Angle Governor & Steady-Aim Tremor Dampener.
//!
//! Provides advanced motion control processing for handheld gaming PCs:
//! - Steady-aim physiological tremor dampening (quadratic attenuation for <0.5 deg/sec micro-aiming)
//! - High-speed flick acceleration curve (scaling sensitivity at >15 deg/sec for instant 180° turns)
//! - Handheld lean angle detection and tilt-to-steer assistance (with deadzone & steering ramp)

use crate::drivers::input::imu::DEG_TO_RAD;

// -----------------------------------------------------------------------------
// Steady-Aim Tremor Dampening & Flick Acceleration Curves
// -----------------------------------------------------------------------------

/// Configuration for motion assist filtering and acceleration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionAssistConfig {
    /// Velocity threshold below which tremor dampening is applied (rad/s).
    /// Default: 0.5 deg/sec = ~0.008726 rad/s.
    pub tremor_threshold_rad_s: f32,
    /// Velocity threshold above which flick acceleration kicks in (rad/s).
    /// Default: 15.0 deg/sec = ~0.2618 rad/s.
    pub flick_threshold_rad_s: f32,
    /// Maximum acceleration multiplier for high-speed flicks (e.g. 2.5x to 3.0x).
    pub flick_max_multiplier: f32,
    /// Tilt-to-steer deadzone angle in degrees (default 5.0°).
    pub tilt_deadzone_deg: f32,
    /// Tilt-to-steer saturation angle in degrees (default 35.0° = 100% steer).
    pub tilt_saturation_deg: f32,
}

impl MotionAssistConfig {
    pub const fn default_handheld() -> Self {
        Self {
            tremor_threshold_rad_s: 0.5 * DEG_TO_RAD,   // 0.5 deg/s (~0.00873 rad/s)
            flick_threshold_rad_s: 15.0 * DEG_TO_RAD,  // 15.0 deg/s (~0.2618 rad/s)
            flick_max_multiplier: 2.5,
            tilt_deadzone_deg: 5.0,
            tilt_saturation_deg: 35.0,
        }
    }
}

/// Lean angle direction for tactical peek or steering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeanDirection {
    Neutral,
    LeanLeft,
    LeanRight,
}

// -----------------------------------------------------------------------------
// Motion Assist Dynamic Governor
// -----------------------------------------------------------------------------

/// Dynamic angle governor applying tremor dampening, flick curves, and tilt steering.
#[derive(Debug, Clone)]
pub struct MotionAssistGovernor {
    pub config: MotionAssistConfig,
}

impl MotionAssistGovernor {
    pub const fn new(config: MotionAssistConfig) -> Self {
        Self { config }
    }

    /// Evaluates dynamic sensitivity multiplier for a given angular velocity magnitude.
    ///
    /// - For |v| < tremor_threshold: Smooth quadratic dampening (v / threshold)^2 to eliminate hand jitter.
    /// - For tremor_threshold <= |v| <= flick_threshold: 1.0 (linear Natural Sensitivity Scale).
    /// - For |v| > flick_threshold: Accelerated sensitivity curve up to flick_max_multiplier.
    pub fn compute_speed_multiplier(&self, speed_rad_s: f32) -> f32 {
        if speed_rad_s <= 0.0 {
            return 0.0;
        }

        if speed_rad_s < self.config.tremor_threshold_rad_s {
            // Quadratic dampening: f(v) = (v / v_thresh)^2
            let ratio = speed_rad_s / self.config.tremor_threshold_rad_s;
            ratio * ratio
        } else if speed_rad_s <= self.config.flick_threshold_rad_s {
            // Linear 1:1 Natural Sensitivity
            1.0
        } else {
            // High-speed flick acceleration
            let excess = speed_rad_s - self.config.flick_threshold_rad_s;
            // Linear ramp over next 45 deg/sec (0.785 rad/s)
            let ramp_range = 45.0 * DEG_TO_RAD;
            let boost_progress = (excess / ramp_range).min(1.0);
            1.0 + boost_progress * (self.config.flick_max_multiplier - 1.0)
        }
    }

    /// Filters and conditions angular velocity `(pitch_rad_s, yaw_rad_s)`:
    /// applies steady-aim tremor dampening and high-speed flick acceleration.
    pub fn filter_aim_velocity(&self, pitch_rad_s: f32, yaw_rad_s: f32) -> (f32, f32) {
        let speed = crate::graphics::sqrt_f32(pitch_rad_s * pitch_rad_s + yaw_rad_s * yaw_rad_s);
        let multiplier = self.compute_speed_multiplier(speed);

        (pitch_rad_s * multiplier, yaw_rad_s * multiplier)
    }

    /// Evaluates tilt-to-steer angle and normalized steering analog value (-1.0 to +1.0).
    pub fn evaluate_tilt_steer(&self, roll_angle_deg: f32) -> (LeanDirection, f32) {
        let abs_roll = if roll_angle_deg < 0.0 { -roll_angle_deg } else { roll_angle_deg };

        if abs_roll <= self.config.tilt_deadzone_deg {
            return (LeanDirection::Neutral, 0.0);
        }

        let direction = if roll_angle_deg > 0.0 {
            LeanDirection::LeanRight
        } else {
            LeanDirection::LeanLeft
        };

        let span = self.config.tilt_saturation_deg - self.config.tilt_deadzone_deg;
        let active_roll = (abs_roll - self.config.tilt_deadzone_deg).min(span);
        let normalized = (active_roll / span).clamp(0.0, 1.0);

        let signed_output = if roll_angle_deg > 0.0 {
            normalized
        } else {
            -normalized
        };

        (direction, signed_output)
    }
}

// -----------------------------------------------------------------------------
// Unit Tests & Verification
// -----------------------------------------------------------------------------

/// Unit test verifying steady-aim tremor dampening, flick acceleration, and tilt-to-steer governor.
pub fn verify_steady_aim_and_flick_acceleration() -> bool {
    let config = MotionAssistConfig::default_handheld();
    let governor = MotionAssistGovernor::new(config);

    // 1. Verify Steady-Aim Tremor Dampening
    // Tremor threshold = 0.5 deg/s (~0.00873 rad/s)
    // Test at 0.25 deg/s (half threshold): multiplier should be (0.5)^2 = 0.25
    let half_thresh = 0.25 * DEG_TO_RAD;
    let mult_half = governor.compute_speed_multiplier(half_thresh);
    if (mult_half - 0.25).abs() > 0.02 {
        return false;
    }

    // Test at 0.1 deg/s (1/5th threshold): multiplier should be (0.2)^2 = 0.04 (96% tremor eliminated)
    let tiny_thresh = 0.1 * DEG_TO_RAD;
    let mult_tiny = governor.compute_speed_multiplier(tiny_thresh);
    if (mult_tiny - 0.04).abs() > 0.01 {
        return false;
    }

    // 2. Verify Baseline Natural Aiming (no dampening, no acceleration)
    // Test at 5.0 deg/sec: multiplier must be exactly 1.0
    let normal_speed = 5.0 * DEG_TO_RAD;
    let mult_normal = governor.compute_speed_multiplier(normal_speed);
    if (mult_normal - 1.0).abs() > 0.0001 {
        return false;
    }

    // 3. Verify High-Speed Flick Acceleration
    // Flick threshold = 15.0 deg/sec. Test below threshold at 12 deg/sec -> 1.0
    let sub_flick = 12.0 * DEG_TO_RAD;
    if (governor.compute_speed_multiplier(sub_flick) - 1.0).abs() > 0.0001 {
        return false;
    }

    // Test at 60 deg/sec (> 15 + 45 = 60 deg/sec saturation): should reach max multiplier (2.5x)
    let max_flick = 60.0 * DEG_TO_RAD;
    let mult_flick = governor.compute_speed_multiplier(max_flick);
    if (mult_flick - 2.5).abs() > 0.02 {
        return false;
    }

    // Test velocity filtering
    let (_p_flick, y_flick) = governor.filter_aim_velocity(0.0, 60.0 * DEG_TO_RAD);
    if (y_flick / (60.0 * DEG_TO_RAD) - 2.5).abs() > 0.02 {
        return false;
    }

    // 4. Verify Tilt-to-Steer / Lean Angle Assistance
    // 4a. Within deadzone (3.0° <= 5.0°) -> Neutral, 0.0 steer
    let (dir_dead, steer_dead) = governor.evaluate_tilt_steer(3.0);
    if dir_dead != LeanDirection::Neutral || steer_dead != 0.0 {
        return false;
    }

    // 4b. Right lean at 20°: (20 - 5) / (35 - 5) = 15 / 30 = 0.50 (50% right steer)
    let (dir_right, steer_right) = governor.evaluate_tilt_steer(20.0);
    if dir_right != LeanDirection::LeanRight || (steer_right - 0.50).abs() > 0.01 {
        return false;
    }

    // 4c. Left lean at -35° (saturation): -1.0 full left steer
    let (dir_left, steer_left) = governor.evaluate_tilt_steer(-35.0);
    if dir_left != LeanDirection::LeanLeft || (steer_left - (-1.0)).abs() > 0.01 {
        return false;
    }

    true
}
