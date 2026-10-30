//! Dual Capacitive Thumb Touchpad Controller & Ballistic Acceleration Governor.
//!
//! Provides ergonomic touchpad control for handheld gaming PCs:
//! - Dual touchpad management (Left & Right thumb touchpads)
//! - Non-linear ballistic velocity curves (1:1 precision at micro-speeds, scaling up to 3.5x for flick turns)
//! - Capacitive touch contact and normalized displacement tracking
//! - Dual-stage click emulation (distinguishing capacitive hover from physical pressure click actuation)
//! - Tactile haptic tick generation for rolling surface emulation

use crate::graphics::sqrt_f32;

// -----------------------------------------------------------------------------
// Constants and Types
// -----------------------------------------------------------------------------

/// Identifies which physical thumb touchpad is being addressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchpadId {
    Left = 0,
    Right = 1,
}

/// Physical geometry of the thumb touchpad surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchpadShape {
    /// Square surface with rounded corners (e.g. Steam Deck).
    Square,
    /// Circular surface with radial boundary clamping.
    Circular,
}

/// Operating stage of the dual-stage touchpad contact sensor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchpadClickStage {
    /// Thumb is hovering or released; no surface contact.
    Hover,
    /// Capacitive contact active (thumb resting or gliding on surface), below physical click pressure.
    CapacitiveTouch,
    /// Surface pressure exceeded mechanical actuation threshold (virtual physical switch pressed).
    PressureClick,
}

/// Discrete transition edge between dual-stage contact and pressure states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickTransition {
    /// No change in contact or click state.
    None,
    /// Capacitive touch contact engaged (Hover -> CapacitiveTouch).
    TouchpadContactDown,
    /// Capacitive touch contact released (CapacitiveTouch -> Hover).
    TouchpadContactUp,
    /// Physical pressure click actuated (CapacitiveTouch -> PressureClick).
    PressureClickDown,
    /// Physical pressure click released (PressureClick -> CapacitiveTouch).
    PressureClickUp,
}

/// Configuration thresholds for dual-stage capacitive touch & pressure click detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DualStageClickConfig {
    /// Raw ADC pressure threshold to detect capacitive touch presence (0..4095).
    pub touch_threshold: u16,
    /// Raw ADC pressure threshold required to actuate physical pressure click (0..4095).
    pub click_press_threshold: u16,
    /// Raw ADC pressure threshold below which physical click releases (hysteresis gap).
    pub click_release_threshold: u16,
}

impl DualStageClickConfig {
    pub const fn default_handheld() -> Self {
        Self {
            touch_threshold: 50,
            click_press_threshold: 2048,   // ~50% maximum surface pressure
            click_release_threshold: 1600, // ~39% hysteresis release to prevent flutter
        }
    }
}

/// Raw sensor sample from the touchpad capacitive controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawTouchpadSample {
    /// Touchpad identifier (Left or Right).
    pub pad_id: TouchpadId,
    /// Normalized horizontal position (-32768 to 32767).
    pub raw_x: i16,
    /// Normalized vertical position (-32768 to 32767).
    pub raw_y: i16,
    /// Surface pressure measurement (0 to 4095).
    pub pressure: u16,
    /// Capacitive touch contact detected (true if thumb is touching surface).
    pub touch_detected: bool,
    /// Sample capture timestamp in microseconds.
    pub timestamp_us: u64,
}

/// Output motion vector produced by the ballistic acceleration governor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchpadMotion {
    /// Source touchpad identifier.
    pub pad_id: TouchpadId,
    /// Ballistically scaled horizontal output delta (pixels / game units).
    pub dx: f32,
    /// Ballistically scaled vertical output delta (pixels / game units).
    pub dy: f32,
    /// Unscaled instantaneous horizontal speed (units/sec).
    pub velocity_x: f32,
    /// Unscaled instantaneous vertical speed (units/sec).
    pub velocity_y: f32,
    /// Overall movement speed magnitude.
    pub speed: f32,
    /// Ballistic multiplier applied [1.0 .. max_multiplier].
    pub ballistic_multiplier: f32,
    /// Whether thumb is actively touching the pad.
    pub touch_active: bool,
    /// Measured pressure level [0.0 .. 1.0].
    pub pressure_normalized: f32,
    /// Current dual-stage contact and pressure stage.
    pub click_stage: TouchpadClickStage,
    /// Edge transition in click or contact status.
    pub click_transition: ClickTransition,
    /// Number of discrete tactile haptic ticks triggered during this sample.
    pub haptic_ticks: u32,
}

impl TouchpadMotion {
    pub const fn zero(pad_id: TouchpadId) -> Self {
        Self {
            pad_id,
            dx: 0.0,
            dy: 0.0,
            velocity_x: 0.0,
            velocity_y: 0.0,
            speed: 0.0,
            ballistic_multiplier: 1.0,
            touch_active: false,
            pressure_normalized: 0.0,
            click_stage: TouchpadClickStage::Hover,
            click_transition: ClickTransition::None,
            haptic_ticks: 0,
        }
    }
}

// -----------------------------------------------------------------------------
// Ballistic Acceleration Configuration
// -----------------------------------------------------------------------------

/// Configuration parameters for non-linear touchpad ballistic acceleration curves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BallisticConfig {
    /// Velocity threshold below which 1:1 precision is preserved (units / second).
    pub low_speed_threshold: f32,
    /// Velocity threshold above which maximum flick acceleration is achieved (units / second).
    pub high_speed_threshold: f32,
    /// Maximum acceleration multiplier for high-speed flick turns (e.g. 3.5x).
    pub max_multiplier: f32,
    /// Base sensitivity gain multiplier.
    pub sensitivity: f32,
    /// Shape geometry of the touchpad.
    pub shape: TouchpadShape,
}

impl BallisticConfig {
    pub const fn default_precision_aiming() -> Self {
        Self {
            low_speed_threshold: 15.0,
            high_speed_threshold: 120.0,
            max_multiplier: 3.5,
            sensitivity: 1.0,
            shape: TouchpadShape::Square,
        }
    }

    pub const fn default_circular_trackball() -> Self {
        Self {
            low_speed_threshold: 15.0,
            high_speed_threshold: 120.0,
            max_multiplier: 3.5,
            sensitivity: 1.0,
            shape: TouchpadShape::Circular,
        }
    }
}

// -----------------------------------------------------------------------------
// Single Touchpad State Machine
// -----------------------------------------------------------------------------

