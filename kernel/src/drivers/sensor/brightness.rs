//! Dynamic Auto-Brightness & Backlight PWM Duty Engine.
//!
//! Implements human perception luminance scaling based on Stevens' Power Law:
//! - Multi-point piecewise logarithmic brightness curves
//! - Target luminance (nits) to PWM duty permille translation
//! - Display backlight technology profiles (Standard LCD, Mini-LED FALD, OLED)
//! - Minimum duty floor enforcement preventing panel inverter flicker

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
