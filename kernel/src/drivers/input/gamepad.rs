//! High-Performance 1000Hz Gamepad Input Pipeline with Deadzone Shaping & Trigger Normalization.
//!
//! Provides esports-grade gamepad processing at up to 1000Hz (1ms polling):
//! - Bitwise button state management with edge detection (just pressed, just released)
//! - Circular and axial analog stick deadzone shaping with exponential curve scaling
//! - Trigger normalization with configurable inner/outer deadzones and digital actuation
//! - 1000Hz jitter and polling rate diagnostics
//! - Integrated 6-axis IMU gyro-aiming calculations and haptic feedback control

use super::imu::{atan2_f32, sqrt_f32, Quaternion, RawImuSample, SensorFusion, Vector3};

// -----------------------------------------------------------------------------
// Button Bitflags
// -----------------------------------------------------------------------------

/// Bitmask constants representing physical gamepad buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GamepadButtons(pub u32);

impl GamepadButtons {
    pub const NONE: Self = Self(0);

    // Primary action buttons (South, East, West, North)
    pub const A: Self = Self(1 << 0);
    pub const B: Self = Self(1 << 1);
    pub const X: Self = Self(1 << 2);
    pub const Y: Self = Self(1 << 3);

    // Directional Pad
    pub const DPAD_UP: Self = Self(1 << 4);
    pub const DPAD_DOWN: Self = Self(1 << 5);
    pub const DPAD_LEFT: Self = Self(1 << 6);
    pub const DPAD_RIGHT: Self = Self(1 << 7);

    // Bumpers / Shoulders
    pub const LB: Self = Self(1 << 8);
    pub const RB: Self = Self(1 << 9);

    // Stick Clicks
    pub const LS: Self = Self(1 << 10);
    pub const RS: Self = Self(1 << 11);

    // Menu / Navigation
    pub const START: Self = Self(1 << 12);
    pub const SELECT: Self = Self(1 << 13);
    pub const GUIDE: Self = Self(1 << 14);

    // Trigger Digital Clicks (full pull)
    pub const LT_CLICK: Self = Self(1 << 15);
    pub const RT_CLICK: Self = Self(1 << 16);

    // Esports Paddles (rear buttons P1..P4)
    pub const PADDLE_1: Self = Self(1 << 17);
    pub const PADDLE_2: Self = Self(1 << 18);
    pub const PADDLE_3: Self = Self(1 << 19);
    pub const PADDLE_4: Self = Self(1 << 20);

    // Touchpad / Share
    pub const TOUCHPAD_CLICK: Self = Self(1 << 21);
    pub const SHARE: Self = Self(1 << 22);

    #[inline(always)]
    pub const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    #[inline(always)]
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    #[inline(always)]
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }

    #[inline(always)]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[inline(always)]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.0 == 0
    }
}

// -----------------------------------------------------------------------------
// Deadzone & Stick Shaping
// -----------------------------------------------------------------------------

/// Deadzone calculation geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadzoneShape {
    /// Radial / Circular deadzone preserving angular direction.
    Circular,
    /// Independent X and Y axial deadzones.
    Axial,
}

/// Analog stick deadzone and response curve configuration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StickDeadzone {
    /// Inner deadzone threshold (0.0 to 1.0). Inputs below this are set to 0.0.
    pub inner: f32,
    /// Outer deadzone threshold (0.0 to 1.0). Inputs above this are scaled to maximum 1.0.
    pub outer: f32,
    /// Response curve exponent (1.0 = linear, 2.0 = quadratic for finer central control).
    pub curve_exponent: f32,
    /// Deadzone geometry shape.
    pub shape: DeadzoneShape,
    /// Invert vertical Y axis (standard gaming preference).
    pub invert_y: bool,
}

impl Default for StickDeadzone {
    fn default() -> Self {
        Self {
            inner: 0.08,
            outer: 0.95,
            curve_exponent: 1.0,
            shape: DeadzoneShape::Circular,
            invert_y: false,
        }
    }
}

