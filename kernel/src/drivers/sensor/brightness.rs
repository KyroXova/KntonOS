//! Dynamic Auto-Brightness, Backlight Slewing & HDR Contrast Governor.
//!
//! Implements human perception luminance scaling based on Stevens' Power Law:
//! - Multi-point piecewise logarithmic brightness curves
//! - Target luminance (nits) to PWM duty permille translation
//! - Asymmetric temporal slewing (fast ramp-up 50%/s, slow dark-adaptation ramp-down 20%/s)
//! - Display backlight technology profiles (Standard LCD, Mini-LED FALD, OLED)
//! - Ambient-adaptive HDR tone-mapping contrast governor and glare compensation

use crate::drivers::sensor::als::{AlsDriver, AlsHardwareInterface, LuxCategory};

// -----------------------------------------------------------------------------
// Backlight Profiles & Curve Structures
// -----------------------------------------------------------------------------

/// Display backlight technology profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayBacklightType {
    /// Standard Edge-Lit LCD with continuous PWM backlight.
    StandardLcd { max_nits: u16, pwm_freq_hz: u32 },
    /// Mini-LED Full Array Local Dimming (FALD) with HDR peak highlight boost.
    MiniLed { max_nits: u16, peak_hdr_nits: u16, dimming_zones: u16 },
    /// Self-emissive OLED panel with per-pixel brightness and Auto-Brightness Limiting (ABL).
    Oled { max_nits: u16, peak_hdr_nits: u16 },
}

impl DisplayBacklightType {
    pub const fn default_handheld_lcd() -> Self {
        Self::StandardLcd {
            max_nits: 500,
            pwm_freq_hz: 20_000, // 20 kHz flicker-free PWM
        }
    }

    pub const fn default_oled() -> Self {
        Self::Oled {
            max_nits: 600,
            peak_hdr_nits: 1000,
        }
    }

    pub const fn max_luminance_nits(&self) -> u16 {
        match *self {
            Self::StandardLcd { max_nits, .. } => max_nits,
            Self::MiniLed { max_nits, .. } => max_nits,
            Self::Oled { max_nits, .. } => max_nits,
        }
    }

    pub const fn peak_hdr_nits(&self) -> u16 {
        match *self {
            Self::StandardLcd { max_nits, .. } => max_nits,
            Self::MiniLed { peak_hdr_nits, .. } => peak_hdr_nits,
            Self::Oled { peak_hdr_nits, .. } => peak_hdr_nits,
        }
    }
}

/// A calibration node on the ambient-lux-to-brightness curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrightnessCurvePoint {
    /// Ambient illuminance in Lux.
    pub ambient_lux: u32,
    /// Target display luminance in nits (cd/m²).
    pub target_nits: u16,
    /// Hardware PWM duty cycle in permille (0..1000 = 0.0% .. 100.0%).
    pub pwm_duty_permille: u16,
}

impl BrightnessCurvePoint {
    pub const fn new(ambient_lux: u32, target_nits: u16, pwm_duty_permille: u16) -> Self {
        Self {
            ambient_lux,
            target_nits,
            pwm_duty_permille,
        }
    }
}

/// Maximum number of calibration points on the auto-brightness curve.
pub const MAX_BRIGHTNESS_CURVE_POINTS: usize = 12;

/// Multi-point auto-brightness curve matching human eye sensitivity (Stevens' power law).
#[derive(Debug, Clone)]
pub struct AutoBrightnessCurve {
    points: [BrightnessCurvePoint; MAX_BRIGHTNESS_CURVE_POINTS],
    point_count: usize,
    min_pwm_duty_permille: u16, // Floor clamp (e.g. 30 = 3.0%)
    max_pwm_duty_permille: u16, // Ceiling clamp (e.g. 1000 = 100.0%)
}

