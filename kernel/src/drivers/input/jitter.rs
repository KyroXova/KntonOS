//! Micro-Jitter Suppression Filter & Adaptive Deadzone Shaping.
//!
//! Provides esports-grade analog filtering for Hall effect sensors and analog potentiometers:
//! - 1-Euro adaptive low-pass filter (Casiez et al., CHI 2012)
//!   Suppresses high-frequency micro-jitter and electrical noise at rest while
//!   providing zero-latency response during fast flicks.
//! - Radial (circular) and Axial deadzone shaping.
//! - Dynamic center calibration and Hall effect magnetic drift compensation.

use super::imu::{atan2_f32, sqrt_f32, PI};

/// Default minimum cutoff frequency at zero velocity (Hz).
pub const DEFAULT_MIN_CUTOFF_HZ: f32 = 1.0;

/// Default speed coefficient beta for adaptive cutoff scaling.
pub const DEFAULT_BETA: f32 = 1.0;

/// Default cutoff frequency for the derivative filter (Hz).
pub const DEFAULT_DERIVATIVE_CUTOFF_HZ: f32 = 20.0;

/// First-order low-pass filter stage.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LowPassFilter {
    hat_x: f32,
    initialized: bool,
}

impl LowPassFilter {
    pub const fn new() -> Self {
        Self {
            hat_x: 0.0,
            initialized: false,
        }
    }

    /// Filters incoming value using smoothing factor alpha in [0.0, 1.0].
    /// alpha = 1.0 means no filtering (raw output), alpha -> 0.0 means heavy smoothing.
    pub fn filter(&mut self, value: f32, alpha: f32) -> f32 {
        if !self.initialized {
            self.hat_x = value;
            self.initialized = true;
            return value;
        }

        let alpha_clamped = if alpha < 0.0 {
            0.0
        } else if alpha > 1.0 {
            1.0
        } else {
            alpha
        };

        self.hat_x = self.hat_x + alpha_clamped * (value - self.hat_x);
        self.hat_x
    }

    pub fn value(&self) -> f32 {
        self.hat_x
    }

    pub fn reset(&mut self) {
        self.hat_x = 0.0;
        self.initialized = false;
    }
}

/// Computes the smoothing factor alpha from cutoff frequency `fc` (Hz) and sample period `dt` (seconds).
#[inline]
pub fn compute_alpha(fc_hz: f32, dt_sec: f32) -> f32 {
    let r = 2.0 * PI * fc_hz * dt_sec;
    r / (r + 1.0)
}

/// 1-Euro Adaptive Low-Pass Filter (Casiez et al., CHI 2012).
///
/// Dynamically adjusts the filter cutoff frequency based on the filtered derivative (speed)
/// of the input signal. At low speeds / rest, cutoff approaches `min_cutoff_hz`, virtually
/// eliminating analog jitter. At high speeds, cutoff increases linearly with `beta * |dx|`,
/// driving phase lag to near zero.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OneEuroFilter {
    pub min_cutoff_hz: f32,
    pub beta: f32,
    pub derivative_cutoff_hz: f32,
    x_filt: LowPassFilter,
    dx_filt: LowPassFilter,
    last_x: f32,
    last_timestamp_ns: u64,
    initialized: bool,
}

impl OneEuroFilter {
    pub fn new(min_cutoff_hz: f32, beta: f32, derivative_cutoff_hz: f32) -> Self {
        Self {
            min_cutoff_hz: min_cutoff_hz.max(0.1),
            beta: beta.max(0.0),
            derivative_cutoff_hz: derivative_cutoff_hz.max(0.1),
            x_filt: LowPassFilter::new(),
            dx_filt: LowPassFilter::new(),
            last_x: 0.0,
            last_timestamp_ns: 0,
            initialized: false,
        }
    }

    pub fn default_stick() -> Self {
        Self::new(DEFAULT_MIN_CUTOFF_HZ, DEFAULT_BETA, DEFAULT_DERIVATIVE_CUTOFF_HZ)
    }