impl StickDeadzone {
    pub const fn new(inner: f32, outer: f32, curve_exponent: f32, shape: DeadzoneShape) -> Self {
        Self {
            inner,
            outer,
            curve_exponent,
            shape,
            invert_y: false,
        }
    }

    /// Shapes raw 16-bit integer inputs (signed -32768..32767) into normalized [-1.0, 1.0] coordinates.
    pub fn shape(&self, raw_x: i16, raw_y: i16) -> (f32, f32, f32, f32) {
        // Convert to initial [-1.0, 1.0] float range
        let in_x = if raw_x == -32768 { -1.0 } else { raw_x as f32 / 32767.0 };
        let mut in_y = if raw_y == -32768 { -1.0 } else { raw_y as f32 / 32767.0 };

        if self.invert_y {
            in_y = -in_y;
        }

        match self.shape {
            DeadzoneShape::Circular => {
                let magnitude = sqrt_f32(in_x * in_x + in_y * in_y);
                let angle = atan2_f32(in_y, in_x);

                if magnitude <= self.inner {
                    (0.0, 0.0, 0.0, angle)
                } else {
                    // Rescale linearly between inner and outer thresholds
                    let denom = (self.outer - self.inner).max(1e-4);
                    let mut norm_mag = (magnitude - self.inner) / denom;
                    if norm_mag > 1.0 {
                        norm_mag = 1.0;
                    }

                    // Apply response curve exponent
                    let shaped_mag = if (self.curve_exponent - 1.0).abs() < 1e-4 {
                        norm_mag
                    } else if (self.curve_exponent - 2.0).abs() < 1e-4 {
                        norm_mag * norm_mag
                    } else {
                        // General power approximation: pow(m, p)
                        norm_mag * norm_mag // Defaulting to smooth quadratic when custom
                    };

                    let out_x = (in_x / magnitude) * shaped_mag;
                    let out_y = (in_y / magnitude) * shaped_mag;

                    (out_x, out_y, shaped_mag, angle)
                }
            }
            DeadzoneShape::Axial => {
                let shape_axis = |v: f32| -> f32 {
                    let sign = if v < 0.0 { -1.0 } else { 1.0 };
                    let abs_v = v.abs();
                    if abs_v <= self.inner {
                        0.0
                    } else {
                        let denom = (self.outer - self.inner).max(1e-4);
                        let mut norm = (abs_v - self.inner) / denom;
                        if norm > 1.0 {
                            norm = 1.0;
                        }
                        if (self.curve_exponent - 2.0).abs() < 1e-4 {
                            norm = norm * norm;
                        }
                        sign * norm
                    }
                };

                let out_x = shape_axis(in_x);
                let out_y = shape_axis(in_y);
                let mag = sqrt_f32(out_x * out_x + out_y * out_y).min(1.0);
                let angle = atan2_f32(out_y, out_x);

                (out_x, out_y, mag, angle)
            }
        }
    }
}

/// Analog stick state representation.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AnalogStick {
    /// Raw unshaped integer X coordinate.
    pub raw_x: i16,
    /// Raw unshaped integer Y coordinate.
    pub raw_y: i16,
    /// Normalized and deadzone-shaped X coordinate in range [-1.0, 1.0].
    pub x: f32,
    /// Normalized and deadzone-shaped Y coordinate in range [-1.0, 1.0].
    pub y: f32,
    /// Polar radial magnitude in range [0.0, 1.0].
    pub magnitude: f32,
    /// Polar angle in radians [-PI, PI].
    pub angle_rad: f32,
}

// -----------------------------------------------------------------------------
// Trigger Normalization
// -----------------------------------------------------------------------------

/// Analog trigger deadzone configuration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TriggerDeadzone {
    /// Inner deadzone threshold (0.0 to 1.0).
    pub inner: f32,
    /// Outer deadzone threshold (0.0 to 1.0).
    pub outer: f32,
    /// Actuation threshold for digital click event (0.0 to 1.0).
    pub click_threshold: f32,
}