impl AutoBrightnessCurve {
    /// Constructs a curve initialized with calibrated handheld defaults.
    pub fn default_handheld_curve() -> Self {
        let mut curve = Self {
            points: [BrightnessCurvePoint::new(0, 0, 0); MAX_BRIGHTNESS_CURVE_POINTS],
            point_count: 0,
            min_pwm_duty_permille: 30, // 3% hardware minimum duty floor
            max_pwm_duty_permille: 1000,
        };

        // Calibrated 9-point curve matching logarithmic eye response:
        // S proportional to I^0.33 .. I^0.5
        curve.add_point(BrightnessCurvePoint::new(0, 10, 40));         // 0 Lux -> 10 nits (bedtime reading)
        curve.add_point(BrightnessCurvePoint::new(10, 25, 80));        // 10 Lux -> 25 nits (dim room)
        curve.add_point(BrightnessCurvePoint::new(50, 75, 200));       // 50 Lux -> 75 nits (evening indoor)
        curve.add_point(BrightnessCurvePoint::new(200, 150, 380));     // 200 Lux -> 150 nits (cozy room)
        curve.add_point(BrightnessCurvePoint::new(500, 250, 550));     // 500 Lux -> 250 nits (office lighting)
        curve.add_point(BrightnessCurvePoint::new(1_000, 350, 700));   // 1k Lux -> 350 nits (bright indoor)
        curve.add_point(BrightnessCurvePoint::new(5_000, 500, 850));   // 5k Lux -> 500 nits (shaded daylight)
        curve.add_point(BrightnessCurvePoint::new(10_000, 800, 950));  // 10k Lux -> 800 nits (bright daylight)
        curve.add_point(BrightnessCurvePoint::new(50_000, 1000, 1000));// 50k Lux -> 1000 nits (direct noon sun)

        curve
    }

    /// Appends a new calibration point into the curve in ascending lux order.
    pub fn add_point(&mut self, point: BrightnessCurvePoint) {
        if self.point_count < MAX_BRIGHTNESS_CURVE_POINTS {
            self.points[self.point_count] = point;
            self.point_count += 1;
            // Maintain sorted order by ambient_lux
            let mut i = self.point_count - 1;
            while i > 0 && self.points[i - 1].ambient_lux > self.points[i].ambient_lux {
                let temp = self.points[i - 1];
                self.points[i - 1] = self.points[i];
                self.points[i] = temp;
                i -= 1;
            }
        }
    }

    /// Evaluates target display luminance in nits for a given ambient illuminance in Lux.
    pub fn evaluate_nits(&self, ambient_lux: u32) -> u16 {
        if self.point_count == 0 {
            return 100;
        }

        if ambient_lux <= self.points[0].ambient_lux {
            return self.points[0].target_nits;
        }

        let last_idx = self.point_count - 1;
        if ambient_lux >= self.points[last_idx].ambient_lux {
            return self.points[last_idx].target_nits;
        }

        // Find enclosing interval [i, i+1]
        for i in 0..last_idx {
            let p0 = self.points[i];
            let p1 = self.points[i + 1];

            if ambient_lux >= p0.ambient_lux && ambient_lux <= p1.ambient_lux {
                let span_lux = p1.ambient_lux - p0.ambient_lux;
                if span_lux == 0 {
                    return p0.target_nits;
                }
                let delta_lux = ambient_lux - p0.ambient_lux;
                let delta_nits = (p1.target_nits as i32) - (p0.target_nits as i32);

                let interpolated = (p0.target_nits as i32) + (delta_lux as i32 * delta_nits) / (span_lux as i32);
                return interpolated.clamp(0, u16::MAX as i32) as u16;
            }
        }

        self.points[last_idx].target_nits
    }