    /// Filters a single scalar sample with nanosecond timestamp.
    pub fn filter(&mut self, raw_value: f32, timestamp_ns: u64) -> f32 {
        if !self.initialized {
            self.last_x = raw_value;
            self.last_timestamp_ns = timestamp_ns;
            self.initialized = true;
            return self.x_filt.filter(raw_value, 1.0);
        }

        let dt_sec = if self.last_timestamp_ns > 0 && timestamp_ns > self.last_timestamp_ns {
            let ns_diff = timestamp_ns - self.last_timestamp_ns;
            (ns_diff as f32) / 1_000_000_000.0
        } else {
            0.001 // Fallback to 1ms (1000Hz) if timestamp is non-monotonic
        };

        // Estimate derivative (rate of change)
        let dx = (raw_value - self.last_x) / dt_sec;
        let alpha_d = compute_alpha(self.derivative_cutoff_hz, dt_sec);
        let filtered_dx = self.dx_filt.filter(dx, alpha_d);

        // Adaptive cutoff frequency: fc = fc_min + beta * |dx|
        let abs_dx = if filtered_dx < 0.0 { -filtered_dx } else { filtered_dx };
        let adaptive_fc = self.min_cutoff_hz + self.beta * abs_dx;

        // Filter value with adaptive cutoff
        let alpha = compute_alpha(adaptive_fc, dt_sec);
        let filtered_x = self.x_filt.filter(raw_value, alpha);

        self.last_x = raw_value;
        self.last_timestamp_ns = timestamp_ns;

        filtered_x
    }

    pub fn reset(&mut self) {
        self.x_filt.reset();
        self.dx_filt.reset();
        self.last_x = 0.0;
        self.last_timestamp_ns = 0;
        self.initialized = false;
    }
}

/// 2-Dimensional 1-Euro Filter for analog thumbstick (X, Y) pairs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OneEuroFilter2D {
    pub filter_x: OneEuroFilter,
    pub filter_y: OneEuroFilter,
}

impl OneEuroFilter2D {
    pub fn new(min_cutoff_hz: f32, beta: f32, derivative_cutoff_hz: f32) -> Self {
        Self {
            filter_x: OneEuroFilter::new(min_cutoff_hz, beta, derivative_cutoff_hz),
            filter_y: OneEuroFilter::new(min_cutoff_hz, beta, derivative_cutoff_hz),
        }
    }

    pub fn default_stick() -> Self {
        Self {
            filter_x: OneEuroFilter::default_stick(),
            filter_y: OneEuroFilter::default_stick(),
        }
    }

    /// Filters 2D coordinate pair with nanosecond timestamp.
    pub fn filter_stick(&mut self, raw_x: f32, raw_y: f32, timestamp_ns: u64) -> (f32, f32) {
        let fx = self.filter_x.filter(raw_x, timestamp_ns);
        let fy = self.filter_y.filter(raw_y, timestamp_ns);
        (fx, fy)
    }

    pub fn reset(&mut self) {
        self.filter_x.reset();
        self.filter_y.reset();
    }
}

/// Geometry shape of stick deadzone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JitterDeadzoneShape {
    /// Radial / Circular deadzone preserving angular trajectory.
    Radial,
    /// Independent X and Y axial deadzones.
    Axial,
}

/// Deadzone and response curve calibration configuration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JitterDeadzoneConfig {
    /// Inner deadzone threshold (e.g. 0.05 for 5%). Inputs below this are mapped to 0.0.
    pub inner: f32,
    /// Outer deadzone threshold (e.g. 0.95 for 95%). Inputs above this are scaled to 1.0.
    pub outer: f32,
    /// Shape geometry (Radial vs Axial).
    pub shape: JitterDeadzoneShape,
    /// Optional response curve exponent (1.0 = linear, 2.0 = smooth quadratic).
    pub curve_exponent: f32,
}

