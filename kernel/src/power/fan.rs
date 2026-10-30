//! Handheld PC Multi-Point Fan Speed Interpolation Curves & PWM Duty Table.
//! Provides multi-point linear and cubic interpolation mapping temperatures (°C)

use alloc::vec::Vec;

// Constants and Operational Limits

/// Minimum audible fan floor percentage (e.g. 15% to guarantee fan motor stall prevention).
pub const DEFAULT_FAN_MIN_AUDIBLE_FLOOR_PERCENT: u8 = 15;

/// Maximum emergency trip temperature in °C (forces 100% fan speed blast).
pub const EMERGENCY_FAN_TRIP_TEMP_CELSIUS: u8 = 95;

/// Maximum number of control points supported per fan curve.
pub const MAX_FAN_CURVE_POINTS: usize = 8;

// Multi-Point Fan Curve Definitions

/// Single temperature-to-PWM control point on a fan curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FanCurvePoint {
    /// Temperature threshold in degrees Celsius.
    pub temp_celsius: u8,
    /// Target PWM duty cycle percentage (0 to 100).
    pub pwm_percent: u8,
}

impl FanCurvePoint {
    pub const fn new(temp_celsius: u8, pwm_percent: u8) -> Self {
        Self {
            temp_celsius,
            pwm_percent: if pwm_percent > 100 { 100 } else { pwm_percent },
        }
    }
}

/// Curve interpolation mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurveInterpolationMode {
    /// Piecewise linear interpolation between adjacent points.
    Linear,
    /// Smooth cubic Hermite interpolation avoiding sharp acoustic corners.
    CubicSmooth,
}

/// Multi-point fan curve configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiPointFanCurve {
    pub points: Vec<FanCurvePoint>,
    pub min_floor_percent: u8,
    pub emergency_temp_celsius: u8,
    pub interpolation_mode: CurveInterpolationMode,
}

impl MultiPointFanCurve {
    pub fn new(interpolation_mode: CurveInterpolationMode) -> Self {
        Self {
            points: Vec::with_capacity(MAX_FAN_CURVE_POINTS),
            min_floor_percent: DEFAULT_FAN_MIN_AUDIBLE_FLOOR_PERCENT,
            emergency_temp_celsius: EMERGENCY_FAN_TRIP_TEMP_CELSIUS,
            interpolation_mode,
        }
    }

    /// Default handheld balanced fan curve preset.
    /// Points: (40°C -> 0%), (50°C -> 25%), (65°C -> 45%), (75°C -> 70%), (85°C -> 90%), (95°C -> 100%)
    pub fn balanced_handheld_preset() -> Self {
        let mut curve = Self::new(CurveInterpolationMode::Linear);
        curve.add_point(FanCurvePoint::new(40, 0));
        curve.add_point(FanCurvePoint::new(50, 25));
        curve.add_point(FanCurvePoint::new(65, 45));
        curve.add_point(FanCurvePoint::new(75, 70));
        curve.add_point(FanCurvePoint::new(85, 90));
        curve.add_point(FanCurvePoint::new(95, 100));
        curve
    }

    /// Quiet handheld fan curve preset (maintains quiet acoustics below 65°C).
    pub fn quiet_handheld_preset() -> Self {
        let mut curve = Self::new(CurveInterpolationMode::CubicSmooth);
        curve.add_point(FanCurvePoint::new(45, 0));
        curve.add_point(FanCurvePoint::new(55, 18));
        curve.add_point(FanCurvePoint::new(68, 35));
        curve.add_point(FanCurvePoint::new(78, 60));
        curve.add_point(FanCurvePoint::new(88, 85));
        curve.add_point(FanCurvePoint::new(95, 100));
        curve
    }

    /// Adds a control point in ascending temperature order.
    pub fn add_point(&mut self, point: FanCurvePoint) {
        if self.points.len() >= MAX_FAN_CURVE_POINTS {
            return;
        }

        // Insert maintaining sorted temperature order
        let mut insert_idx = self.points.len();
        for (i, p) in self.points.iter().enumerate() {
            if point.temp_celsius < p.temp_celsius {
                insert_idx = i;
                break;
            }
        }
        self.points.insert(insert_idx, point);
    }