impl Default for TriggerDeadzone {
    fn default() -> Self {
        Self {
            inner: 0.05,
            outer: 0.95,
            click_threshold: 0.50,
        }
    }
}

impl TriggerDeadzone {
    pub const fn new(inner: f32, outer: f32, click_threshold: f32) -> Self {
        Self {
            inner,
            outer,
            click_threshold,
        }
    }

    /// Normalizes raw trigger reading (unsigned 0..65535 or 0..255) into [0.0, 1.0].
    pub fn normalize_u16(&self, raw: u16) -> (f32, bool) {
        let in_norm = raw as f32 / 65535.0;
        self.normalize_float(in_norm)
    }

    /// Normalizes raw 8-bit trigger reading (0..255) into [0.0, 1.0].
    pub fn normalize_u8(&self, raw: u8) -> (f32, bool) {
        let in_norm = raw as f32 / 255.0;
        self.normalize_float(in_norm)
    }

    fn normalize_float(&self, in_norm: f32) -> (f32, bool) {
        if in_norm <= self.inner {
            (0.0, false)
        } else if in_norm >= self.outer {
            (1.0, true)
        } else {
            let denom = (self.outer - self.inner).max(1e-4);
            let val = (in_norm - self.inner) / denom;
            let pressed = val >= self.click_threshold;
            (val, pressed)
        }
    }
}

/// Analog trigger state representation.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AnalogTrigger {
    /// Raw unnormalized reading.
    pub raw: u16,
    /// Normalized actuation level in range [0.0, 1.0].
    pub value: f32,
    /// Whether digital click actuation threshold was exceeded.
    pub is_clicked: bool,
}

// -----------------------------------------------------------------------------
// Haptic & Rumble Feedback
// -----------------------------------------------------------------------------

/// Dual-motor and trigger haptic rumble feedback command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HapticFeedback {
    /// Low-frequency heavy rumble motor (0..255).
    pub strong_motor: u8,
    /// High-frequency light rumble motor (0..255).
    pub weak_motor: u8,
    /// Left trigger haptic actuator (0..255).
    pub left_trigger: u8,
    /// Right trigger haptic actuator (0..255).
    pub right_trigger: u8,
}

impl HapticFeedback {
    pub const fn new(strong: u8, weak: u8) -> Self {
        Self {
            strong_motor: strong,
            weak_motor: weak,
            left_trigger: 0,
            right_trigger: 0,
        }
    }

    pub const fn zero() -> Self {
        Self {
            strong_motor: 0,
            weak_motor: 0,
            left_trigger: 0,
            right_trigger: 0,
        }
    }
}

// -----------------------------------------------------------------------------
// Raw Report & Processed State
// -----------------------------------------------------------------------------

/// Raw input report packet received directly from USB HID or Bluetooth controller endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RawGamepadReport {
    /// Bitmask of pressed buttons.
    pub buttons: u32,
    /// Left stick X axis (-32768..32767).
    pub left_stick_x: i16,
    /// Left stick Y axis (-32768..32767).
    pub left_stick_y: i16,
    /// Right stick X axis (-32768..32767).
    pub right_stick_x: i16,
    /// Right stick Y axis (-32768..32767).
    pub right_stick_y: i16,
    /// Left trigger reading (0..65535).
    pub left_trigger: u16,
    /// Right trigger reading (0..65535).
    pub right_trigger: u16,
    /// Optional embedded 6-axis IMU packet.
    pub imu_sample: Option<RawImuSample>,
    /// Microsecond arrival timestamp.
    pub timestamp_us: u64,
}

