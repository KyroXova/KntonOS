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

// -----------------------------------------------------------------------------
// Multi-Axis Gyroscope Zero-Drift & Motion Aiming Governor (TASK-KNT-131)
// -----------------------------------------------------------------------------

/// Conversion factor: Degrees to Radians.
pub const DEG2RAD: f32 = PI / 180.0;
/// Conversion factor: Radians to Degrees.
pub const RAD2DEG: f32 = 180.0 / PI;

/// Tri-axial gyroscope angular velocity representation in physical degrees per second (°/s).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GyroAngularVelocity {
    /// Roll angular rate in °/s (rotation around longitudinal axis).
    pub roll_dps: f32,
    /// Pitch angular rate in °/s (rotation around lateral axis).
    pub pitch_dps: f32,
    /// Yaw angular rate in °/s (rotation around vertical axis).
    pub yaw_dps: f32,
    /// Capture timestamp in microseconds.
    pub timestamp_us: u64,
}

impl GyroAngularVelocity {
    /// Creates a new angular velocity reading.
    pub const fn new(roll_dps: f32, pitch_dps: f32, yaw_dps: f32, timestamp_us: u64) -> Self {
        Self {
            roll_dps,
            pitch_dps,
            yaw_dps,
            timestamp_us,
        }
    }

    /// Converts the angular velocity tuple to radians per second: (pitch, yaw, roll).
    #[inline(always)]
    pub fn to_rad_per_sec(&self) -> (f32, f32, f32) {
        (
            self.pitch_dps * DEG2RAD,
            self.yaw_dps * DEG2RAD,
            self.roll_dps * DEG2RAD,
        )
    }

    /// Computes the Euclidean angular velocity vector norm in °/s.
    #[inline(always)]
    pub fn magnitude_dps(&self) -> f32 {
        crate::graphics::sqrt_f32(
            self.roll_dps * self.roll_dps
                + self.pitch_dps * self.pitch_dps
                + self.yaw_dps * self.yaw_dps,
        )
    }
}

/// Persistent zero-bias calibration registers for IMU background bias compensation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GyroBiasRegisters {
    /// Zero-rate offset for roll axis in °/s.
    pub bias_roll_dps: f32,
    /// Zero-rate offset for pitch axis in °/s.
    pub bias_pitch_dps: f32,
    /// Zero-rate offset for yaw axis in °/s.
    pub bias_yaw_dps: f32,
    /// Number of stationary frames accumulated into current bias estimate.
    pub sample_count: u32,
    /// Flag indicating if background calibration converged.
    pub is_calibrated: bool,
}

impl GyroBiasRegisters {
    /// Creates an initial uncalibrated bias register block.
    pub const fn new() -> Self {
        Self {
            bias_roll_dps: 0.0,
            bias_pitch_dps: 0.0,
            bias_yaw_dps: 0.0,
            sample_count: 0,
            is_calibrated: false,
        }
    }

    /// Resets accumulated calibration registers.
    pub fn reset(&mut self) {
        self.bias_roll_dps = 0.0;
        self.bias_pitch_dps = 0.0;
        self.bias_yaw_dps = 0.0;
        self.sample_count = 0;
        self.is_calibrated = false;
    }
}

/// Motion aiming sensitivity transfer curve profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AimingCurveType {
    /// Linear 1:1 proportional response curve.
    Linear,
    /// Power curve (y = x^gamma) providing fine control in the center and quick turns on edges.
    Exponential,
    /// Dual-zone response: reduced sensitivity in micro-aim core, linear transition outside.
    DualZone,
}

/// Configuration profile governing motion transfer, deadzones, and acceleration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GyroAimProfile {
    /// Curve profile type.
    pub curve_type: AimingCurveType,
    /// Base sensitivity scaling multiplier.
    pub base_sensitivity: f32,
    /// Exponential gamma exponent (typically 1.2 to 1.8).
    pub exponent: f32,
    /// Tremor deadzone floor in °/s (micro-movements below this are suppressed).
    pub tremor_deadzone_dps: f32,
    /// Speed threshold in °/s above which high-speed angular acceleration engages.
    pub acceleration_threshold_dps: f32,
    /// Maximum acceleration multiplier for quick flick turns.
    pub max_acceleration_multiplier: f32,
}

