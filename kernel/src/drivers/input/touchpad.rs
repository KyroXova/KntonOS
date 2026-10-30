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
    TouchContactDown,
    /// Capacitive touch contact released (CapacitiveTouch -> Hover).
    TouchContactUp,
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
                        ClickTransition::TouchContactDown
                    }
                } else {
                    ClickTransition::None
                }
            }
            TouchpadClickStage::CapacitiveTouch => {
                if !has_contact {
                    self.click_stage = TouchpadClickStage::Hover;
                    ClickTransition::TouchContactUp
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
                    ClickTransition::TouchContactUp
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
    // Must transition to CapacitiveTouch with TouchContactDown
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
    if m_touch.click_transition != ClickTransition::TouchContactDown {
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
    // Must transition to Hover with TouchContactUp
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
    if m_lift.click_transition != ClickTransition::TouchContactUp {
        return false;
    }

    true
}
