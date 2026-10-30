//! Gyroscopic Aiming Subsystem & Natural Sensitivity Scale Integrator.
//!
//! Provides high-precision gyro aiming for handheld gaming PCs (Steam Deck, ROG Ally, Legion Go):
//! - Pitch / Yaw angular velocity integration with sub-pixel delta accumulation
//! - Natural Sensitivity Scale (NSS - 1:1 real-world angular to in-game pixel ratio)
//! - Capacitive touch thumbstick / touchpad and analog trigger ADS activation modes
//! - Automatic stationary zero-bias calibration drift removal
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

/// Gyro aiming activation modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GyroActivationMode {
    /// Gyro is permanently active.
    AlwaysOn,
    /// Activated when right thumbstick capacitive cap is touched (Steam Deck / handheld standard).
    RightThumbstickTouch,
    /// Activated when right touchpad surface is touched.
    RightTouchpadTouch,
    /// Activated when Left Trigger (LT / L2 ADS) is pulled beyond threshold (default 20%).
    LeftTriggerSoftPull,
    /// Activated when Right Trigger (RT / R2 Hipfire/Fire) is pulled beyond threshold.
    RightTriggerSoftPull,
    /// Activated while a designated grip/paddle button is held.
    ButtonHeld,
    /// Inverted: Gyro active by default, temporarily disabled while thumbstick is touched (ratcheting).
    InvertThumbstickRatcheting,
}

/// Snapshot of analog triggers and capacitive touch sensors for activation evaluation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GamepadTriggerState {
    pub left_trigger_analog: f32, // 0.0 to 1.0
    pub right_trigger_analog: f32, // 0.0 to 1.0
    pub right_thumbstick_touched: bool,
    pub right_touchpad_touched: bool,
    pub activation_button_held: bool,
}

impl GamepadTriggerState {
    pub const fn empty() -> Self {
        Self {
            left_trigger_analog: 0.0,
            right_trigger_analog: 0.0,
            right_thumbstick_touched: false,
            right_touchpad_touched: false,
            activation_button_held: false,
        }
    }
}

/// Configuration for gyroscopic aiming, activation modes, and sensitivity scaling.
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
    /// Selected activation trigger mode.
    pub activation_mode: GyroActivationMode,
    /// Trigger pull analog threshold (e.g. 0.20 = 20% pull).
    pub trigger_threshold: f32,
}

impl GyroAimConfig {
    pub const fn default_handheld() -> Self {
        Self {
            natural_sensitivity: 1.0,
            dots_per_revolution: DEFAULT_DOTS_PER_REVOLUTION,
            invert_pitch: false,
            invert_yaw: false,
            roll_to_yaw_blend: 0.0, // Pure yaw by default
            activation_mode: GyroActivationMode::AlwaysOn,
            trigger_threshold: 0.20,
        }
    }

    #[inline(always)]
    pub fn dots_per_radian(&self) -> f32 {
        self.dots_per_revolution / TWO_PI
    }

    /// Evaluates whether the gyro should be active given the current controller trigger/touch inputs.
    pub fn is_active(&self, triggers: &GamepadTriggerState) -> bool {
        match self.activation_mode {
            GyroActivationMode::AlwaysOn => true,
            GyroActivationMode::RightThumbstickTouch => triggers.right_thumbstick_touched,
            GyroActivationMode::RightTouchpadTouch => triggers.right_touchpad_touched,
            GyroActivationMode::LeftTriggerSoftPull => triggers.left_trigger_analog >= self.trigger_threshold,
            GyroActivationMode::RightTriggerSoftPull => triggers.right_trigger_analog >= self.trigger_threshold,
            GyroActivationMode::ButtonHeld => triggers.activation_button_held,
            GyroActivationMode::InvertThumbstickRatcheting => !triggers.right_thumbstick_touched,
        }
    }
}

// -----------------------------------------------------------------------------
// Zero-Bias Auto-Calibration & Angular Velocity Integrator
// -----------------------------------------------------------------------------