    /// Evaluates target raw PWM percentage for a given temperature (°C).
    pub fn evaluate_pwm(&self, temp_celsius: u8) -> u8 {
        if self.points.is_empty() {
            return self.min_floor_percent;
        }

        // 1. Critical thermal trip: 100% blast override
        if temp_celsius >= self.emergency_temp_celsius {
            return 100;
        }

        // 2. Below minimum curve point
        if temp_celsius <= self.points[0].temp_celsius {
            let p0 = self.points[0].pwm_percent;
            return if p0 == 0 { 0 } else { p0.max(self.min_floor_percent) };
        }

        // 3. Above maximum defined curve point
        let last = self.points.last().unwrap();
        if temp_celsius >= last.temp_celsius {
            return last.pwm_percent.max(self.min_floor_percent);
        }

        // 4. Find bounding interval [p_prev, p_next]
        for i in 0..(self.points.len() - 1) {
            let p0 = self.points[i];
            let p1 = self.points[i + 1];

            if temp_celsius >= p0.temp_celsius && temp_celsius <= p1.temp_celsius {
                let temp_range = (p1.temp_celsius - p0.temp_celsius) as u32;
                if temp_range == 0 {
                    return p0.pwm_percent;
                }

                let temp_delta = (temp_celsius - p0.temp_celsius) as u32;

                let target_pwm = match self.interpolation_mode {
                    CurveInterpolationMode::Linear => {
                        let pwm_range = (p1.pwm_percent as i32) - (p0.pwm_percent as i32);
                        let interpolated = (p0.pwm_percent as i32) + (pwm_range * temp_delta as i32) / (temp_range as i32);
                        interpolated.clamp(0, 100) as u8
                    }
                    CurveInterpolationMode::CubicSmooth => {
                        // Smoothstep S-curve: t = temp_delta / temp_range, s = 3*t^2 - 2*t^3 (scaled by 1024)
                        let t_1024 = (temp_delta * 1024) / temp_range;
                        let s_1024 = (t_1024 * t_1024 * (3072 - 2 * t_1024)) / (1024 * 1024);
                        let pwm_range = (p1.pwm_percent as i32) - (p0.pwm_percent as i32);
                        let interpolated = (p0.pwm_percent as i32) + (pwm_range * s_1024 as i32) / 1024;
                        interpolated.clamp(0, 100) as u8
                    }
                };

                // Enforce minimum audible floor if non-zero
                return if target_pwm > 0 {
                    target_pwm.max(self.min_floor_percent)
                } else {
                    0
                };
            }
        }

        self.min_floor_percent
    }
}

// Unit Tests & Verification

// Thermal Hysteresis Damping & Acoustic Rate-of-Change Limiter

/// Default temperature hysteresis in °C to prevent rapid step switching.
pub const DEFAULT_FAN_HYSTERESIS_CELSIUS: u8 = 3;

/// Default maximum allowable PWM ramp-up rate per second (e.g. 20% / sec).
pub const DEFAULT_MAX_RAMP_UP_PERCENT_PER_SEC: u8 = 20;

/// Default maximum allowable PWM ramp-down rate per second (e.g. 5% / sec, gentle spin-down).
pub const DEFAULT_MAX_RAMP_DOWN_PERCENT_PER_SEC: u8 = 5;

/// Fan Controller with thermal hysteresis and acoustic rate-of-change limiter.
#[derive(Debug, Clone)]
pub struct DampedFanController {
    pub curve: MultiPointFanCurve,
    pub current_pwm: u8,
    pub last_stable_temp_celsius: u8,
    pub hysteresis_celsius: u8,
    pub max_ramp_up_per_sec: u8,
    pub max_ramp_down_per_sec: u8,
}

impl DampedFanController {
    pub fn new(curve: MultiPointFanCurve) -> Self {
        Self {
            curve,
            current_pwm: 0,
            last_stable_temp_celsius: 0,
            hysteresis_celsius: DEFAULT_FAN_HYSTERESIS_CELSIUS,
            max_ramp_up_per_sec: DEFAULT_MAX_RAMP_UP_PERCENT_PER_SEC,
            max_ramp_down_per_sec: DEFAULT_MAX_RAMP_DOWN_PERCENT_PER_SEC,
        }
    }

