//! Unified Handheld PC Hall Effect Magnetic Joystick & Trigger Calibration Engine.
//!
//! Provides contact-free, zero-wear, zero-drift magnetic sensing for handheld gaming PC
//! joysticks (Left/Right Stick X/Y) and analog triggers (L2/R2).
//!
//! Unlike carbon-film potentiometers which suffer from mechanical wear, wiper dust, and
//! progressive analog center drift, Hall effect magnetic sensors detect changes in magnetic
//! flux density (B-field) as the neodymium magnet attached to the gimbal or trigger paddle
//! translates or pivots across a multi-axis Hall IC (e.g. A1304 / TMAG5170 / MLX90393).
//!
//! This module provides:
//! - Raw 12-bit (`0..4095`) and 16-bit (`0..65535`) magnetic flux ADC ingestion.
//! - Temperature drift compensation based on chassis/sensor thermistor telemetry.
//! - Deadzone profiles: Radial (circular Euclidean) and Axial deadzones with inner/outer bounds.
//! - Anti-deadzone compensation to eliminate hardcoded deadzone lag in game engines.
//! - Outer boundary saturation clamping to guarantee 100% deflection in all diagonal vectors.
//! - Circularity normalization algorithms (True Circular vs Squaring the Circle).
//! - Customizable trigger travel response curves (Linear, Exponential, Hair-Trigger, Dual-Stage).
//! - Dynamic self-centering bias tracking with resting state exponential moving average (EMA).


/// Supported ADC resolution for Hall effect sensor interfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdcResolution {
    /// 12-bit ADC (0..4095, nominal center 2048).
    Bits12,
    /// 16-bit ADC (0..65535, nominal center 32768).
    Bits16,
}

impl AdcResolution {
    /// Returns the maximum raw ADC integer value for this resolution.
    #[inline(always)]
    pub const fn max_value(self) -> u16 {
        match self {
            Self::Bits12 => 4095,
            Self::Bits16 => 65535,
        }
    }

    /// Returns the nominal center integer value for this resolution.
    #[inline(always)]
    pub const fn nominal_center(self) -> u16 {
        match self {
            Self::Bits12 => 2048,
            Self::Bits16 => 32768,
        }
    }

    /// Full scale dynamic range in LSB counts.
    #[inline(always)]
    pub const fn full_scale(self) -> f32 {
        match self {
            Self::Bits12 => 4095.0,
            Self::Bits16 => 65535.0,
        }
    }
}

/// Identifies the physical Hall effect sensor channel on the handheld device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HallSensorChannel {
    LeftStickX,
    LeftStickY,
    RightStickX,
    RightStickY,
    LeftTrigger,
    RightTrigger,
}

impl HallSensorChannel {
    /// Returns true if this channel corresponds to an analog trigger.
    #[inline(always)]
    pub const fn is_trigger(self) -> bool {
        matches!(self, Self::LeftTrigger | Self::RightTrigger)
    }

    /// Returns true if this channel corresponds to a joystick axis.
    #[inline(always)]
    pub const fn is_stick(self) -> bool {
        !self.is_trigger()
    }
}

/// Raw ADC sample received from the Hall effect sensor front-end.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawAdcSample {
    /// Sensor channel.
    pub channel: HallSensorChannel,
    /// Raw uncompensated ADC code.
    pub raw_code: u16,
    /// Onboard sensor/chassis temperature in degrees Celsius (°C).
    pub temperature_c: f32,
    /// Hardware timestamp in microseconds.
    pub timestamp_us: u64,
}

impl RawAdcSample {
    /// Creates a new raw ADC sample.
    pub const fn new(
        channel: HallSensorChannel,
        raw_code: u16,
        temperature_c: f32,
        timestamp_us: u64,
    ) -> Self {
        Self {
            channel,
            raw_code,
            temperature_c,
            timestamp_us,
        }
    }
}

/// Calibrated magnetic flux reading in physical units and normalized range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MagneticFluxReading {
    /// Sensor channel.
    pub channel: HallSensorChannel,
    /// Estimated magnetic flux density in milliTesla (mT).
    pub flux_millitesla: f32,
    /// Normalized value: [-1.0, 1.0] for sticks, [0.0, 1.0] for triggers.
    pub normalized: f32,
    /// True if temperature drift compensation was applied.
    pub temp_compensated: bool,
}

/// Deadzone boundary shape evaluation strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HallDeadzoneShape {
    /// Radial Euclidean distance: sqrt(x^2 + y^2).
    /// Avoids axis-clamping "cross" artifacts and ensures uniform circular deadzone.
    Radial,
    /// Axial independent X and Y axis thresholds.
    /// Good for retro 4-way / 8-way directional feel.
    Axial,
}