/// State tracking for a single capacitive touchpad.
#[derive(Debug, Clone)]
pub struct SingleTouchpad {
    /// Pad identifier.
    pub pad_id: TouchpadId,
    /// Ballistic configuration.
    pub config: BallisticConfig,
    /// Dual-stage click threshold configuration.
    pub click_config: DualStageClickConfig,
    /// Current dual-stage click state.
    pub click_stage: TouchpadClickStage,
    /// Displacement interval per haptic tick (units).
    pub haptic_tick_interval: f32,
    /// Accumulated distance towards next haptic tick.
    pub haptic_tick_accumulator: f32,
    /// Previous raw X position.
    prev_x: f32,
    /// Previous raw Y position.
    prev_y: f32,
    /// Previous timestamp in microseconds.
    prev_timestamp_us: u64,
    /// True if touch was active in the previous sample.
    was_touching: bool,
    /// Filtered velocity X.
    filtered_vx: f32,
    /// Filtered velocity Y.
    filtered_vy: f32,
}

impl SingleTouchpad {
    pub const fn new(pad_id: TouchpadId, config: BallisticConfig) -> Self {
        Self {
            pad_id,
            config,
            click_config: DualStageClickConfig::default_handheld(),
            click_stage: TouchpadClickStage::Hover,
            haptic_tick_interval: 2.0,
            haptic_tick_accumulator: 0.0,
            prev_x: 0.0,
            prev_y: 0.0,
            prev_timestamp_us: 0,
            was_touching: false,
            filtered_vx: 0.0,
            filtered_vy: 0.0,
        }
    }

    /// Evaluates ballistic velocity multiplier for a given thumb speed.
    ///
    /// - For speed <= low_speed_threshold: multiplier = 1.0 (pure 1:1 precision aiming)
    /// - For speed between low and high threshold: smooth progressive acceleration curve
    /// - For speed >= high_speed_threshold: saturated at max_multiplier (3.5x for flick turns)
    pub fn compute_ballistic_multiplier(&self, speed: f32) -> f32 {
        if speed <= self.config.low_speed_threshold {
            1.0
        } else if speed >= self.config.high_speed_threshold {
            self.config.max_multiplier
        } else {
            let span = self.config.high_speed_threshold - self.config.low_speed_threshold;
            let progress = (speed - self.config.low_speed_threshold) / span;
            // Quadratic easing curve for smooth acceleration transition: f(p) = p^2
            let eased = progress * progress;
            1.0 + eased * (self.config.max_multiplier - 1.0)
        }
    }

    /// Updates the dual-stage click state machine and returns any transition edge.
    fn update_click_stage(&mut self, touch_detected: bool, pressure: u16) -> ClickTransition {
        let has_contact = touch_detected && (pressure >= self.click_config.touch_threshold);

        match self.click_stage {
            TouchpadClickStage::Hover => {
                if has_contact {
                    if pressure >= self.click_config.click_press_threshold {
                        self.click_stage = TouchpadClickStage::PressureClick;
                        ClickTransition::PressureClickDown
                    } else {
                        self.click_stage = TouchpadClickStage::CapacitiveTouch;
                        ClickTransition::TouchpadContactDown
                    }
                } else {
                    ClickTransition::None
                }
            }
            TouchpadClickStage::CapacitiveTouch => {
                if !has_contact {
                    self.click_stage = TouchpadClickStage::Hover;
                    ClickTransition::TouchpadContactUp
                } else if pressure >= self.click_config.click_press_threshold {
                    self.click_stage = TouchpadClickStage::PressureClick;
                    ClickTransition::PressureClickDown
                } else {
                    ClickTransition::None
                }
            }
            TouchpadClickStage::PressureClick => {
                if !has_contact {
                    self.click_stage = TouchpadClickStage::Hover;
                    ClickTransition::TouchpadContactUp
                } else if pressure < self.click_config.click_release_threshold {
                    self.click_stage = TouchpadClickStage::CapacitiveTouch;
                    ClickTransition::PressureClickUp
                } else {
                    ClickTransition::None
                }
            }
        }
    }

    /// Ingests a raw touchpad sample and computes ballistic motion deltas, dual-stage clicks, and haptic ticks.
    pub fn process_sample(&mut self, sample: RawTouchpadSample) -> TouchpadMotion {
        let pressure_norm = (sample.pressure as f32) / 4095.0;
        let transition = self.update_click_stage(sample.touch_detected, sample.pressure);

        // Convert raw -32768..32767 to normalized unit range [-100.0 .. 100.0]
        let current_x = (sample.raw_x as f32) / 327.67;
        let current_y = (sample.raw_y as f32) / 327.67;

        if !sample.touch_detected {
            self.was_touching = false;
            self.filtered_vx = 0.0;
            self.filtered_vy = 0.0;
            self.haptic_tick_accumulator = 0.0;
            return TouchpadMotion {
                pad_id: self.pad_id,
                dx: 0.0,
                dy: 0.0,
                velocity_x: 0.0,
                velocity_y: 0.0,
                speed: 0.0,
                ballistic_multiplier: 1.0,
                touch_active: false,
                pressure_normalized: pressure_norm,
                click_stage: self.click_stage,
                click_transition: transition,
                haptic_ticks: 0,
            };
        }

        if !self.was_touching {
            // First contact down: initialize position history without emitting displacement
            self.prev_x = current_x;
            self.prev_y = current_y;
            self.prev_timestamp_us = sample.timestamp_us;
            self.was_touching = true;
            self.filtered_vx = 0.0;
            self.filtered_vy = 0.0;
            self.haptic_tick_accumulator = 0.0;

            return TouchpadMotion {
                pad_id: self.pad_id,
                dx: 0.0,
                dy: 0.0,
                velocity_x: 0.0,
                velocity_y: 0.0,
                speed: 0.0,
                ballistic_multiplier: 1.0,
                touch_active: true,
                pressure_normalized: pressure_norm,
                click_stage: self.click_stage,
                click_transition: transition,
                haptic_ticks: 0,
            };
        }

        let dt_us = sample.timestamp_us.saturating_sub(self.prev_timestamp_us);
        let dt_sec = (dt_us as f32) / 1_000_000.0;

        let raw_dx = current_x - self.prev_x;
        let raw_dy = current_y - self.prev_y;
        self.prev_x = current_x;
        self.prev_y = current_y;
        self.prev_timestamp_us = sample.timestamp_us;

        // Calculate travel distance and haptic ticks
        let step_dist = sqrt_f32(raw_dx * raw_dx + raw_dy * raw_dy);
        self.haptic_tick_accumulator += step_dist;
        let mut ticks = 0u32;
        if self.haptic_tick_interval > 0.0 && self.haptic_tick_accumulator >= self.haptic_tick_interval {
            ticks = (self.haptic_tick_accumulator / self.haptic_tick_interval) as u32;
            self.haptic_tick_accumulator -= (ticks as f32) * self.haptic_tick_interval;
        }

        if dt_sec < 0.0005 {
            return TouchpadMotion {
                pad_id: self.pad_id,
                dx: 0.0,
                dy: 0.0,
                velocity_x: self.filtered_vx,
                velocity_y: self.filtered_vy,
                speed: sqrt_f32(self.filtered_vx * self.filtered_vx + self.filtered_vy * self.filtered_vy),
                ballistic_multiplier: 1.0,
                touch_active: true,
                pressure_normalized: pressure_norm,
                click_stage: self.click_stage,
                click_transition: transition,
                haptic_ticks: ticks,
            };
        }

        // Instantaneous speed
        let inst_vx = raw_dx / dt_sec;
        let inst_vy = raw_dy / dt_sec;

        // Smooth velocity with alpha filter (alpha = 0.60)
        self.filtered_vx = self.filtered_vx * 0.40 + inst_vx * 0.60;
        self.filtered_vy = self.filtered_vy * 0.40 + inst_vy * 0.60;
        let speed = sqrt_f32(self.filtered_vx * self.filtered_vx + self.filtered_vy * self.filtered_vy);

        // Compute dynamic ballistic acceleration multiplier
        let multiplier = self.compute_ballistic_multiplier(speed);
        let final_dx = raw_dx * multiplier * self.config.sensitivity;
        let final_dy = raw_dy * multiplier * self.config.sensitivity;

        TouchpadMotion {
            pad_id: self.pad_id,
            dx: final_dx,
            dy: final_dy,
            velocity_x: self.filtered_vx,
            velocity_y: self.filtered_vy,
            speed,
            ballistic_multiplier: multiplier,
            touch_active: true,
            pressure_normalized: pressure_norm,
            click_stage: self.click_stage,
            click_transition: transition,
            haptic_ticks: ticks,
        }
    }