    /// Evaluates target PWM with thermal hysteresis damping and slewing rate limiter.
    /// # Parameters:
    pub fn update(&mut self, measured_temp: u8, elapsed_ms: u32) -> u8 {
        // Critical emergency trip overrides all rate limiters and hysteresis
        if measured_temp >= self.curve.emergency_temp_celsius {
            self.current_pwm = 100;
            self.last_stable_temp_celsius = measured_temp;
            return 100;
        }

        // Apply temperature hysteresis
        // If temperature drops by less than hysteresis_celsius, stay at previous temperature regime
        let effective_temp = if measured_temp < self.last_stable_temp_celsius {
            if self.last_stable_temp_celsius.saturating_sub(measured_temp) < self.hysteresis_celsius {
                self.last_stable_temp_celsius
            } else {
                self.last_stable_temp_celsius = measured_temp;
                measured_temp
            }
        } else {
            self.last_stable_temp_celsius = measured_temp;
            measured_temp
        };

        let target_pwm = self.curve.evaluate_pwm(effective_temp);

        // Acoustic rate-of-change slewing limiter
        if target_pwm > self.current_pwm {
            // Ramp up limited by max_ramp_up_per_sec
            let max_delta = ((self.max_ramp_up_per_sec as u32 * elapsed_ms) / 1000).max(1) as u8;
            self.current_pwm = (self.current_pwm.saturating_add(max_delta)).min(target_pwm);
        } else if target_pwm < self.current_pwm {
            // Ramp down limited by max_ramp_down_per_sec (gentle spin down)
            let max_delta = ((self.max_ramp_down_per_sec as u32 * elapsed_ms) / 1000).max(1) as u8;
            self.current_pwm = (self.current_pwm.saturating_sub(max_delta)).max(target_pwm);
        }

        self.current_pwm
    }
}

/// Unit test verifying thermal hysteresis damping and acoustic rate-of-change limiter.
pub fn verify_thermal_hysteresis_and_rate_limiter() -> bool {
    let curve = MultiPointFanCurve::balanced_handheld_preset();
    let mut controller = DampedFanController::new(curve);

    // 1. Initial stable state at 50°C (target = 25%)
    // Ramp up over 2 seconds (2000 ms @ 20%/s => can ramp up by 40%, reaches target 25%)
    let pwm1 = controller.update(50, 2000);
    if pwm1 != 25 {
        return false;
    }

    // 2. Temperature drops by 2°C (to 48°C), less than hysteresis threshold (3°C)
    // Controller must remain at 50°C regime (25%) rather than throttling fan immediately
    let pwm2 = controller.update(48, 1000);
    if pwm2 != 25 {
        return false;
    }

    // 3. Temperature drops by 5°C (to 43°C), exceeding hysteresis threshold
    // Target drops towards 43°C regime (~7%), but rate limiter prevents abrupt acoustic dropping
    let pwm3 = controller.update(43, 1000);
    if pwm3 != 20 {
        return false;
    }

    // 4. Critical emergency trip (96°C) overrides rate limiter immediately (instant 100%)
    let pwm_emergency = controller.update(96, 50);
    if pwm_emergency != 100 {
        return false;
    }

    true
}

/// Unit test verifying multi-point fan interpolation curves and PWM duty table.
pub fn verify_fan_curve_interpolation() -> bool {
    let balanced = MultiPointFanCurve::balanced_handheld_preset();

    // 1. Below first point (35°C < 40°C) -> 0%
    if balanced.evaluate_pwm(35) != 0 {
        return false;
    }

    // 2. Exactly at control point (50°C) -> 25%
    if balanced.evaluate_pwm(50) != 25 {
        return false;
    }

    // 3. Linear interpolation between 50°C (25%) and 65°C (45%):
    // At 56°C: delta = 6, range = 15 => pwm = 25 + (20 * 6) / 15 = 25 + 8 = 33%
    let pwm_56 = balanced.evaluate_pwm(56);
    if pwm_56 != 33 {
        return false;
    }

    // 4. Critical emergency trip (96°C >= 95°C) -> 100% blast
    if balanced.evaluate_pwm(96) != 100 {
        return false;
    }

    // 5. Test Cubic Smoothstep mode
    let quiet = MultiPointFanCurve::quiet_handheld_preset();
    let pwm_cubic = quiet.evaluate_pwm(60);
    // 60°C is between 55°C (18%) and 68°C (35%)
    if !(18..=35).contains(&pwm_cubic) {
        return false;
    }

    true
}