/// Deadzone configuration profile for joysticks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeadzoneProfile {
    /// Evaluation shape (Radial or Axial).
    pub shape: HallDeadzoneShape,
    /// Inner deadzone threshold [0.0, 1.0].
    /// Raw deflections below this magnitude are squelched to exactly 0.0.
    pub inner_deadzone: f32,
    /// Outer deadzone threshold [0.0, 1.0].
    /// Deflections above this magnitude are clamped to 1.0 full scale.
    pub outer_deadzone: f32,
    /// Anti-deadzone offset [0.0, 0.5].
    /// Instantly boosts output to this minimum floor when breaking out of inner deadzone,
    /// counteracting aggressive game-engine internal deadzones.
    pub anti_deadzone: f32,
}

impl DeadzoneProfile {
    /// Default radial deadzone for modern precision handheld gaming:
    /// 5% inner deadzone, 95% outer clamp, 0% anti-deadzone.
    pub const fn default_radial() -> Self {
        Self {
            shape: HallDeadzoneShape::Radial,
            inner_deadzone: 0.05,
            outer_deadzone: 0.95,
            anti_deadzone: 0.0,
        }
    }

    /// Competitive FPS profile with aggressive anti-deadzone boost:
    /// 3% inner deadzone, 96% outer clamp, 15% anti-deadzone.
    pub const fn competitive_fps() -> Self {
        Self {
            shape: HallDeadzoneShape::Radial,
            inner_deadzone: 0.03,
            outer_deadzone: 0.96,
            anti_deadzone: 0.15,
        }
    }
}

/// Single-axis calibration parameters for a Hall effect sensor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisCalibration {
    /// Raw ADC count at minimum mechanical limit.
    pub raw_min: u16,
    /// Raw ADC count at mechanical neutral center.
    pub raw_center: u16,
    /// Raw ADC count at maximum mechanical limit.
    pub raw_max: u16,
    /// Temperature drift coefficient in fractional sensitivity per °C (e.g. -0.0005 for -0.05%/°C).
    pub temp_coeff_drift: f32,
    /// Reference calibration temperature in degrees Celsius (nominally 25.0°C).
    pub ref_temp_c: f32,
    /// Whether axis polarity is inverted.
    pub invert: bool,
    /// Operating ADC bit depth.
    pub resolution: AdcResolution,
}

impl AxisCalibration {
    /// Creates a 12-bit axis calibration with nominal ranges.
    pub const fn default_12bit(invert: bool) -> Self {
        Self {
            raw_min: 0,
            raw_center: 2048,
            raw_max: 4095,
            temp_coeff_drift: -0.0005, // -0.05% / °C typical for NdFeB magnets + Hall IC
            ref_temp_c: 25.0,
            invert,
            resolution: AdcResolution::Bits12,
        }
    }

    /// Creates a 16-bit axis calibration with nominal ranges.
    pub const fn default_16bit(invert: bool) -> Self {
        Self {
            raw_min: 0,
            raw_center: 32768,
            raw_max: 65535,
            temp_coeff_drift: -0.0005,
            ref_temp_c: 25.0,
            invert,
            resolution: AdcResolution::Bits16,
        }
    }

    /// Applies temperature compensation to raw ADC readings.
    ///
    /// As temperature increases, neodymium magnet flux density decreases by ~0.11%/°C,
    /// while Hall element sensitivity typically increases by ~0.06%/°C, yielding a net
    /// system drift of ~ -0.05%/°C.
    ///
    /// Given ΔT = (T_current - T_ref):
    /// Compensated_Delta = Raw_Delta / (1.0 + TempCoeff * ΔT)
    #[inline]
    pub fn compensate_temperature(&self, raw: u16, temp_c: f32) -> f32 {
        let center = self.raw_center as f32;
        let raw_val = raw as f32;
        let delta = raw_val - center;
        let delta_t = temp_c - self.ref_temp_c;
        let temp_scale = 1.0 + (self.temp_coeff_drift * delta_t);
        // Avoid division by zero or extreme scaling if temp sensor reports an error
        let safe_scale = if temp_scale > 0.5 && temp_scale < 1.5 {
            temp_scale
        } else {
            1.0
        };

        center + (delta / safe_scale)
    }

    /// Normalizes a temperature-compensated raw reading for a joystick axis into [-1.0, 1.0].
    #[inline]
    pub fn normalize_stick_axis(&self, raw: u16, temp_c: f32) -> f32 {
        let comp_val = self.compensate_temperature(raw, temp_c);
        let center = self.raw_center as f32;

        let norm = if comp_val >= center {
            let span = (self.raw_max as f32) - center;
            if span > 1.0 {
                (comp_val - center) / span
            } else {
                0.0
            }
        } else {
            let span = center - (self.raw_min as f32);
            if span > 1.0 {
                (comp_val - center) / span
            } else {
                0.0
            }
        };

        let clamped = if norm > 1.0 {
            1.0
        } else if norm < -1.0 {
            -1.0
        } else {
            norm
        };

        if self.invert {
            -clamped
        } else {
            clamped
        }
    }