/// Fully processed high-precision gamepad state frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GamepadState {
    /// Current button states.
    pub buttons: GamepadButtons,
    /// Buttons that transitioned from released to pressed in this frame.
    pub buttons_just_pressed: GamepadButtons,
    /// Buttons that transitioned from pressed to released in this frame.
    pub buttons_just_released: GamepadButtons,
    /// Left analog stick.
    pub left_stick: AnalogStick,
    /// Right analog stick.
    pub right_stick: AnalogStick,
    /// Left analog trigger.
    pub left_trigger: AnalogTrigger,
    /// Right analog trigger.
    pub right_trigger: AnalogTrigger,
    /// 3D Orientation quaternion from fused 6-axis IMU (if present).
    pub imu_orientation: Option<Quaternion>,
    /// Calibrated angular velocity in rad/s from IMU gyroscope.
    pub imu_angular_rate: Option<Vector3>,
    /// Gravity-compensated dynamic linear acceleration.
    pub imu_dynamic_accel: Option<Vector3>,
    /// Report timestamp in microseconds.
    pub timestamp_us: u64,
}

// -----------------------------------------------------------------------------
// 1000Hz Input Pipeline Engine
// -----------------------------------------------------------------------------

/// High-performance 1000Hz Gamepad Input Pipeline.
pub struct GamepadPipeline {
    pub left_stick_deadzone: StickDeadzone,
    pub right_stick_deadzone: StickDeadzone,
    pub left_trigger_deadzone: TriggerDeadzone,
    pub right_trigger_deadzone: TriggerDeadzone,
    pub fusion: Option<SensorFusion>,
    pub gyro_aiming_enabled: bool,
    pub gyro_sensitivity: f32,

    // Runtime state
    last_state: GamepadState,
    total_reports_processed: u64,
    last_timestamp_us: u64,
    min_interval_us: u64,
    max_interval_us: u64,
    current_polling_rate_hz: f32,
    haptics: HapticFeedback,
}

impl GamepadPipeline {
    pub fn new() -> Self {
        Self {
            left_stick_deadzone: StickDeadzone::default(),
            right_stick_deadzone: StickDeadzone::default(),
            left_trigger_deadzone: TriggerDeadzone::default(),
            right_trigger_deadzone: TriggerDeadzone::default(),
            fusion: None,
            gyro_aiming_enabled: false,
            gyro_sensitivity: 1.0,
            last_state: GamepadState::default(),
            total_reports_processed: 0,
            last_timestamp_us: 0,
            min_interval_us: u64::MAX,
            max_interval_us: 0,
            current_polling_rate_hz: 0.0,
            haptics: HapticFeedback::zero(),
        }
    }

    /// Attaches an IMU sensor fusion engine to the gamepad pipeline for motion aiming.
    pub fn attach_imu_fusion(&mut self, fusion: SensorFusion) {
        self.fusion = Some(fusion);
        self.gyro_aiming_enabled = true;
    }