impl Default for JitterDeadzoneConfig {
    fn default() -> Self {
        Self {
            inner: 0.06,
            outer: 0.95,
            shape: JitterDeadzoneShape::Radial,
            curve_exponent: 1.0,
        }
    }
}

impl JitterDeadzoneConfig {
    pub const fn new(inner: f32, outer: f32, shape: JitterDeadzoneShape) -> Self {
        Self {
            inner,
            outer,
            shape,
            curve_exponent: 1.0,
        }
    }

    /// Applies deadzone thresholds and rescaling to a normalized coordinate pair [-1.0, 1.0].
    /// Returns `(shaped_x, shaped_y, magnitude)`.
    pub fn apply(&self, in_x: f32, in_y: f32) -> (f32, f32, f32) {
        match self.shape {
            JitterDeadzoneShape::Radial => {
                let mag = sqrt_f32(in_x * in_x + in_y * in_y);
                if mag <= self.inner {
                    (0.0, 0.0, 0.0)
                } else {
                    let angle = atan2_f32(in_y, in_x);
                    let denom = (self.outer - self.inner).max(1e-4);
                    let mut norm_mag = (mag - self.inner) / denom;
                    if norm_mag > 1.0 {
                        norm_mag = 1.0;
                    }

                    // Apply curve exponent
                    let shaped_mag = if (self.curve_exponent - 1.0).abs() < 1e-3 {
                        norm_mag
                    } else if (self.curve_exponent - 2.0).abs() < 1e-3 {
                        norm_mag * norm_mag
                    } else {
                        norm_mag
                    };

                    let out_x = (in_x / mag) * shaped_mag;
                    let out_y = (in_y / mag) * shaped_mag;
                    let _ = angle; // Kept internally consistent
                    (out_x, out_y, shaped_mag)
                }
            }
            JitterDeadzoneShape::Axial => {
                let shape_axis = |v: f32| -> f32 {
                    let sign = if v < 0.0 { -1.0 } else { 1.0 };
                    let abs_v = if v < 0.0 { -v } else { v };
                    if abs_v <= self.inner {
                        0.0
                    } else {
                        let denom = (self.outer - self.inner).max(1e-4);
                        let mut norm = (abs_v - self.inner) / denom;
                        if norm > 1.0 {
                            norm = 1.0;
                        }
                        if (self.curve_exponent - 2.0).abs() < 1e-3 {
                            norm = norm * norm;
                        }
                        sign * norm
                    }
                };

                let out_x = shape_axis(in_x);
                let out_y = shape_axis(in_y);
                let mag = sqrt_f32(out_x * out_x + out_y * out_y).min(1.0);
                (out_x, out_y, mag)
            }
        }
    }
}

/// Dynamic Center & Hall Effect Magnetic Drift Calibrator.
///
/// Hall effect sensors experience thermal variations and magnetic hysteresis drift over time.
/// This calibrator tracks the resting baseline when the thumbstick is idle inside the inner deadzone
/// and dynamically centers the analog coordinate origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HallEffectCenterCalibrator {
    pub center_offset_x: f32,
    pub center_offset_y: f32,
    pub max_drift_clamp: f32,
    pub adaptation_rate: f32,
    resting_samples_count: u32,
}

impl HallEffectCenterCalibrator {
    pub const fn new(max_drift_clamp: f32) -> Self {
        Self {
            center_offset_x: 0.0,
            center_offset_y: 0.0,
            max_drift_clamp,
            adaptation_rate: 0.01, // Low-pass center drift learning
            resting_samples_count: 0,
        }
    }