    /// Resets the touchpad tracking history.
    pub fn reset(&mut self) {
        self.prev_x = 0.0;
        self.prev_y = 0.0;
        self.prev_timestamp_us = 0;
        self.was_touching = false;
        self.filtered_vx = 0.0;
        self.filtered_vy = 0.0;
        self.click_stage = TouchpadClickStage::Hover;
        self.haptic_tick_accumulator = 0.0;
    }
}

// -----------------------------------------------------------------------------
// Dual Touchpad Controller
// -----------------------------------------------------------------------------

/// Controller managing both Left and Right handheld capacitive touchpads.
pub struct DualTouchpadController {
    /// Left and right touchpad handlers.
    pub pads: [SingleTouchpad; 2],
    /// Total samples processed.
    pub total_samples: u64,
}

impl DualTouchpadController {
    /// Initializes dual touchpad controller with default handheld precision configurations.
    pub fn new() -> Self {
        Self {
            pads: [
                SingleTouchpad::new(TouchpadId::Left, BallisticConfig::default_precision_aiming()),
                SingleTouchpad::new(TouchpadId::Right, BallisticConfig::default_precision_aiming()),
            ],
            total_samples: 0,
        }
    }

    /// Processes an incoming raw touchpad sample from either pad.
    pub fn process_sample(&mut self, sample: RawTouchpadSample) -> TouchpadMotion {
        self.total_samples += 1;
        let idx = sample.pad_id as usize;
        self.pads[idx].process_sample(sample)
    }

    /// Returns a reference to a specific touchpad.
    pub fn get_pad(&self, pad_id: TouchpadId) -> &SingleTouchpad {
        &self.pads[pad_id as usize]
    }

    /// Returns a mutable reference to a specific touchpad.
    pub fn get_pad_mut(&mut self, pad_id: TouchpadId) -> &mut SingleTouchpad {
        &mut self.pads[pad_id as usize]
    }
}

// -----------------------------------------------------------------------------
// Unit Tests & Verification
// -----------------------------------------------------------------------------

/// Unit test verifying dual capacitive thumb touchpad driver and ballistic velocity curve.
pub fn verify_dual_touchpad_and_ballistic_curve() -> bool {
    let mut controller = DualTouchpadController::new();

    // 1. Initial State: No touch active
    let s_idle = RawTouchpadSample {
        pad_id: TouchpadId::Right,
        raw_x: 0,
        raw_y: 0,
        pressure: 0,
        touch_detected: false,
        timestamp_us: 10_000,
    };
    let m_idle = controller.process_sample(s_idle);
    if m_idle.touch_active || m_idle.dx != 0.0 || m_idle.dy != 0.0 {
        return false;
    }

    // 2. Initial Touch Down on Right Pad: no displacement on first sample
    let s_down = RawTouchpadSample {
        pad_id: TouchpadId::Right,
        raw_x: 1000,
        raw_y: 1000,
        pressure: 500,
        touch_detected: true,
        timestamp_us: 20_000,
    };
    let m_down = controller.process_sample(s_down);
    if !m_down.touch_active || m_down.dx != 0.0 || m_down.dy != 0.0 {
        return false;
    }

    // 3. Micro-Movement Precision Aiming Test (v <= 15.0 units/sec)
    // Low speed threshold = 15.0 -> multiplier must be exactly 1.0 (no acceleration overshoot)
    let s_slow = RawTouchpadSample {
        pad_id: TouchpadId::Right,
        raw_x: 1010, // delta = +10 raw units (~0.0305 normalized units)
        raw_y: 1000,
        pressure: 500,
        touch_detected: true,
        timestamp_us: 30_000, // dt = 10ms (10,000 us) -> speed ~ 3.0 units/s <= 15.0
    };
    let m_slow = controller.process_sample(s_slow);
    if (m_slow.ballistic_multiplier - 1.0).abs() > 0.001 {
        return false;
    }
    if m_slow.dx <= 0.0 || m_slow.dy != 0.0 {
        return false;
    }

    // 4. High-Speed 180° Flick Turn Acceleration Test (v >= 120.0 units/sec)
    // Move 1500 raw units in 10ms -> 4.577 units / 0.01s = 457.7 units/s > 120.0
    // Multiplier must saturate at max_multiplier (3.5x)
    let mut t = 30_000u64;
    let mut raw_x = 1010i16;
    let mut m_flick = TouchpadMotion::zero(TouchpadId::Right);
    for _ in 0..5 {
        t += 10_000;
        raw_x += 1500;
        let s_fast = RawTouchpadSample {
            pad_id: TouchpadId::Right,
            raw_x,
            raw_y: 1000,
            pressure: 600,
            touch_detected: true,
            timestamp_us: t,
        };
        m_flick = controller.process_sample(s_fast);
    }
    if (m_flick.ballistic_multiplier - 3.5).abs() > 0.05 {
        return false;
    }
    if m_flick.speed < 120.0 {
        return false;
    }

    // 5. Left Touchpad Independence Check
    // Left pad should still be untouched and idle
    let left_pad = controller.get_pad(TouchpadId::Left);
    if left_pad.was_touching || left_pad.filtered_vx != 0.0 {
        return false;
    }

    true
}