impl GyroAimProfile {
    /// Default handheld aiming configuration tuned for competitive FPS and precision navigation.
    pub const fn default_competitive() -> Self {
        Self {
            curve_type: AimingCurveType::Exponential,
            base_sensitivity: 1.0,
            exponent: 1.4,
            tremor_deadzone_dps: 0.25, // 0.25 °/s tremor suppression
            acceleration_threshold_dps: 30.0, // 30 °/s flick threshold
            max_acceleration_multiplier: 2.5,
        }
    }
}

/// Verification helper for Commit 1 angular velocity structures and aiming curves.
pub fn verify_gyro_angular_structures_and_curves() -> bool {
    // 1. Verify GyroAngularVelocity Initialization & Unit Conversion
    let gyro = GyroAngularVelocity::new(0.0, 90.0, 0.0, 10_000);
    if gyro.pitch_dps != 90.0 || gyro.timestamp_us != 10_000 {
        return false;
    }

    let (pitch_rad, yaw_rad, roll_rad) = gyro.to_rad_per_sec();
    // 90° = PI / 2 radians (~1.57079 rad)
    if (pitch_rad - (PI / 2.0)).abs() > 0.001 || yaw_rad != 0.0 || roll_rad != 0.0 {
        return false;
    }
    if (gyro.magnitude_dps() - 90.0).abs() > 0.001 {
        return false;
    }

    // 2. Verify GyroBiasRegisters
    let mut bias_regs = GyroBiasRegisters::new();
    if bias_regs.is_calibrated || bias_regs.sample_count != 0 {
        return false;
    }
    bias_regs.bias_yaw_dps = 0.5;
    bias_regs.is_calibrated = true;
    bias_regs.reset();
    if bias_regs.bias_yaw_dps != 0.0 || bias_regs.is_calibrated {
        return false;
    }

    // 3. Verify Aiming Profile Defaults
    let profile = GyroAimProfile::default_competitive();
    if profile.curve_type != AimingCurveType::Exponential || profile.tremor_deadzone_dps != 0.25 {
        return false;
    }

    true
}
/// Stationary threshold in °/s below which the handheld device is considered at rest.
pub const STATIONARY_THRESHOLD_DPS: f32 = 0.85;
/// Minimum stationary resting duration in seconds before background bias adaptation begins.
pub const STATIONARY_MIN_REST_SEC: f32 = 0.30;

/// Background stationary zero-drift auto-calibrator and dynamic bias compensator.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StationaryDriftCompensator {
    /// Internal hardware bias registers.
    pub registers: GyroBiasRegisters,
    /// Velocity magnitude threshold in °/s to qualify as stationary rest.
    pub threshold_dps: f32,
    /// Accumulated duration of continuous stationary rest in seconds.
    pub rest_duration_sec: f32,
    /// Total consecutive stationary samples processed.
    pub stationary_samples: u32,
}

impl StationaryDriftCompensator {
    /// Creates a new auto-calibrator with default resting thresholds.
    pub const fn new() -> Self {
        Self {
            registers: GyroBiasRegisters::new(),
            threshold_dps: STATIONARY_THRESHOLD_DPS,
            rest_duration_sec: 0.0,
            stationary_samples: 0,
        }
    }

    /// Evaluates incoming gyro sample for resting condition and updates background bias estimates.
    pub fn update(&mut self, sample: &GyroAngularVelocity, dt_sec: f32) {
        let mag = sample.magnitude_dps();

        if mag <= self.threshold_dps {
            self.rest_duration_sec += dt_sec;
            self.stationary_samples = self.stationary_samples.saturating_add(1);

            // Once the device has been stably resting for at least 300ms, adapt bias
            if self.rest_duration_sec >= STATIONARY_MIN_REST_SEC {
                let alpha = (1.8 * dt_sec).min(0.08); // Exponential smoothing rate

                self.registers.bias_roll_dps =
                    self.registers.bias_roll_dps * (1.0 - alpha) + sample.roll_dps * alpha;
                self.registers.bias_pitch_dps =
                    self.registers.bias_pitch_dps * (1.0 - alpha) + sample.pitch_dps * alpha;
                self.registers.bias_yaw_dps =
                    self.registers.bias_yaw_dps * (1.0 - alpha) + sample.yaw_dps * alpha;

                self.registers.sample_count = self.registers.sample_count.saturating_add(1);
                if self.registers.sample_count >= 15 {
                    self.registers.is_calibrated = true;
                }
            }
        } else {
            // Active motion detected: freeze bias learning and reset rest timer
            self.rest_duration_sec = 0.0;
            self.stationary_samples = 0;
        }
    }