/// Automatic stationary zero-bias calibrator to eliminate IMU temperature drift.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZeroBiasCalibrator {
    pub bias_pitch: f32,
    pub bias_yaw: f32,
    pub bias_roll: f32,
    pub stationary_time_sec: f32,
    pub stationary_threshold_rad_s: f32,
}

impl ZeroBiasCalibrator {
    pub const fn new() -> Self {
        Self {
            bias_pitch: 0.0,
            bias_yaw: 0.0,
            bias_roll: 0.0,
            stationary_time_sec: 0.0,
            stationary_threshold_rad_s: 0.015, // ~0.85 deg/sec maximum stationary threshold
        }
    }

    /// Feeds raw angular velocity and updates zero-bias estimate when controller is resting.
    pub fn update(&mut self, raw: (f32, f32, f32), dt_sec: f32) {
        let (p, y, r) = raw;
        let mag = (p * p + y * y + r * r) as f32;

        if mag < self.stationary_threshold_rad_s * self.stationary_threshold_rad_s {
            self.stationary_time_sec += dt_sec;
            // After resting stationary for > 0.25 seconds, adaptively calibrate bias
            if self.stationary_time_sec >= 0.25 {
                let alpha = (2.0 * dt_sec).min(0.10); // Low-pass filter baseline
                self.bias_pitch = self.bias_pitch * (1.0 - alpha) + p * alpha;
                self.bias_yaw = self.bias_yaw * (1.0 - alpha) + y * alpha;
                self.bias_roll = self.bias_roll * (1.0 - alpha) + r * alpha;
            }
        } else {
            self.stationary_time_sec = 0.0;
        }
    }

    /// Corrects raw readings by subtracting estimated zero-bias.
    #[inline(always)]
    pub fn correct(&self, raw: (f32, f32, f32)) -> (f32, f32, f32) {
        (
            raw.0 - self.bias_pitch,
            raw.1 - self.bias_yaw,
            raw.2 - self.bias_roll,
        )
    }
}

/// High-precision angular velocity integrator with sub-pixel residual tracking.
#[derive(Debug, Clone)]
pub struct GyroAimIntegrator {
    pub config: GyroAimConfig,
    pub calibrator: ZeroBiasCalibrator,
    /// Sub-pixel accumulator for horizontal X-axis (yaw).
    pub subpixel_x: f32,
    /// Sub-pixel accumulator for vertical Y-axis (pitch).
    pub subpixel_y: f32,
    /// Cumulative integrated yaw angle in radians.
    pub total_yaw_rad: f32,
    /// Cumulative integrated pitch angle in radians.
    pub total_pitch_rad: f32,
    /// Previous active state to detect activation edges and prevent jump on resume.
    pub was_active: bool,
}

impl GyroAimIntegrator {
    pub const fn new(config: GyroAimConfig) -> Self {
        Self {
            config,
            calibrator: ZeroBiasCalibrator::new(),
            subpixel_x: 0.0,
            subpixel_y: 0.0,
            total_yaw_rad: 0.0,
            total_pitch_rad: 0.0,
            was_active: false,
        }
    }

    /// Resets sub-pixel accumulators and integrated angle history.
    pub fn reset(&mut self) {
        self.subpixel_x = 0.0;
        self.subpixel_y = 0.0;
        self.total_yaw_rad = 0.0;
        self.total_pitch_rad = 0.0;
        self.was_active = false;
    }

    /// Main processing step: updates calibration, checks activation, and integrates angular velocity.
    pub fn process_sample(
        &mut self,
        raw_gyro: (f32, f32, f32),
        triggers: &GamepadTriggerState,
        dt_sec: f32,
    ) -> (i32, i32) {
        // 1. Update zero-bias drift calibration
        self.calibrator.update(raw_gyro, dt_sec);

        // 2. Evaluate activation mode
        let active = self.config.is_active(triggers);

        if !active {
            self.subpixel_x = 0.0;
            self.subpixel_y = 0.0;
            self.was_active = false;
            return (0, 0);
        }

        // On activation edge, clear sub-pixels to prevent discontinuous jump
        if !self.was_active {
            self.subpixel_x = 0.0;
            self.subpixel_y = 0.0;
            self.was_active = true;
        }

        // 3. Subtract calibrated bias
        let corrected = self.calibrator.correct(raw_gyro);

        // 4. Integrate corrected angular velocity
        self.integrate(corrected, dt_sec)
    }