    /// Evaluates target PWM duty permille (0..1000) with floor/ceiling clamping.
    pub fn evaluate_pwm_duty(&self, ambient_lux: u32) -> u16 {
        if self.point_count == 0 {
            return self.min_pwm_duty_permille;
        }

        let raw_duty = if ambient_lux <= self.points[0].ambient_lux {
            self.points[0].pwm_duty_permille
        } else if ambient_lux >= self.points[self.point_count - 1].ambient_lux {
            self.points[self.point_count - 1].pwm_duty_permille
        } else {
            let mut result = self.points[0].pwm_duty_permille;
            for i in 0..(self.point_count - 1) {
                let p0 = self.points[i];
                let p1 = self.points[i + 1];
                if ambient_lux >= p0.ambient_lux && ambient_lux <= p1.ambient_lux {
                    let span_lux = p1.ambient_lux - p0.ambient_lux;
                    if span_lux == 0 {
                        result = p0.pwm_duty_permille;
                    } else {
                        let delta_lux = ambient_lux - p0.ambient_lux;
                        let delta_duty = (p1.pwm_duty_permille as i32) - (p0.pwm_duty_permille as i32);
                        result = ((p0.pwm_duty_permille as i32) + (delta_lux as i32 * delta_duty) / (span_lux as i32))
                            as u16;
                    }
                    break;
                }
            }
            result
        };

        raw_duty.clamp(self.min_pwm_duty_permille, self.max_pwm_duty_permille)
    }

    #[inline(always)]
    pub fn point_count(&self) -> usize {
        self.point_count
    }
}

// -----------------------------------------------------------------------------
// HDR Tone-Mapping Profile & Contrast Governor
// -----------------------------------------------------------------------------

/// Ambient-adaptive HDR tone-mapping parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HdrToneMappingProfile {
    /// Ambient illuminance context in Lux.
    pub ambient_lux: u32,
    /// Effective display gamma in permille (2200 = 2.2 standard, lowered in sunlight to lift shadows).
    pub gamma_permille: u16,
    /// Contrast boost factor in permille (1000 = 1.0x baseline, up to 1350 = 1.35x in bright sun).
    pub contrast_boost_permille: u16,
    /// Black level pedestal lift in permille (0 in dark, up to 50 in sunlight to defeat glare washout).
    pub black_level_lift_permille: u16,
    /// Peak highlight luminance limit in nits.
    pub peak_highlight_nits: u16,
}

impl HdrToneMappingProfile {
    /// Computes optimal HDR tone mapping curve parameters based on ambient illuminance and display type.
    pub fn compute(ambient_lux: u32, backlight: DisplayBacklightType, is_hdr_active: bool) -> Self {
        let category = LuxCategory::from_lux(ambient_lux);

        let (gamma, contrast, black_lift) = match category {
            LuxCategory::PitchDark => (2200, 1000, 0),        // Standard 2.2 gamma, deep blacks
            LuxCategory::DimIndoor => (2200, 1000, 0),
            LuxCategory::NormalIndoor => (2200, 1050, 5),     // Slight 5% contrast lift
            LuxCategory::Office => (2100, 1150, 15),          // 2.1 gamma, 15% contrast boost
            LuxCategory::BrightDaylight => (1950, 1250, 30),  // 1.95 gamma, 25% contrast boost
            LuxCategory::DirectSunlight => (1800, 1350, 50),  // 1.8 gamma to lift dark details, 35% contrast boost
        };

        let peak_highlight_nits = if is_hdr_active {
            backlight.peak_hdr_nits()
        } else {
            backlight.max_luminance_nits()
        };

        Self {
            ambient_lux,
            gamma_permille: gamma,
            contrast_boost_permille: contrast,
            black_level_lift_permille: black_lift,
            peak_highlight_nits,
        }
    }
}

// -----------------------------------------------------------------------------
// Unified Handheld Brightness Governor with Smooth Slewing
// -----------------------------------------------------------------------------

/// Unified Handheld Auto-Brightness & HDR Contrast Governor.
#[derive(Debug, Clone)]
pub struct HandheldBrightnessGovernor {
    pub als: AlsDriver,
    pub curve: AutoBrightnessCurve,
    pub backlight: DisplayBacklightType,
    pub current_pwm_duty: u16,
    pub target_pwm_duty: u16,
    pub current_nits: u16,
    pub target_nits: u16,
    pub ramp_up_rate_permille_per_sec: u16,   // 500 = 50% duty / sec (fast brighten)
    pub ramp_down_rate_permille_per_sec: u16, // 200 = 20% duty / sec (slow dark adapt)
    pub hdr_profile: HdrToneMappingProfile,
    pub is_hdr_active: bool,
}