    /// Subtracts the learned zero-drift bias from raw angular velocity.
    pub fn compensate(&self, raw: &GyroAngularVelocity) -> GyroAngularVelocity {
        GyroAngularVelocity {
            roll_dps: raw.roll_dps - self.registers.bias_roll_dps,
            pitch_dps: raw.pitch_dps - self.registers.bias_pitch_dps,
            yaw_dps: raw.yaw_dps - self.registers.bias_yaw_dps,
            timestamp_us: raw.timestamp_us,
        }
    }
}

/// Verification helper for Commit 2 stationary zero-drift calibration.
pub fn verify_stationary_drift_calibration() -> bool {
    let mut compensator = StationaryDriftCompensator::new();

    // 1. Initial State: Uncalibrated
    if compensator.registers.is_calibrated || compensator.rest_duration_sec != 0.0 {
        return false;
    }

    // 2. Feed 100 resting samples with intentional slow temperature drift (+0.40 °/s yaw, -0.30 °/s pitch)
    // 100 steps * 10ms (0.01s) = 1.0 second total rest
    for i in 0..100 {
        let sample = GyroAngularVelocity::new(0.0, -0.30, 0.40, (i as u64) * 10_000);
        compensator.update(&sample, 0.01);
    }

    // Bias should have converged close to the intentional drift offsets
    if !compensator.registers.is_calibrated {
        return false;
    }
    if (compensator.registers.bias_yaw_dps - 0.40).abs() > 0.06 {
        return false;
    }
    if (compensator.registers.bias_pitch_dps - (-0.30)).abs() > 0.06 {
        return false;
    }

    // 3. Compensate drifting raw reading: should output near-zero
    let drifting_sample = GyroAngularVelocity::new(0.0, -0.30, 0.40, 1_000_000);
    let corrected = compensator.compensate(&drifting_sample);
    if corrected.yaw_dps.abs() > 0.06 || corrected.pitch_dps.abs() > 0.06 {
        return false;
    }

    // 4. Feed active high-velocity motion (e.g. 45 °/s flick turn)
    let flick_sample = GyroAngularVelocity::new(0.0, 10.0, 45.0, 1_010_000);
    compensator.update(&flick_sample, 0.01);
    // Resting timer must have reset to 0, while learned bias remains frozen
    if compensator.rest_duration_sec != 0.0 {
        return false;
    }
    if (compensator.registers.bias_yaw_dps - 0.40).abs() > 0.06 {
        return false;
    }

    true
}

// -----------------------------------------------------------------------------
// Pure no_std Fast Math Approximations for Curve Calculation
// -----------------------------------------------------------------------------

/// Fast natural logarithm approximation for positive f32.
pub fn ln_f32(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    let bits = x.to_bits();
    let exp = ((bits >> 23) & 0xFF) as i32 - 127;
    let mant = (bits & 0x007F_FFFF) | 0x3F80_0000;
    let m = f32::from_bits(mant);
    let u = (m - 1.0) / (m + 1.0);
    let u2 = u * u;
    let res = 2.0 * u * (1.0 + u2 * ((1.0 / 3.0) + u2 * ((1.0 / 5.0) + u2 * ((1.0 / 7.0) + u2 * (1.0 / 9.0)))));
    res + (exp as f32) * 0.69314718
}

/// Fast exponential approximation e^y.
pub fn exp_f32(y: f32) -> f32 {
    const LN2: f32 = 0.69314718;
    const INV_LN2: f32 = 1.44269504;
    let k_f = y * INV_LN2;
    let k = if k_f >= 0.0 {
        (k_f + 0.5) as i32
    } else {
        (k_f - 0.5) as i32
    };
    let r = y - (k as f32) * LN2;
    let expr = 1.0 + r * (1.0 + r * (0.5 + r * ((1.0 / 6.0) + r * ((1.0 / 24.0) + r * (1.0 / 120.0)))));
    let exp_bits = ((k + 127).max(0).min(255) as u32) << 23;
    let scale = f32::from_bits(exp_bits);
    expr * scale
}