/// Unit test verifying dual-stage pressure click threshold, hysteresis, and haptic click tick generation.
pub fn verify_dual_stage_click_and_haptic_ticks() -> bool {
    let mut pad = SingleTouchpad::new(TouchpadId::Right, BallisticConfig::default_precision_aiming());
    pad.haptic_tick_interval = 2.0;

    // 1. Initial Hover state
    if pad.click_stage != TouchpadClickStage::Hover {
        return false;
    }

    // 2. Light touch (pressure = 400 > touch_threshold: 50, but < click_press: 2048)
    // Must transition to CapacitiveTouch with TouchpadContactDown
    let s_touch = RawTouchpadSample {
        pad_id: TouchpadId::Right,
        raw_x: 0,
        raw_y: 0,
        pressure: 400,
        touch_detected: true,
        timestamp_us: 10_000,
    };
    let m_touch = pad.process_sample(s_touch);
    if m_touch.click_stage != TouchpadClickStage::CapacitiveTouch {
        return false;
    }
    if m_touch.click_transition != ClickTransition::TouchpadContactDown {
        return false;
    }

    // 3. Movement generating haptic click ticks:
    // Move 10.0 normalized units (raw_x = 3277 = 10.0 units) over 10ms with interval = 2.0 units
    // Should trigger exactly 5 ticks (10.0 / 2.0)
    let s_move = RawTouchpadSample {
        pad_id: TouchpadId::Right,
        raw_x: 3277,
        raw_y: 0,
        pressure: 400,
        touch_detected: true,
        timestamp_us: 20_000,
    };
    let m_move = pad.process_sample(s_move);
    if m_move.haptic_ticks != 5 {
        return false;
    }
    if m_move.click_stage != TouchpadClickStage::CapacitiveTouch {
        return false;
    }

    // 4. Heavy thumb squeeze (pressure = 2500 >= 2048 click_press_threshold)
    // Must actuate physical click with PressureClickDown
    let s_press = RawTouchpadSample {
        pad_id: TouchpadId::Right,
        raw_x: 3277,
        raw_y: 0,
        pressure: 2500,
        touch_detected: true,
        timestamp_us: 30_000,
    };
    let m_press = pad.process_sample(s_press);
    if m_press.click_stage != TouchpadClickStage::PressureClick {
        return false;
    }
    if m_press.click_transition != ClickTransition::PressureClickDown {
        return false;
    }

    // 5. Hysteresis test: pressure drops to 1800 (below 2048, but above release threshold 1600)
    // Must remain in PressureClick with NO release transition
    let s_hyst = RawTouchpadSample {
        pad_id: TouchpadId::Right,
        raw_x: 3277,
        raw_y: 0,
        pressure: 1800,
        touch_detected: true,
        timestamp_us: 40_000,
    };
    let m_hyst = pad.process_sample(s_hyst);
    if m_hyst.click_stage != TouchpadClickStage::PressureClick {
        return false;
    }
    if m_hyst.click_transition != ClickTransition::None {
        return false;
    }

    // 6. Release click: pressure drops to 1200 (< 1600 release threshold)
    // Must transition to CapacitiveTouch with PressureClickUp
    let s_rel_click = RawTouchpadSample {
        pad_id: TouchpadId::Right,
        raw_x: 3277,
        raw_y: 0,
        pressure: 1200,
        touch_detected: true,
        timestamp_us: 50_000,
    };
    let m_rel_click = pad.process_sample(s_rel_click);
    if m_rel_click.click_stage != TouchpadClickStage::CapacitiveTouch {
        return false;
    }
    if m_rel_click.click_transition != ClickTransition::PressureClickUp {
        return false;
    }

    // 7. Lift thumb completely off touchpad surface
    // Must transition to Hover with TouchpadContactUp
    let s_lift = RawTouchpadSample {
        pad_id: TouchpadId::Right,
        raw_x: 3277,
        raw_y: 0,
        pressure: 0,
        touch_detected: false,
        timestamp_us: 60_000,
    };
    let m_lift = pad.process_sample(s_lift);
    if m_lift.click_stage != TouchpadClickStage::Hover {
        return false;
    }
    if m_lift.click_transition != ClickTransition::TouchpadContactUp {
        return false;
    }

    true
}

// -----------------------------------------------------------------------------
// High-Precision Multi-Finger Capacitive Gesture Engine (TASK-KNT-128)
// -----------------------------------------------------------------------------

/// Maximum concurrent contact points tracked across the capacitive matrix.
pub const MAX_TOUCHPAD_CONTACTS: usize = 5;

/// Operational lifecycle state of an individual capacitive contact point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchpadContactState {
    /// Contact slot inactive or unassigned.
    Inactive,
    /// Finger initial contact down on surface.
    TouchDown,
    /// Finger stationary and pressing on surface.
    Holding,
    /// Finger gliding across surface with valid velocity vector.
    Moving,
    /// Finger breaking surface contact.
    LiftUp,
}

/// Detailed capacitive finger contact descriptor with ellipse geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchpadContact {
    /// Unique hardware tracking ID for this contact.
    pub contact_id: u8,
    /// Current contact lifecycle state.
    pub state: TouchpadContactState,
    /// Sub-pixel horizontal coordinate (0.0 .. 1920.0).
    pub x: f32,
    /// Sub-pixel vertical coordinate (0.0 .. 1080.0).
    pub y: f32,
    /// Surface pressure measurement (0..4095).
    pub pressure_z: u16,
    /// Major axis radius of contact ellipse in sensor pitch units.
    pub major_axis: u16,
    /// Minor axis radius of contact ellipse in sensor pitch units.
    pub minor_axis: u16,
    /// Contact orientation angle in degrees (-90 .. 90).
    pub orientation_deg: i16,
    /// Timestamp of contact observation in microseconds.
    pub timestamp_us: u64,
}

impl TouchpadContact {
    /// Creates an empty inactive contact slot.
    pub const fn empty(contact_id: u8) -> Self {
        Self {
            contact_id,
            state: TouchpadContactState::Inactive,
            x: 0.0,
            y: 0.0,
            pressure_z: 0,
            major_axis: 0,
            minor_axis: 0,
            orientation_deg: 0,
            timestamp_us: 0,
        }
    }
}