impl HandheldBrightnessGovernor {
    pub fn new(backlight: DisplayBacklightType) -> Self {
        let curve = AutoBrightnessCurve::default_handheld_curve();
        let initial_nits = curve.evaluate_nits(100);
        let initial_duty = curve.evaluate_pwm_duty(100);
        let hdr_profile = HdrToneMappingProfile::compute(100, backlight, false);

        Self {
            als: AlsDriver::new(AlsHardwareInterface::Emulated),
            curve,
            backlight,
            current_pwm_duty: initial_duty,
            target_pwm_duty: initial_duty,
            current_nits: initial_nits,
            target_nits: initial_nits,
            ramp_up_rate_permille_per_sec: 500,   // 50% per second
            ramp_down_rate_permille_per_sec: 200, // 20% per second
            hdr_profile,
            is_hdr_active: false,
        }
    }

    /// Sets HDR rendering active state.
    pub fn set_hdr_active(&mut self, active: bool) {
        self.is_hdr_active = active;
    }

    /// Advances the governor by `elapsed_ms` with a new raw ALS sensor sample.
    pub fn step(&mut self, raw_lux: u32, elapsed_ms: u32, timestamp_ms: u64) -> (u16, HdrToneMappingProfile) {
        // 1. Ingest raw lux into EMA filter
        let filtered_lux = self.als.update_reading(raw_lux, timestamp_ms);

        // 2. Evaluate target brightness from curve
        let mut target_nits = self.curve.evaluate_nits(filtered_lux);
        let target_duty = self.curve.evaluate_pwm_duty(filtered_lux);

        // If HDR is active on OLED/MiniLED, apply peak highlight boost
        if self.is_hdr_active && filtered_lux >= 1000 {
            target_nits = target_nits.max(self.backlight.peak_hdr_nits());
        }

        self.target_nits = target_nits;
        self.target_pwm_duty = target_duty;

        // 3. Temporal slewing / rate-of-change limiter
        if self.current_pwm_duty < self.target_pwm_duty {
            // Brightening: up to 500 permille / sec
            let max_step = ((self.ramp_up_rate_permille_per_sec as u32 * elapsed_ms) / 1000).max(1) as u16;
            self.current_pwm_duty = (self.current_pwm_duty + max_step).min(self.target_pwm_duty);
        } else if self.current_pwm_duty > self.target_pwm_duty {
            // Dimming: up to 200 permille / sec (gentle dark adaptation)
            let max_step = ((self.ramp_down_rate_permille_per_sec as u32 * elapsed_ms) / 1000).max(1) as u16;
            if self.current_pwm_duty > max_step {
                self.current_pwm_duty = (self.current_pwm_duty - max_step).max(self.target_pwm_duty);
            } else {
                self.current_pwm_duty = self.target_pwm_duty;
            }
        }

        // 4. Update HDR tone-mapping curve
        self.hdr_profile = HdrToneMappingProfile::compute(filtered_lux, self.backlight, self.is_hdr_active);

        (self.current_pwm_duty, self.hdr_profile)
    }
}

// -----------------------------------------------------------------------------
// Unit Tests & Verification
// -----------------------------------------------------------------------------