    /// Integrates calibrated angular velocity (rad/s) over timestep `dt_sec` and outputs integer mouse deltas (dx, dy).
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

/// Unit test verifying capacitive touch thumbstick, trigger pull activation modes, and zero-bias calibration.
pub fn verify_activation_modes_and_calibration() -> bool {
    let mut triggers = GamepadTriggerState::empty();

    // 1. Verify RightThumbstickTouch mode
    let mut touch_config = GyroAimConfig::default_handheld();
    touch_config.activation_mode = GyroActivationMode::RightThumbstickTouch;
    let mut integrator = GyroAimIntegrator::new(touch_config);

    // Thumb not touching stick -> output must be (0, 0)
    triggers.right_thumbstick_touched = false;
    let (dx0, dy0) = integrator.process_sample((0.0, 1.0, 0.0), &triggers, 0.01);
    if dx0 != 0 || dy0 != 0 {
        return false;
    }

    // Thumb touches stick -> active output
    triggers.right_thumbstick_touched = true;
    let (dx1, _) = integrator.process_sample((0.0, 1.0, 0.0), &triggers, 0.01);
    if dx1 <= 0 {
        return false;
    }

    // 2. Verify LeftTriggerSoftPull mode (ADS aim)
    let mut lt_config = GyroAimConfig::default_handheld();
    lt_config.activation_mode = GyroActivationMode::LeftTriggerSoftPull;
    lt_config.trigger_threshold = 0.25; // 25% pull threshold
    let mut lt_integrator = GyroAimIntegrator::new(lt_config);

    // 15% pull (< 25%) -> inactive
    triggers.left_trigger_analog = 0.15;
    let (dx_lt0, _) = lt_integrator.process_sample((0.0, 1.0, 0.0), &triggers, 0.01);
    if dx_lt0 != 0 {
        return false;
    }

    // 35% pull (>= 25%) -> active
    triggers.left_trigger_analog = 0.35;
    let (dx_lt1, _) = lt_integrator.process_sample((0.0, 1.0, 0.0), &triggers, 0.01);
    if dx_lt1 <= 0 {
        return false;
    }

    // 3. Verify InvertThumbstickRatcheting mode
    let mut ratch_config = GyroAimConfig::default_handheld();
    ratch_config.activation_mode = GyroActivationMode::InvertThumbstickRatcheting;
    triggers.right_thumbstick_touched = false;
    if !ratch_config.is_active(&triggers) {
        return false; // Active when NOT touching
    }
    triggers.right_thumbstick_touched = true;
    if ratch_config.is_active(&triggers) {
        return false; // Disengaged when touching stick
    }

    // 4. Verify Zero-Bias drift calibration
    let cal_config = GyroAimConfig::default_handheld();
    let mut cal_integrator = GyroAimIntegrator::new(cal_config);
    let cal_triggers = GamepadTriggerState::empty();

    // Constant slow drift: 0.008 rad/s yaw (below 0.015 stationary threshold)
    // Over 1.0 second (100 steps of 10ms):
    for _ in 0..100 {
        cal_integrator.process_sample((0.0, 0.008, 0.0), &cal_triggers, 0.01);
    }
    // Bias should have adapted to ~0.008
    if (cal_integrator.calibrator.bias_yaw - 0.008).abs() > 0.002 {
        return false;
    }

    // Now, with bias corrected, stationary output deltas must be 0
    let (dx_drift, _) = cal_integrator.process_sample((0.0, 0.008, 0.0), &cal_triggers, 0.01);
    if dx_drift != 0 {
        return false;
    }

    true
}