/// Multi-contact raw frame decoded from capacitive controller I2C/HID report.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchpadFrame {
    /// Number of active physical fingers detected on the pad (0..MAX_TOUCHPAD_CONTACTS).
    pub contact_count: u8,
    /// Array of tracked contact descriptors.
    pub contacts: [TouchpadContact; MAX_TOUCHPAD_CONTACTS],
    /// Physical dome switch / pressure click active.
    pub physical_button_down: bool,
    /// Frame capture timestamp in microseconds.
    pub timestamp_us: u64,
}

impl TouchpadFrame {
    /// Creates a blank frame with 0 contacts.
    pub const fn empty(timestamp_us: u64) -> Self {
        Self {
            contact_count: 0,
            contacts: [
                TouchpadContact::empty(0),
                TouchpadContact::empty(1),
                TouchpadContact::empty(2),
                TouchpadContact::empty(3),
                TouchpadContact::empty(4),
            ],
            physical_button_down: false,
            timestamp_us,
        }
    }

    /// Decodes a 24-byte multi-touch report packet into a structured TouchpadFrame.
    pub fn decode_packet(bytes: &[u8], timestamp_us: u64) -> Result<Self, &'static str> {
        if bytes.len() < 4 {
            return Err("Touchpad packet too short");
        }

        let report_id = bytes[0];
        if report_id != 0x3F && report_id != 0x12 {
            return Err("Invalid touchpad report ID");
        }

        let count = (bytes[1] as usize).min(MAX_TOUCHPAD_CONTACTS);
        let button = (bytes[2] & 0x01) != 0;

        let mut frame = Self::empty(timestamp_us);
        frame.contact_count = count as u8;
        frame.physical_button_down = button;

        // Each contact payload is 4 bytes: [ID_state, X_lo, X_hi_Y_lo, Y_hi]
        let mut offset = 3;
        for i in 0..count {
            if offset + 4 > bytes.len() {
                break;
            }
            let b_id_state = bytes[offset];
            let id = b_id_state & 0x0F;
            let is_down = (b_id_state & 0x10) != 0;

            let x_raw = (bytes[offset + 1] as u16) | (((bytes[offset + 2] & 0x0F) as u16) << 8);
            let y_raw = ((bytes[offset + 2] >> 4) as u16) | ((bytes[offset + 3] as u16) << 4);

            frame.contacts[i] = TouchpadContact {
                contact_id: id,
                state: if is_down { TouchpadContactState::Moving } else { TouchpadContactState::TouchDown },
                x: x_raw as f32,
                y: y_raw as f32,
                pressure_z: 1024,
                major_axis: 12,
                minor_axis: 8,
                orientation_deg: 0,
                timestamp_us,
            };
            offset += 4;
        }

        Ok(frame)
    }
}

/// High-level multi-finger gesture types recognized by the gesture engine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TouchpadGestureType {
    /// No gesture active.
    None,
    /// 1-Finger discrete tap at coordinate.
    SingleTap { x: f32, y: f32 },
    /// 1-Finger cursor displacement drag.
    SingleDrag { dx: f32, dy: f32 },
    /// 2-Finger scroll with vertical and horizontal displacement.
    TwoFingerScroll { delta_x: f32, delta_y: f32, is_inertial: bool },
    /// 2-Finger pinch-to-zoom scaling factor delta.
    TwoFingerPinchZoom { scale_delta: f32, center_x: f32, center_y: f32 },
    /// 3-Finger fast horizontal swipe (e.g. desktop/workspace switch).
    ThreeFingerSwipeHorizontal { swipe_left: bool },
    /// 3-Finger fast vertical swipe (e.g. window overview / task view).
    ThreeFingerSwipeVertical { swipe_up: bool },
}

/// Gesture recognition state lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchpadGestureState {
    Possible,
    Began,
    Changed,
    Ended,
    Cancelled,
}

/// High-level gesture event delivered to window manager and input compositor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchpadGestureEvent {
    /// Recognized gesture type with motion payload.
    pub gesture_type: TouchpadGestureType,
    /// Gesture state lifecycle phase.
    pub state: TouchpadGestureState,
    /// Number of active fingers driving this gesture.
    pub finger_count: u8,
    /// Event occurrence timestamp in microseconds.
    pub timestamp_us: u64,
}

/// Verification helper for Commit 1 contact structures, decoders, and gesture descriptors.
pub fn verify_touchpad_contact_structures_and_decoders() -> bool {
    // 1. Verify Empty Frame Initialization
    let frame = TouchpadFrame::empty(10_000);
    if frame.contact_count != 0 || frame.physical_button_down || frame.timestamp_us != 10_000 {
        return false;
    }

    // 2. Verify Multi-Touch Raw Packet Decoding
    // Report ID 0x3F, 2 contacts, Button Down (0x01)
    // Contact 0: ID 0, state down (0x10), X = 800 (0x320: lo=0x20, hi=0x03), Y = 600 (0x258: lo=0x05, hi=0x25)
    // Contact 1: ID 1, state down (0x11), X = 1200 (0x4B0: lo=0xB0, hi=0x04), Y = 900 (0x384: lo=0x08, hi=0x38)
    let raw_packet: [u8; 11] = [
        0x3F, 0x02, 0x01,
        0x10, 0x20, 0x53, 0x25,
        0x11, 0xB0, 0x84, 0x38,
    ];

    let decoded = match TouchpadFrame::decode_packet(&raw_packet, 20_000) {
        Ok(f) => f,
        Err(_) => return false,
    };

    if decoded.contact_count != 2 || !decoded.physical_button_down {
        return false;
    }
    if decoded.contacts[0].contact_id != 0 || decoded.contacts[0].x != 800.0 || decoded.contacts[0].y != 600.0 {
        return false;
    }
    if decoded.contacts[1].contact_id != 1 || decoded.contacts[1].x != 1200.0 || decoded.contacts[1].y != 900.0 {
        return false;
    }

    // 3. Verify Gesture Event Structure
    let event = TouchpadGestureEvent {
        gesture_type: TouchpadGestureType::TwoFingerScroll { delta_x: 0.0, delta_y: -15.5, is_inertial: false },
        state: TouchpadGestureState::Changed,
        finger_count: 2,
        timestamp_us: 20_500,
    };
    if event.finger_count != 2 || event.state != TouchpadGestureState::Changed {
        return false;
    }

    true
}
/// Sub-pixel deadzone threshold in pixels to prevent static finger micro-tremor cursor drift.
pub const SUBPIXEL_DEADZONE_PX: f32 = 1.2;