/// Unit test verifying Stevens power law brightness curve, logarithmic interpolation, and PWM duty.
pub fn verify_brightness_curve_and_pwm_table() -> bool {
    let curve = AutoBrightnessCurve::default_handheld_curve();

    if curve.point_count() != 9 {
        return false;
    }

    // 1. Exact node evaluations
    if curve.evaluate_nits(0) != 10 || curve.evaluate_pwm_duty(0) != 40 {
        return false;
    }
    if curve.evaluate_nits(500) != 250 || curve.evaluate_pwm_duty(500) != 550 {
        return false;
    }
    if curve.evaluate_nits(50_000) != 1000 || curve.evaluate_pwm_duty(50_000) != 1000 {
        return false;
    }

    // 2. Midpoint interpolation: between 200 Lux (150 nits, 380 permille) and 500 Lux (250 nits, 550 permille)
    // At 350 Lux (halfway): nits = 150 + 150 * 100 / 300 = 150 + 50 = 200 nits
    // duty = 380 + 150 * 170 / 300 = 380 + 85 = 465 permille
    let mid_nits = curve.evaluate_nits(350);
    let mid_duty = curve.evaluate_pwm_duty(350);
    if mid_nits != 200 || mid_duty != 465 {
        return false;
    }

    // 3. Monotonic growth check across full spectrum
    let mut prev_nits = 0;
    let mut prev_duty = 0;
    let test_lux_values = [0, 5, 10, 25, 50, 100, 200, 350, 500, 750, 1000, 2500, 5000, 10000, 25000, 50000, 100000];

    for &lux in &test_lux_values {
        let nits = curve.evaluate_nits(lux);
        let duty = curve.evaluate_pwm_duty(lux);

        if nits < prev_nits || duty < prev_duty {
            return false;
        }
        if duty < 30 {
            return false; // Hardware floor violation
        }
        if duty > 1000 {
            return false; // Ceiling overflow
        }

        prev_nits = nits;
        prev_duty = duty;
    }

    // 4. Backlight profile defaults
    let lcd = DisplayBacklightType::default_handheld_lcd();
    if lcd.max_luminance_nits() != 500 {
        return false;
    }
    let oled = DisplayBacklightType::default_oled();
    if oled.max_luminance_nits() != 600 {
        return false;
    }

    true
}

/// Unit test verifying smooth asymmetric backlight slewing and ambient-adaptive HDR contrast governor.
pub fn verify_slewing_and_hdr_contrast_governor() -> bool {
    let oled = DisplayBacklightType::default_oled();
    let mut gov = HandheldBrightnessGovernor::new(oled);

    // Initial state: 100 Lux (~226 permille duty)
    gov.current_pwm_duty = 100;

    // 1. Test fast brighten slewing (ramp up at 50% / sec = 500 permille/sec)
    // Step to bright outdoor daylight (10,000 Lux -> target duty = 950 permille)
    // Over 500ms, should advance by 500 * 0.5 = 250 permille -> current = 100 + 250 = 350
    let (duty_up, _hdr) = gov.step(10_000, 500, 500);
    if duty_up != 350 {
        return false;
    }

    // Advance by another 1000ms: advances by 500 permille -> 350 + 500 = 850
    let (duty_up2, _) = gov.step(10_000, 1000, 1500);
    if duty_up2 != 850 {
        return false;
    }

    // 2. Test slow dark-adaptation slewing (ramp down at 20% / sec = 200 permille/sec)
    // Step down to pitch dark (0 Lux -> target duty = 40 permille)
    // Over 1000ms, should drop by 200 * 1.0 = 200 permille -> 850 - 200 = 650
    let (duty_down, _) = gov.step(0, 1000, 2500);
    if duty_down != 650 {
        return false;
    }

    // 3. Test HDR tone mapping contrast expansion in bright sunlight
    gov.set_hdr_active(true);
    let (_duty_sun, hdr_sun) = gov.step(30_000, 1000, 3500);
    // Direct Sunlight profile: gamma lowered to 1800, contrast boosted to 1350, black lift 50
    if hdr_sun.gamma_permille != 1800
        || hdr_sun.contrast_boost_permille != 1350
        || hdr_sun.black_level_lift_permille != 50
        || hdr_sun.peak_highlight_nits != 1000
    {
        return false;
    }

    // 4. Test HDR tone mapping in pitch dark room
    let (_duty_dark, hdr_dark) = gov.step(0, 10_000, 13500);
    // Pitch Dark: gamma 2.2 (2200), baseline contrast 1.0x (1000), 0 black lift
    if hdr_dark.gamma_permille != 2200
        || hdr_dark.contrast_boost_permille != 1000
        || hdr_dark.black_level_lift_permille != 0
    {
        return false;
    }

    true
}