    /// Normalizes a temperature-compensated raw reading for an analog trigger into [0.0, 1.0].
    #[inline]
    pub fn normalize_trigger_axis(&self, raw: u16, temp_c: f32) -> f32 {
        let comp_val = self.compensate_temperature(raw, temp_c);
        let min_val = self.raw_min as f32;
        let max_val = self.raw_max as f32;
        let span = max_val - min_val;

        let norm = if span > 1.0 {
            (comp_val - min_val) / span
        } else {
            0.0
        };

        let clamped = if norm < 0.0 {
            0.0
        } else if norm > 1.0 {
            1.0
        } else {
            norm
        };

        if self.invert {
            1.0 - clamped
        } else {
            clamped
        }
    }
}

/// Dual-axis calibration for a 2D Hall effect joystick gimbal (Left or Right Stick).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JoystickCalibration {
    /// Calibration for X axis (horizontal deflection).
    pub axis_x: AxisCalibration,
    /// Calibration for Y axis (vertical deflection).
    pub axis_y: AxisCalibration,
    /// Deadzone profile.
    pub deadzone: DeadzoneProfile,
}

impl JoystickCalibration {
    /// Creates a default 12-bit joystick calibration.
    pub const fn default_12bit() -> Self {
        Self {
            axis_x: AxisCalibration::default_12bit(false),
            axis_y: AxisCalibration::default_12bit(false),
            deadzone: DeadzoneProfile::default_radial(),
        }
    }

    /// Creates a default 16-bit joystick calibration.
    pub const fn default_16bit() -> Self {
        Self {
            axis_x: AxisCalibration::default_16bit(false),
            axis_y: AxisCalibration::default_16bit(false),
            deadzone: DeadzoneProfile::default_radial(),
        }
    }
}

/// Verification helper for Commit 1 structures and algorithms.
pub fn verify_calibration_and_adc_profiles() -> bool {
    // 1. Verify ADC resolutions
    let r12 = AdcResolution::Bits12;
    if r12.max_value() != 4095 || r12.nominal_center() != 2048 {
        return false;
    }
    let r16 = AdcResolution::Bits16;
    if r16.max_value() != 65535 || r16.nominal_center() != 32768 {
        return false;
    }

    // 2. Verify Channel properties
    if !HallSensorChannel::LeftTrigger.is_trigger() || HallSensorChannel::LeftStickX.is_trigger() {
        return false;
    }
    if !HallSensorChannel::RightStickY.is_stick() || HallSensorChannel::RightTrigger.is_stick() {
        return false;
    }

    // 3. Verify Temperature Drift Compensation
    let cal12 = AxisCalibration::default_12bit(false);
    // At reference temperature (25°C), compensated == raw
    let comp_at_ref = cal12.compensate_temperature(3072, 25.0);
    if (comp_at_ref - 3072.0).abs() > 0.01 {
        return false;
    }

    // At elevated temperature (65°C -> ΔT = +40°C), magnet weakens:
    // temp_scale = 1.0 + (-0.0005 * 40) = 0.98.
    // Compensated delta should be larger than raw delta to counteract weakening.
    let comp_hot = cal12.compensate_temperature(3072, 65.0);
    let raw_delta = 3072.0 - 2048.0; // 1024.0
    let expected_delta = raw_delta / 0.98; // ~1044.8979
    if (comp_hot - (2048.0 + expected_delta)).abs() > 0.1 {
        return false;
    }

    // 4. Verify Stick Normalization
    let center_norm = cal12.normalize_stick_axis(2048, 25.0);
    if center_norm.abs() > 1e-4 {
        return false;
    }
    let max_norm = cal12.normalize_stick_axis(4095, 25.0);
    if (max_norm - 1.0).abs() > 1e-4 {
        return false;
    }
    let min_norm = cal12.normalize_stick_axis(0, 25.0);
    if (min_norm - (-1.0)).abs() > 1e-4 {
        return false;
    }

    // 5. Verify Inverted Axis
    let cal_inv = AxisCalibration::default_12bit(true);
    let inv_max_norm = cal_inv.normalize_stick_axis(4095, 25.0);
    if (inv_max_norm - (-1.0)).abs() > 1e-4 {
        return false;
    }

    // 6. Verify Trigger Normalization
    let trig_zero = cal12.normalize_trigger_axis(0, 25.0);
    if trig_zero.abs() > 1e-4 {
        return false;
    }
    let trig_full = cal12.normalize_trigger_axis(4095, 25.0);
    if (trig_full - 1.0).abs() > 1e-4 {
        return false;
    }

    true
}