/// Filtered contact state tracking position, smoothed velocity, and trajectory history.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FilteredContact {
    /// Associated contact tracking identifier.
    pub contact_id: u8,
    /// Unfiltered raw position.
    pub raw_x: f32,
    pub raw_y: f32,
    /// Adaptive smoothed position after jitter filtering.
    pub filtered_x: f32,
    pub filtered_y: f32,
    /// Delta displacement from previous frame.
    pub dx: f32,
    pub dy: f32,
    /// Estimated velocity in pixels per millisecond.
    pub vx: f32,
    pub vy: f32,
    /// Touch-down origin coordinates.
    pub start_x: f32,
    pub start_y: f32,
    /// Total Euclidean distance traversed since touch-down.
    pub total_distance: f32,
    /// Active contact flag.
    pub active: bool,
}

impl FilteredContact {
    /// Creates an empty inactive filtered contact.
    pub const fn empty(contact_id: u8) -> Self {
        Self {
            contact_id,
            raw_x: 0.0,
            raw_y: 0.0,
            filtered_x: 0.0,
            filtered_y: 0.0,
            dx: 0.0,
            dy: 0.0,
            vx: 0.0,
            vy: 0.0,
            start_x: 0.0,
            start_y: 0.0,
            total_distance: 0.0,
            active: false,
        }
    }
}

/// Adaptive jitter filter and multi-contact tracking engine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchpadTrackingEngine {
    /// Array of filtered contact states for up to 5 concurrent contacts.
    pub contacts: [FilteredContact; MAX_TOUCHPAD_CONTACTS],
    /// Previous frame timestamp in microseconds.
    pub prev_timestamp_us: u64,
}

impl TouchpadTrackingEngine {
    /// Creates a new tracking engine instance.
    pub const fn new() -> Self {
        Self {
            contacts: [
                FilteredContact::empty(0),
                FilteredContact::empty(1),
                FilteredContact::empty(2),
                FilteredContact::empty(3),
                FilteredContact::empty(4),
            ],
            prev_timestamp_us: 0,
        }
    }

    /// Ingests a raw TouchpadFrame, applies adaptive exponential jitter filtering and deadzone governor.
    pub fn process_frame(&mut self, frame: &TouchpadFrame) {
        let dt_ms = if self.prev_timestamp_us > 0 && frame.timestamp_us > self.prev_timestamp_us {
            ((frame.timestamp_us - self.prev_timestamp_us) as f32 / 1000.0).max(1.0)
        } else {
            10.0 // Default 100Hz frame interval (10ms)
        };
        self.prev_timestamp_us = frame.timestamp_us;

        // Mark all slots as inactive initially for frame reconciliation
        let mut active_in_frame = [false; MAX_TOUCHPAD_CONTACTS];

        for i in 0..(frame.contact_count as usize).min(MAX_TOUCHPAD_CONTACTS) {
            let raw_c = &frame.contacts[i];
            let id = (raw_c.contact_id as usize) % MAX_TOUCHPAD_CONTACTS;
            active_in_frame[id] = true;

            let fc = &mut self.contacts[id];

            if !fc.active {
                // New touch contact established
                fc.contact_id = raw_c.contact_id;
                fc.raw_x = raw_c.x;
                fc.raw_y = raw_c.y;
                fc.filtered_x = raw_c.x;
                fc.filtered_y = raw_c.y;
                fc.start_x = raw_c.x;
                fc.start_y = raw_c.y;
                fc.dx = 0.0;
                fc.dy = 0.0;
                fc.vx = 0.0;
                fc.vy = 0.0;
                fc.total_distance = 0.0;
                fc.active = true;
            } else {
                // Existing contact update: compute raw displacement
                let raw_dx = raw_c.x - fc.raw_x;
                let raw_dy = raw_c.y - fc.raw_y;
                fc.raw_x = raw_c.x;
                fc.raw_y = raw_c.y;

                let raw_dist = sqrt_f32(raw_dx * raw_dx + raw_dy * raw_dy);

                // 1. Sub-pixel deadzone governor: suppress micro-jitter when nearly stationary
                if raw_dist < SUBPIXEL_DEADZONE_PX {
                    fc.dx = 0.0;
                    fc.dy = 0.0;
                    fc.vx = 0.0;
                    fc.vy = 0.0;
                } else {
                    // 2. Adaptive Exponential Moving Average (EMA) smoothing factor:
                    // High speed -> alpha = 0.85 (responsive, zero lag)
                    // Low speed  -> alpha = 0.25 (heavy noise reduction)
                    let speed = raw_dist / dt_ms; // px/ms
                    let alpha = if speed > 1.5 { 0.85 } else if speed > 0.5 { 0.55 } else { 0.25 };

                    let smooth_x = fc.filtered_x + alpha * (raw_c.x - fc.filtered_x);
                    let smooth_y = fc.filtered_y + alpha * (raw_c.y - fc.filtered_y);

                    fc.dx = smooth_x - fc.filtered_x;
                    fc.dy = smooth_y - fc.filtered_y;
                    fc.filtered_x = smooth_x;
                    fc.filtered_y = smooth_y;

                    fc.vx = fc.dx / dt_ms;
                    fc.vy = fc.dy / dt_ms;
                    fc.total_distance += sqrt_f32(fc.dx * fc.dx + fc.dy * fc.dy);
                }
            }
        }

        // Deactivate contacts not present in this frame
        for id in 0..MAX_TOUCHPAD_CONTACTS {
            if !active_in_frame[id] {
                self.contacts[id].active = false;
            }
        }
    }
}