/// Fast power function base^exp for positive base.
pub fn powf_f32(base: f32, exp: f32) -> f32 {
    if base <= 0.0 {
        return 0.0;
    }
    exp_f32(exp * ln_f32(base))
}

// -----------------------------------------------------------------------------
// Motion Aiming Governor & Auto-Centering Ratchet
// -----------------------------------------------------------------------------

/// Motion aiming governor handling sensitivity curve mapping, tremor deadzone suppression,
/// flick angular acceleration, and yaw auto-centering ratcheting during thumbstick panning.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionAimingGovernor {
    /// Active aiming profile configuration.
    pub profile: GyroAimProfile,
    /// Stick deflection threshold [0.0..1.0] above which yaw auto-centering activates.
    pub ratchet_threshold: f32,
    /// Rate at which accumulated yaw is recentered during ratcheting (degrees/second).
    pub ratchet_speed_dps: f32,
    /// Indicates whether auto-centering ratchet is actively engaged.
    pub is_ratcheting: bool,
    /// Current accumulated integrated yaw angle in degrees.
    pub integrated_yaw_deg: f32,
}

impl MotionAimingGovernor {
    /// Creates a motion aiming governor with default competitive profile and ratchet parameters.
    pub const fn new(profile: GyroAimProfile) -> Self {
        Self {
            profile,
            ratchet_threshold: 0.70,
            ratchet_speed_dps: 30.0,
            is_ratcheting: false,
            integrated_yaw_deg: 0.0,
        }
    }

    /// Evaluates transfer function for a single angular velocity axis in °/s.
    ///
    /// Applies tremor deadzone clamping, curve translation (Linear, Exponential, DualZone),
    /// and speed-dependent angular flick acceleration.
    pub fn apply_transfer_function(&self, speed_dps: f32) -> f32 {
        let abs_speed = speed_dps.abs();
        if abs_speed <= self.profile.tremor_deadzone_dps {
            return 0.0;
        }

        let effective_speed = abs_speed - self.profile.tremor_deadzone_dps;

        let mut curved_speed = match self.profile.curve_type {
            AimingCurveType::Linear => effective_speed * self.profile.base_sensitivity,
            AimingCurveType::Exponential => {
                powf_f32(effective_speed, self.profile.exponent) * self.profile.base_sensitivity
            }
            AimingCurveType::DualZone => {
                if effective_speed <= 15.0 {
                    effective_speed * 0.65 * self.profile.base_sensitivity
                } else {
                    (15.0 * 0.65 + (effective_speed - 15.0) * 1.35) * self.profile.base_sensitivity
                }
            }
        };

        // Speed-dependent angular acceleration (flick turns)
        if abs_speed >= self.profile.acceleration_threshold_dps {
            let accel_ratio =
                (abs_speed - self.profile.acceleration_threshold_dps) / self.profile.acceleration_threshold_dps;
            let accel_mult = (1.0 + accel_ratio).min(self.profile.max_acceleration_multiplier);
            curved_speed *= accel_mult;
        }

        if speed_dps < 0.0 {
            -curved_speed
        } else {
            curved_speed
        }
    }

    /// Updates the auto-centering ratchet governor based on right analog stick deflection.
    ///
    /// When the player holds a large stick turn (|right_stick_x| >= ratchet_threshold),
    /// the gyro yaw angle is smoothly ratcheted back toward zero to relieve wrist strain.
    pub fn update_ratchet(&mut self, right_stick_x: f32, dt_sec: f32) -> bool {
        if right_stick_x.abs() >= self.ratchet_threshold {
            self.is_ratcheting = true;
            let decay = (self.ratchet_speed_dps * dt_sec).min(self.integrated_yaw_deg.abs());
            if self.integrated_yaw_deg > 0.0 {
                self.integrated_yaw_deg -= decay;
            } else if self.integrated_yaw_deg < 0.0 {
                self.integrated_yaw_deg += decay;
            }
            true
        } else {
            self.is_ratcheting = false;
            false
        }
    }