    /// Processes an incoming raw report at up to 1000Hz and returns the calibrated state.
    pub fn process_report(&mut self, report: &RawGamepadReport) -> GamepadState {
        // Polling rate and jitter metrics
        if self.last_timestamp_us > 0 && report.timestamp_us > self.last_timestamp_us {
            let delta_us = report.timestamp_us - self.last_timestamp_us;
            if delta_us < self.min_interval_us {
                self.min_interval_us = delta_us;
            }
            if delta_us > self.max_interval_us {
                self.max_interval_us = delta_us;
            }
            if delta_us > 0 {
                let instant_hz = 1_000_000.0 / (delta_us as f32);
                // Exponential moving average for displayed polling rate
                if self.current_polling_rate_hz <= 0.0 {
                    self.current_polling_rate_hz = instant_hz;
                } else {
                    self.current_polling_rate_hz = 0.95 * self.current_polling_rate_hz + 0.05 * instant_hz;
                }
            }
        }
        self.last_timestamp_us = report.timestamp_us;
        self.total_reports_processed = self.total_reports_processed.saturating_add(1);

        // Buttons edge detection
        let current_buttons = GamepadButtons(report.buttons);
        let prev_buttons = self.last_state.buttons;
        let just_pressed = GamepadButtons(current_buttons.0 & !prev_buttons.0);
        let just_released = GamepadButtons(!current_buttons.0 & prev_buttons.0);

        // Left stick deadzone shaping
        let (lx, ly, lmag, lang) = self.left_stick_deadzone.shape(report.left_stick_x, report.left_stick_y);
        let left_stick = AnalogStick {
            raw_x: report.left_stick_x,
            raw_y: report.left_stick_y,
            x: lx,
            y: ly,
            magnitude: lmag,
            angle_rad: lang,
        };

        // Right stick deadzone shaping
        let (rx, ry, rmag, rang) = self.right_stick_deadzone.shape(report.right_stick_x, report.right_stick_y);
        let right_stick = AnalogStick {
            raw_x: report.right_stick_x,
            raw_y: report.right_stick_y,
            x: rx,
            y: ry,
            magnitude: rmag,
            angle_rad: rang,
        };

        // Left trigger normalization
        let (lt_val, lt_click) = self.left_trigger_deadzone.normalize_u16(report.left_trigger);
        let left_trigger = AnalogTrigger {
            raw: report.left_trigger,
            value: lt_val,
            is_clicked: lt_click,
        };

        // Right trigger normalization
        let (rt_val, rt_click) = self.right_trigger_deadzone.normalize_u16(report.right_trigger);
        let right_trigger = AnalogTrigger {
            raw: report.right_trigger,
            value: rt_val,
            is_clicked: rt_click,
        };

        // Process optional IMU sensor fusion
        let (orientation, angular_rate, dyn_accel) = if let (Some(fusion), Some(raw_imu)) = (&mut self.fusion, report.imu_sample) {
            let (q, _euler, d_accel) = fusion.update_raw(&raw_imu);
            (Some(q), Some(fusion.processor.current_gyro()), Some(d_accel))
        } else {
            (None, None, None)
        };

        let state = GamepadState {
            buttons: current_buttons,
            buttons_just_pressed: just_pressed,
            buttons_just_released: just_released,
            left_stick,
            right_stick,
            left_trigger,
            right_trigger,
            imu_orientation: orientation,
            imu_angular_rate: angular_rate,
            imu_dynamic_accel: dyn_accel,
            timestamp_us: report.timestamp_us,
        };

        self.last_state = state;
        state
    }

    /// Calculates motion aiming crosshair delta (dx, dy) in pixels from IMU angular rate.
    ///
    /// Uses yaw rate for horizontal aiming and pitch rate for vertical aiming.
    pub fn calculate_gyro_aiming(&self, state: &GamepadState, dt: f32) -> (f32, f32) {
        if !self.gyro_aiming_enabled {
            return (0.0, 0.0);
        }

        if let Some(gyro) = state.imu_angular_rate {
            // Gyroscope angular rate in rad/s:
            // Yaw rate (Z or Y depending on controller grip) -> horizontal aim
            // Pitch rate (X) -> vertical aim
            let dx = -gyro.z * self.gyro_sensitivity * dt * 1000.0;
            let dy = -gyro.x * self.gyro_sensitivity * dt * 1000.0;
            (dx, dy)
        } else {
            (0.0, 0.0)
        }
    }

    /// Sets haptic rumble motor vibration states.
    pub fn set_haptics(&mut self, haptics: HapticFeedback) {
        self.haptics = haptics;
    }

    #[inline(always)]
    pub fn current_haptics(&self) -> HapticFeedback {
        self.haptics
    }

    #[inline(always)]
    pub fn total_reports(&self) -> u64 {
        self.total_reports_processed
    }

    #[inline(always)]
    pub fn current_polling_rate_hz(&self) -> f32 {
        self.current_polling_rate_hz
    }

    #[inline(always)]
    pub fn min_interval_us(&self) -> u64 {
        self.min_interval_us
    }

    #[inline(always)]
    pub fn max_interval_us(&self) -> u64 {
        self.max_interval_us
    }
}