/// Verification helper for Commit 2 jitter filter and contact tracking engine.
pub fn verify_jitter_filter_and_contact_tracking() -> bool {
    let mut engine = TouchpadTrackingEngine::new();

    // 1. Initial touch down at (500.0, 500.0)
    let mut frame0 = TouchpadFrame::empty(10_000);
    frame0.contact_count = 1;
    frame0.contacts[0] = TouchpadContact {
        contact_id: 0,
        state: TouchpadContactState::TouchDown,
        x: 500.0,
        y: 500.0,
        pressure_z: 1000,
        major_axis: 10,
        minor_axis: 10,
        orientation_deg: 0,
        timestamp_us: 10_000,
    };
    engine.process_frame(&frame0);

    let c0 = &engine.contacts[0];
    if !c0.active || c0.filtered_x != 500.0 || c0.filtered_y != 500.0 {
        return false;
    }

    // 2. Micro-jitter input: movement of only 0.5 px (below SUBPIXEL_DEADZONE_PX 1.2 px)
    let mut frame_jitter = TouchpadFrame::empty(20_000);
    frame_jitter.contact_count = 1;
    frame_jitter.contacts[0] = TouchpadContact {
        contact_id: 0,
        state: TouchpadContactState::Moving,
        x: 500.5,
        y: 500.3,
        pressure_z: 1000,
        major_axis: 10,
        minor_axis: 10,
        orientation_deg: 0,
        timestamp_us: 20_000,
    };
    engine.process_frame(&frame_jitter);

    // Deadzone governor must suppress displacement to 0
    let c_jit = &engine.contacts[0];
    if c_jit.dx != 0.0 || c_jit.dy != 0.0 {
        return false;
    }

    // 3. Significant deliberate movement: jump of 50.0 px (high speed)
    let mut frame_move = TouchpadFrame::empty(30_000);
    frame_move.contact_count = 1;
    frame_move.contacts[0] = TouchpadContact {
        contact_id: 0,
        state: TouchpadContactState::Moving,
        x: 550.0,
        y: 500.0,
        pressure_z: 1000,
        major_axis: 10,
        minor_axis: 10,
        orientation_deg: 0,
        timestamp_us: 30_000,
    };
    engine.process_frame(&frame_move);

    let c_move = &engine.contacts[0];
    // High speed EMA (alpha 0.85) produces strong responsive displacement (> 35.0 px)
    if c_move.dx < 35.0 || c_move.vx <= 0.0 {
        return false;
    }

    true
}
/// Discrete haptic feedback pulse types emitted on gesture threshold transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HapticPulseType {
    /// No haptic actuation.
    None,
    /// Subtle high-frequency tick emitted on incremental scroll step (e.g. 20 px boundary).
    LightScrollTick,
    /// Medium detent pulse emitted on pinch-to-zoom step threshold.
    PinchStepDetent,
    /// Sharp tactile click on discrete tap or physical pressure click.
    TapClick,
    /// Strong crisp snap on 3-finger workspace/task switch gesture.
    SwipeSnap,
}

/// Comprehensive Multi-Finger Gesture Recognizer and Haptic Event Synthesizer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchpadGestureRecognizer {
    /// Active contact tracking and jitter filtering engine.
    pub tracker: TouchpadTrackingEngine,
    /// Initial two-finger pinch distance at gesture onset.
    pub initial_pinch_dist: f32,
    /// Accumulated vertical scroll distance since last haptic tick.
    pub scroll_accum_y: f32,
    /// Touch-down timestamp for tap recognition (in us).
    pub touch_down_us: u64,
    /// Most recent haptic pulse synthesized.
    pub last_haptic_pulse: HapticPulseType,
}

impl TouchpadGestureRecognizer {
    /// Creates a new gesture recognizer instance.
    pub const fn new() -> Self {
        Self {
            tracker: TouchpadTrackingEngine::new(),
            initial_pinch_dist: 0.0,
            scroll_accum_y: 0.0,
            touch_down_us: 0,
            last_haptic_pulse: HapticPulseType::None,
        }
    }

    /// Processes a new multi-touch frame, recognizes gestures, and synthesizes haptic feedback pulses.
    pub fn process_frame(
        &mut self,
        frame: &TouchpadFrame,
    ) -> (Option<TouchpadGestureEvent>, HapticPulseType) {
        self.last_haptic_pulse = HapticPulseType::None;

        // 1. Update tracking engine with jitter filtering
        self.tracker.process_frame(frame);

        let active_count = frame.contact_count;

        // Track touch down timestamp for tap detection
        if active_count == 1 && self.touch_down_us == 0 {
            self.touch_down_us = frame.timestamp_us;
        }

        // 2. Gesture evaluation based on active finger count
        match active_count {
            0 => {
                // Check if a finger just lifted for SingleTap (held < 250ms, moved < 10px)
                if self.touch_down_us > 0 {
                    let duration_ms = (frame.timestamp_us.saturating_sub(self.touch_down_us)) / 1000;
                    self.touch_down_us = 0;
                    self.initial_pinch_dist = 0.0;
                    self.scroll_accum_y = 0.0;

                    let c0 = &self.tracker.contacts[0];
                    if duration_ms < 250 && c0.total_distance < 10.0 {
                        self.last_haptic_pulse = HapticPulseType::TapClick;
                        let event = TouchpadGestureEvent {
                            gesture_type: TouchpadGestureType::SingleTap { x: c0.filtered_x, y: c0.filtered_y },
                            state: TouchpadGestureState::Ended,
                            finger_count: 1,
                            timestamp_us: frame.timestamp_us,
                        };
                        return (Some(event), self.last_haptic_pulse);
                    }
                }
                (None, HapticPulseType::None)
            }

            1 => {
                // 1-Finger drag motion
                let c0 = &self.tracker.contacts[0];
                if c0.active && (c0.dx.abs() > 0.01 || c0.dy.abs() > 0.01) {
                    let event = TouchpadGestureEvent {
                        gesture_type: TouchpadGestureType::SingleDrag { dx: c0.dx, dy: c0.dy },
                        state: TouchpadGestureState::Changed,
                        finger_count: 1,
                        timestamp_us: frame.timestamp_us,
                    };
                    (Some(event), HapticPulseType::None)
                } else {
                    (None, HapticPulseType::None)
                }
            }

            2 => {
                self.touch_down_us = 0; // Cancel tap
                let c0 = &self.tracker.contacts[0];
                let c1 = &self.tracker.contacts[1];

                if c0.active && c1.active {
                    let dx = c1.filtered_x - c0.filtered_x;
                    let dy = c1.filtered_y - c0.filtered_y;
                    let current_dist = sqrt_f32(dx * dx + dy * dy);

                    if self.initial_pinch_dist == 0.0 {
                        self.initial_pinch_dist = current_dist;
                    }

                    let dist_delta = current_dist - self.initial_pinch_dist;

                    // If distance delta is significant (> 12px) and fingers move in opposite directions -> Pinch
                    if dist_delta.abs() > 12.0 {
                        let scale_delta = current_dist / self.initial_pinch_dist.max(1.0);
                        self.initial_pinch_dist = current_dist;
                        self.last_haptic_pulse = HapticPulseType::PinchStepDetent;

                        let event = TouchpadGestureEvent {
                            gesture_type: TouchpadGestureType::TwoFingerPinchZoom {
                                scale_delta,
                                center_x: (c0.filtered_x + c1.filtered_x) / 2.0,
                                center_y: (c0.filtered_y + c1.filtered_y) / 2.0,
                            },
                            state: TouchpadGestureState::Changed,
                            finger_count: 2,
                            timestamp_us: frame.timestamp_us,
                        };
                        return (Some(event), self.last_haptic_pulse);
                    } else {
                        // Parallel 2-finger scroll
                        let avg_dx = (c0.dx + c1.dx) / 2.0;
                        let avg_dy = (c0.dy + c1.dy) / 2.0;

                        self.scroll_accum_y += avg_dy;
                        if self.scroll_accum_y.abs() >= 20.0 {
                            self.last_haptic_pulse = HapticPulseType::LightScrollTick;
                            self.scroll_accum_y = 0.0;
                        }

                        let event = TouchpadGestureEvent {
                            gesture_type: TouchpadGestureType::TwoFingerScroll {
                                delta_x: avg_dx,
                                delta_y: avg_dy,
                                is_inertial: false,
                            },
                            state: TouchpadGestureState::Changed,
                            finger_count: 2,
                            timestamp_us: frame.timestamp_us,
                        };
                        return (Some(event), self.last_haptic_pulse);
                    }
                }
                (None, HapticPulseType::None)
            }

            3 => {
                // 3-Finger fast horizontal or vertical gesture swipe
                self.touch_down_us = 0;
                let c0 = &self.tracker.contacts[0];
                let c1 = &self.tracker.contacts[1];
                let c2 = &self.tracker.contacts[2];

                if c0.active && c1.active && c2.active {
                    let avg_dx = (c0.dx + c1.dx + c2.dx) / 3.0;
                    let avg_dy = (c0.dy + c1.dy + c2.dy) / 3.0;

                    if avg_dx.abs() > 30.0 && avg_dx.abs() > avg_dy.abs() * 1.5 {
                        self.last_haptic_pulse = HapticPulseType::SwipeSnap;
                        let event = TouchpadGestureEvent {
                            gesture_type: TouchpadGestureType::ThreeFingerSwipeHorizontal {
                                swipe_left: avg_dx < 0.0,
                            },
                            state: TouchpadGestureState::Ended,
                            finger_count: 3,
                            timestamp_us: frame.timestamp_us,
                        };
                        return (Some(event), self.last_haptic_pulse);
                    } else if avg_dy.abs() > 30.0 && avg_dy.abs() > avg_dx.abs() * 1.5 {
                        self.last_haptic_pulse = HapticPulseType::SwipeSnap;
                        let event = TouchpadGestureEvent {
                            gesture_type: TouchpadGestureType::ThreeFingerSwipeVertical {
                                swipe_up: avg_dy < 0.0,
                            },
                            state: TouchpadGestureState::Ended,
                            finger_count: 3,
                            timestamp_us: frame.timestamp_us,
                        };
                        return (Some(event), self.last_haptic_pulse);
                    }
                }
                (None, HapticPulseType::None)
            }

            _ => (None, HapticPulseType::None),
        }
    }
}