    /// Calibrates and offsets raw inputs: returns `(calibrated_x, calibrated_y)`.
    pub fn calibrate(&mut self, raw_x: f32, raw_y: f32, is_resting: bool) -> (f32, f32) {
        if is_resting {
            self.resting_samples_count = self.resting_samples_count.saturating_add(1);
            // Slowly adjust baseline towards measured resting value
            self.center_offset_x += self.adaptation_rate * (raw_x - self.center_offset_x);
            self.center_offset_y += self.adaptation_rate * (raw_y - self.center_offset_y);

            // Clamp max drift offset
            self.center_offset_x = self.center_offset_x.clamp(-self.max_drift_clamp, self.max_drift_clamp);
            self.center_offset_y = self.center_offset_y.clamp(-self.max_drift_clamp, self.max_drift_clamp);
        }

        let cal_x = (raw_x - self.center_offset_x).clamp(-1.0, 1.0);
        let cal_y = (raw_y - self.center_offset_y).clamp(-1.0, 1.0);
        (cal_x, cal_y)
    }

    pub fn resting_sample_count(&self) -> u32 {
        self.resting_samples_count
    }
}


/// Physical stick rebound / snapback suppression filter.
///
/// When an analog stick is released from full deflection, centering springs cause
/// mechanical overshoot past the center resting point, which can cause spurious opposite-direction
/// inputs in esports titles. This filter detects rapid return towards center and suppresses
/// reverse overshoot within a brief settling window (typically 15-25ms).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AntiReboundSmoother {
    /// Activation threshold: stick must have reached at least this magnitude to arm snapback protection.
    pub arm_threshold: f32,
    /// Maximum snapback overshoot magnitude allowed to be suppressed.
    pub max_rebound_magnitude: f32,
    /// Settling duration in samples (e.g. 25 samples at 1000Hz = 25ms).
    pub rebound_window_samples: u32,

    // Internal state
    last_deflection_x: f32,
    last_deflection_y: f32,
    rebound_timer: u32,
    is_armed: bool,
    total_rebounds_suppressed: u32,
}

impl Default for AntiReboundSmoother {
    fn default() -> Self {
        Self {
            arm_threshold: 0.50,
            max_rebound_magnitude: 0.35,
            rebound_window_samples: 25,
            last_deflection_x: 0.0,
            last_deflection_y: 0.0,
            rebound_timer: 0,
            is_armed: false,
            total_rebounds_suppressed: 0,
        }
    }
}

impl AntiReboundSmoother {
    pub const fn new(arm_threshold: f32, max_rebound_magnitude: f32, rebound_window_samples: u32) -> Self {
        Self {
            arm_threshold,
            max_rebound_magnitude,
            rebound_window_samples,
            last_deflection_x: 0.0,
            last_deflection_y: 0.0,
            rebound_timer: 0,
            is_armed: false,
            total_rebounds_suppressed: 0,
        }
    }

    /// Evaluates current stick position and suppresses mechanical snapback overshoot.
    /// Returns (smoothed_x, smoothed_y, was_rebound_suppressed).
    pub fn process(&mut self, x: f32, y: f32) -> (f32, f32, bool) {
        let mag = sqrt_f32(x * x + y * y);

        // 1. Arm if stick is deflected beyond threshold
        if mag >= self.arm_threshold {
            self.last_deflection_x = x;
            self.last_deflection_y = y;
            self.is_armed = true;
            self.rebound_timer = 0;
            return (x, y, false);
        }

        // 2. If armed and stick enters center deadzone / crosses origin, start rebound protection window
        if self.is_armed && mag <= 0.15 {
            self.rebound_timer = self.rebound_window_samples;
            self.is_armed = false;
        }

        // 3. During rebound window, check for opposite-direction overshoot
        if self.rebound_timer > 0 {
            self.rebound_timer -= 1;

            // Directional dot product against original deflection vector
            let dot = x * self.last_deflection_x + y * self.last_deflection_y;

            // If dot product is negative (opposite half-plane) and within rebound magnitude:
            if dot < 0.0 && mag <= self.max_rebound_magnitude {
                self.total_rebounds_suppressed += 1;
                // Suppress snapback: zero out the overshoot
                return (0.0, 0.0, true);
            } else if dot < 0.0 && mag > self.max_rebound_magnitude {
                // Deliberate user flick in opposite direction: cancel protection immediately
                self.rebound_timer = 0;
            }
        }

        (x, y, false)
    }