    /// Processes an input angular velocity frame, updating yaw integration and auto-centering.
    ///
    /// Returns compensated and curved angular velocity: `(pitch_dps, yaw_dps)`.
    pub fn process_motion(
        &mut self,
        sample: &GyroAngularVelocity,
        right_stick_x: f32,
        dt_sec: f32,
    ) -> (f32, f32) {
        let curved_pitch = self.apply_transfer_function(sample.pitch_dps);
        let curved_yaw = self.apply_transfer_function(sample.yaw_dps);

        // Accumulate integrated yaw
        self.integrated_yaw_deg += curved_yaw * dt_sec;

        // Apply auto-centering ratchet
        self.update_ratchet(right_stick_x, dt_sec);

        (curved_pitch, curved_yaw)
    }
}

/// Verification helper for Commit 3 motion transfer, deadzone filter, and yaw auto-centering ratchet.
pub fn verify_motion_transfer_and_yaw_autocentering() -> bool {
    let profile = GyroAimProfile::default_competitive();
    let mut governor = MotionAimingGovernor::new(profile);

    // 1. Verify Tremor Deadzone Suppression (deadzone = 0.25 °/s)
    let micro_tremor_pos = governor.apply_transfer_function(0.18);
    let micro_tremor_neg = governor.apply_transfer_function(-0.24);
    if micro_tremor_pos != 0.0 || micro_tremor_neg != 0.0 {
        return false;
    }

    // 2. Verify Linear vs Exponential Curve Dynamics
    let linear_profile = GyroAimProfile {
        curve_type: AimingCurveType::Linear,
        base_sensitivity: 1.0,
        exponent: 1.0,
        tremor_deadzone_dps: 0.25,
        acceleration_threshold_dps: 50.0,
        max_acceleration_multiplier: 2.0,
    };
    let linear_gov = MotionAimingGovernor::new(linear_profile);
    let lin_out = linear_gov.apply_transfer_function(5.25);
    // (5.25 - 0.25) * 1.0 = 5.0
    if (lin_out - 5.0).abs() > 0.01 {
        return false;
    }

    // Exponential curve with gamma 1.4: (5.25 - 0.25)^1.4 = 5.0^1.4 ≈ 9.52
    let exp_out = governor.apply_transfer_function(5.25);
    if (exp_out - 9.52).abs() > 0.5 {
        return false;
    }

    // Negative velocity must maintain sign
    let neg_out = governor.apply_transfer_function(-5.25);
    if (neg_out - (-exp_out)).abs() > 0.001 {
        return false;
    }

    // 3. Verify Flick Angular Acceleration (threshold = 30.0 °/s, max_mult = 2.5)
    // Speed = 60.0 °/s -> effective = 59.75. abs_speed >= 30.0 -> accel_ratio = (60 - 30) / 30 = 1.0 -> mult = 2.0
    let flick_out = governor.apply_transfer_function(60.0);
    let unaccelerated = powf_f32(60.0 - 0.25, 1.4);
    if flick_out <= unaccelerated * 1.5 {
        return false;
    }

    // 4. Verify Yaw Auto-Centering Ratchet Governor
    governor.integrated_yaw_deg = 20.0;
    // With stick neutral (0.0), ratchet should not engage and yaw should not decay
    let ratcheting = governor.update_ratchet(0.0, 0.05);
    if ratcheting || governor.is_ratcheting || governor.integrated_yaw_deg != 20.0 {
        return false;
    }

    // Deflect right stick past threshold (0.85 >= 0.70)
    let ratcheting_active = governor.update_ratchet(0.85, 0.1);
    if !ratcheting_active || !governor.is_ratcheting {
        return false;
    }
    // With ratchet_speed_dps = 30.0 and dt = 0.1, decay = 3.0 degrees
    if (governor.integrated_yaw_deg - 17.0).abs() > 0.01 {
        return false;
    }

    // Run additional steps to recenter past 0: decay clamps smoothly at 0 without overshoot
    for _ in 0..10 {
        governor.update_ratchet(0.85, 0.1);
    }
    if governor.integrated_yaw_deg != 0.0 {
        return false;
    }

    true
}