/// Verification helper for Commit 3 multi-finger gestures and haptic pulse synthesis.
pub fn verify_multi_finger_gestures_and_haptics() -> bool {
    let mut recognizer = TouchpadGestureRecognizer::new();

    // 1. Verify Single Tap Gesture (touch down and up in 50ms, 0 displacement)
    let mut frame_down = TouchpadFrame::empty(100_000);
    frame_down.contact_count = 1;
    frame_down.contacts[0] = TouchpadContact {
        contact_id: 0,
        state: TouchpadContactState::TouchDown,
        x: 400.0,
        y: 400.0,
        pressure_z: 1000,
        major_axis: 10,
        minor_axis: 10,
        orientation_deg: 0,
        timestamp_us: 100_000,
    };
    recognizer.process_frame(&frame_down);

    let frame_up = TouchpadFrame::empty(150_000); // 50ms later, 0 contacts
    let (tap_event, haptic_tap) = recognizer.process_frame(&frame_up);

    if haptic_tap != HapticPulseType::TapClick {
        return false;
    }
    match tap_event {
        Some(ev) => {
            if ev.finger_count != 1 || ev.state != TouchpadGestureState::Ended {
                return false;
            }
            match ev.gesture_type {
                TouchpadGestureType::SingleTap { x, y } => {
                    if x != 400.0 || y != 400.0 {
                        return false;
                    }
                }
                _ => return false,
            }
        }
        None => return false,
    }

    // 2. Verify 2-Finger Scroll and Scroll Haptic Tick
    let mut frame_2f_0 = TouchpadFrame::empty(200_000);
    frame_2f_0.contact_count = 2;
    frame_2f_0.contacts[0] = TouchpadContact {
        contact_id: 0,
        state: TouchpadContactState::TouchDown,
        x: 300.0,
        y: 300.0,
        pressure_z: 1000,
        major_axis: 10,
        minor_axis: 10,
        orientation_deg: 0,
        timestamp_us: 200_000,
    };
    frame_2f_0.contacts[1] = TouchpadContact {
        contact_id: 1,
        state: TouchpadContactState::TouchDown,
        x: 400.0,
        y: 300.0,
        pressure_z: 1000,
        major_axis: 10,
        minor_axis: 10,
        orientation_deg: 0,
        timestamp_us: 200_000,
    };
    recognizer.process_frame(&frame_2f_0);

    // Move both fingers downward by 25.0 px (exceeds 20 px scroll haptic tick)
    let mut frame_2f_1 = TouchpadFrame::empty(210_000);
    frame_2f_1.contact_count = 2;
    frame_2f_1.contacts[0] = TouchpadContact {
        contact_id: 0,
        state: TouchpadContactState::Moving,
        x: 300.0,
        y: 325.0,
        pressure_z: 1000,
        major_axis: 10,
        minor_axis: 10,
        orientation_deg: 0,
        timestamp_us: 210_000,
    };
    frame_2f_1.contacts[1] = TouchpadContact {
        contact_id: 1,
        state: TouchpadContactState::Moving,
        x: 400.0,
        y: 325.0,
        pressure_z: 1000,
        major_axis: 10,
        minor_axis: 10,
        orientation_deg: 0,
        timestamp_us: 210_000,
    };
    let (scroll_event, haptic_scroll) = recognizer.process_frame(&frame_2f_1);

    if haptic_scroll != HapticPulseType::LightScrollTick {
        return false;
    }
    match scroll_event {
        Some(ev) => {
            if ev.finger_count != 2 {
                return false;
            }
            match ev.gesture_type {
                TouchpadGestureType::TwoFingerScroll { delta_y, .. } => {
                    if delta_y <= 0.0 {
                        return false;
                    }
                }
                _ => return false,
            }
        }
        None => return false,
    }

    true
}