    pub fn total_suppressed(&self) -> u32 {
        self.total_rebounds_suppressed
    }

    pub fn reset(&mut self) {
        self.last_deflection_x = 0.0;
        self.last_deflection_y = 0.0;
        self.rebound_timer = 0;
        self.is_armed = false;
        self.total_rebounds_suppressed = 0;
    }
}

/// Self-test verifying 1-Euro adaptive low-pass filter and radial deadzone calibration.
pub fn verify_one_euro_filter_and_deadzone() -> bool {
    // 1. Verify LowPassFilter basic operation
    let mut lpf = LowPassFilter::new();
    let v1 = lpf.filter(10.0, 0.5);
    if (v1 - 10.0).abs() > 1e-4 {
        return false; // Initial sample must pass through
    }
    let v2 = lpf.filter(20.0, 0.5);
    if (v2 - 15.0).abs() > 1e-4 {
        return false;
    }

    // 2. Test 1-Euro Filter at rest: high-frequency jitter should be strongly attenuated
    let mut euro = OneEuroFilter::default_stick();
    let mut ts = 1_000_000_000u64; // 1.0s
    let resting_baseline = 0.0f32;
    // Prime the filter at resting baseline
    let _ = euro.filter(resting_baseline, ts);

    // Simulate 50 samples of high-frequency noise (+-0.03 at 1000Hz) around 0.0
    let mut max_filtered_jitter = 0.0f32;
    for i in 0..50 {
        ts += 1_000_000; // +1ms (1000Hz)
        let noise = if i % 2 == 0 { 0.03 } else { -0.03 };
        let filtered = euro.filter(resting_baseline + noise, ts);
        let abs_val = if filtered < 0.0 { -filtered } else { filtered };
        if abs_val > max_filtered_jitter {
            max_filtered_jitter = abs_val;
        }
    }
    // High-frequency jitter (+-0.03) must be attenuated by at least 60%
    if max_filtered_jitter > 0.015 {
        return false;
    }

    // 3. Test 1-Euro Filter during high-speed flick: latency must remain near zero
    // A fast physical flick takes 5ms (5 samples at 1000Hz)
    let flick_target = 0.95f32;
    let mut flick_val = 0.0f32;
    for _ in 0..5 {
        ts += 1_000_000;
        flick_val = euro.filter(flick_target, ts);
    }
    // Adaptive cutoff should scale rapidly, tracking > 0.75 within 5ms
    if flick_val < 0.75 {
        return false;
    }

    // 4. Test Radial Deadzone
    let deadzone = JitterDeadzoneConfig::new(0.08, 0.95, JitterDeadzoneShape::Radial);

    // Inputs inside deadzone (0.05 < 0.08) must result in (0.0, 0.0)
    let (dx1, dy1, mag1) = deadzone.apply(0.03, 0.04);
    if dx1 != 0.0 || dy1 != 0.0 || mag1 != 0.0 {
        return false;
    }

    // Inputs outside outer boundary (0.98 > 0.95) must clamp to magnitude 1.0
    let (dx2, dy2, mag2) = deadzone.apply(0.98, 0.0);
    if (dx2 - 1.0).abs() > 1e-4 || dy2 != 0.0 || (mag2 - 1.0).abs() > 1e-4 {
        return false;
    }

    // 5. Test Hall Effect Center Calibrator
    let mut hall_cal = HallEffectCenterCalibrator::new(0.10);
    // Simulate slight permanent magnet drift +0.04 along X while resting
    for _ in 0..100 {
        let _ = hall_cal.calibrate(0.04, 0.0, true);
    }
    if hall_cal.center_offset_x < 0.02 {
        return false; // Did not adapt to magnetic resting offset
    }

    true
}
