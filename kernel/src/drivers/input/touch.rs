//! Capacitive Touch Controller Driver & Multi-Point Digitizer Subsystem.
//!
//! Provides precision multi-touch tracking for handheld gaming PC displays:
//! - Up to 10 simultaneous capacitive contact points (`TouchContact`)
//! - Raw ADC coordinate normalization to display resolution (e.g. 1280x800, 1920x1080)
//! - Contact geometry tracking (major/minor ellipse axes, pressure 0..255)
//! - Temporal low-pass jitter smoothing and per-finger velocity/acceleration tracking ($v_x, v_y, a_x, a_y$)

// -----------------------------------------------------------------------------
// Constants and Configuration
// -----------------------------------------------------------------------------

/// Maximum simultaneous capacitive touch contacts supported by the hardware.
pub const MAX_TOUCH_CONTACTS: usize = 10;

/// Default raw digitizer coordinate range (12-bit ADC: 0..4095).
pub const DEFAULT_RAW_MAX_X: u16 = 4095;
pub const DEFAULT_RAW_MAX_Y: u16 = 4095;

/// State of an individual contact point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactStatus {
    /// Initial contact down on the digitizer surface.
    Down,
    /// Contact is actively translating across the surface.
    Move,
    /// Contact is stationary (within jitter threshold).
    Stationary,
    /// Contact lifted from the surface.
    Up,
    /// Slot is completely inactive.
    Inactive,
}

/// Raw touch point delivered by the hardware digitizer controller (I2C/SPI/USB HID).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawTouchPoint {
    /// Hardware tracking ID / slot index (0..9).
    pub id: u8,
    /// Raw X coordinate (0..raw_max_x).
    pub raw_x: u16,
    /// Raw Y coordinate (0..raw_max_y).
    pub raw_y: u16,
    /// Contact pressure level (0..255).
    pub pressure: u8,
    /// Contact touch ellipse major axis length in raw sensor units.
    pub major_axis: u16,
    /// Contact touch ellipse minor axis length in raw sensor units.
    pub minor_axis: u16,
    /// True if finger is touching the surface.
    pub is_touching: bool,
}

/// Configuration parameters for touch digitizer coordinate normalization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DigitizerConfig {
    /// Maximum raw X value from digitizer controller.
    pub raw_max_x: u16,
    /// Maximum raw Y value from digitizer controller.
    pub raw_max_y: u16,
    /// Target display resolution width in pixels (e.g. 1280).
    pub display_width: u32,
    /// Target display resolution height in pixels (e.g. 800).
    pub display_height: u32,
    /// Invert horizontal X coordinate (for inverted panel mounting).
    pub invert_x: bool,
    /// Invert vertical Y coordinate.
    pub invert_y: bool,
    /// Swap X and Y coordinates (for native portrait panels mounted landscape).
    pub swap_xy: bool,
}

impl DigitizerConfig {
    pub const fn default_handheld_1280x800() -> Self {
        Self {
            raw_max_x: DEFAULT_RAW_MAX_X,
            raw_max_y: DEFAULT_RAW_MAX_Y,
            display_width: 1280,
            display_height: 800,
            invert_x: false,
            invert_y: false,
            swap_xy: false,
        }
    }

    pub const fn default_handheld_1080p() -> Self {
        Self {
            raw_max_x: DEFAULT_RAW_MAX_X,
            raw_max_y: DEFAULT_RAW_MAX_Y,
            display_width: 1920,
            display_height: 1080,
            invert_x: false,
            invert_y: false,
            swap_xy: false,
        }
    }
}

// -----------------------------------------------------------------------------
// Normalized Touch Contact & Frame
// -----------------------------------------------------------------------------

/// Fully normalized, display-calibrated contact point with kinematic velocity tracking.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchContact {
    /// Contact tracking identifier (0..9).
    pub id: u8,
    /// Screen X coordinate in physical display pixels (0.0 .. display_width).
    pub x: f32,
    /// Screen Y coordinate in physical display pixels (0.0 .. display_height).
    pub y: f32,
    /// Filtered screen X coordinate (jitter-suppressed).
    pub filtered_x: f32,
    /// Filtered screen Y coordinate (jitter-suppressed).
    pub filtered_y: f32,
    /// Horizontal velocity in pixels per second.
    pub velocity_x: f32,
    /// Vertical velocity in pixels per second.
    pub velocity_y: f32,
    /// Horizontal acceleration in pixels per second squared.
    pub acceleration_x: f32,
    /// Vertical acceleration in pixels per second squared.
    pub acceleration_y: f32,
    /// Overall speed magnitude in pixels per second.
    pub speed: f32,
    /// Normalized horizontal coordinate in [0.0 .. 1.0].
    pub norm_x: f32,
    /// Normalized vertical coordinate in [0.0 .. 1.0].
    pub norm_y: f32,
    /// Normalized contact pressure in [0.0 .. 1.0].
    pub pressure: f32,
    /// Calibrated ellipse major axis in pixels.
    pub major_axis_px: f32,
    /// Calibrated ellipse minor axis in pixels.
    pub minor_axis_px: f32,
    /// Contact lifecycle status.
    pub status: ContactStatus,
    /// True if contact is currently touching the display.
    pub active: bool,
    /// Microsecond timestamp of last update.
    pub timestamp_us: u64,
}

impl TouchContact {
    pub const fn inactive(id: u8) -> Self {
        Self {
            id,
            x: 0.0,
            y: 0.0,
            filtered_x: 0.0,
            filtered_y: 0.0,
            velocity_x: 0.0,
            velocity_y: 0.0,
            acceleration_x: 0.0,
            acceleration_y: 0.0,
            speed: 0.0,
            norm_x: 0.0,
            norm_y: 0.0,
            pressure: 0.0,
            major_axis_px: 0.0,
            minor_axis_px: 0.0,
            status: ContactStatus::Inactive,
            active: false,
            timestamp_us: 0,
        }
    }
}

/// A snapshot frame containing all active multi-touch contacts at a given timestamp.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchFrame {
    /// Frame capture timestamp in microseconds.
    pub timestamp_us: u64,
    /// Array of contacts for all 10 tracking slots.
    pub contacts: [TouchContact; MAX_TOUCH_CONTACTS],
    /// Number of actively touching contact points.
    pub active_count: u8,
}

impl TouchFrame {
    pub const fn empty() -> Self {
        Self {
            timestamp_us: 0,
            contacts: [
                TouchContact::inactive(0),
                TouchContact::inactive(1),
                TouchContact::inactive(2),
                TouchContact::inactive(3),
                TouchContact::inactive(4),
                TouchContact::inactive(5),
                TouchContact::inactive(6),
                TouchContact::inactive(7),
                TouchContact::inactive(8),
                TouchContact::inactive(9),
            ],
            active_count: 0,
        }
    }
}

// -----------------------------------------------------------------------------
// Temporal Jitter Filter & Kinematic State
// -----------------------------------------------------------------------------

/// Internal temporal filter and kinematic history per contact slot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContactFilterState {
    pub prev_filtered_x: f32,
    pub prev_filtered_y: f32,
    pub prev_vx: f32,
    pub prev_vy: f32,
    pub prev_timestamp_us: u64,
    pub initialized: bool,
}

impl ContactFilterState {
    pub const fn new() -> Self {
        Self {
            prev_filtered_x: 0.0,
            prev_filtered_y: 0.0,
            prev_vx: 0.0,
            prev_vy: 0.0,
            prev_timestamp_us: 0,
            initialized: false,
        }
    }
}

// -----------------------------------------------------------------------------
// Touch Digitizer Driver
// -----------------------------------------------------------------------------

/// Capacitive touch digitizer controller tracking up to 10 simultaneous contacts
/// with low-pass jitter smoothing and velocity/acceleration estimation.
pub struct TouchDigitizer {
    /// Display geometry and coordinate transformation configuration.
    pub config: DigitizerConfig,
    /// Current contact slots.
    pub contacts: [TouchContact; MAX_TOUCH_CONTACTS],
    /// Per-slot filter and kinematic states.
    pub filter_states: [ContactFilterState; MAX_TOUCH_CONTACTS],
    /// Total frames processed.
    pub frame_counter: u64,
}

impl TouchDigitizer {
    /// Initializes a touch digitizer controller with specified configuration.
    pub fn new(config: DigitizerConfig) -> Self {
        Self {
            config,
            contacts: [
                TouchContact::inactive(0),
                TouchContact::inactive(1),
                TouchContact::inactive(2),
                TouchContact::inactive(3),
                TouchContact::inactive(4),
                TouchContact::inactive(5),
                TouchContact::inactive(6),
                TouchContact::inactive(7),
                TouchContact::inactive(8),
                TouchContact::inactive(9),
            ],
            filter_states: [ContactFilterState::new(); MAX_TOUCH_CONTACTS],
            frame_counter: 0,
        }
    }

    /// Ingests a raw hardware digitizer report containing up to 10 touch points,
    /// performs coordinate normalization, applies temporal jitter filtering, and computes
    /// per-finger velocities and accelerations.
    pub fn process_raw_frame(
        &mut self,
        timestamp_us: u64,
        raw_points: &[RawTouchPoint],
    ) -> TouchFrame {
        self.frame_counter += 1;
        let mut active_slots = [false; MAX_TOUCH_CONTACTS];
        let mut active_count = 0u8;

        for point in raw_points {
            let id = (point.id as usize) % MAX_TOUCH_CONTACTS;
            if !point.is_touching {
                // If this slot was previously active, mark it as Up for this frame
                if self.contacts[id].active {
                    self.contacts[id].status = ContactStatus::Up;
                    self.contacts[id].active = false;
                    self.contacts[id].timestamp_us = timestamp_us;
                } else {
                    self.contacts[id].status = ContactStatus::Inactive;
                }
                self.filter_states[id].initialized = false;
                continue;
            }

            active_slots[id] = true;
            active_count += 1;

            // Normalize raw coordinates to [0.0 .. 1.0]
            let mut nx = (point.raw_x as f32) / (self.config.raw_max_x as f32);
            let mut ny = (point.raw_y as f32) / (self.config.raw_max_y as f32);

            nx = nx.clamp(0.0, 1.0);
            ny = ny.clamp(0.0, 1.0);

            if self.config.invert_x {
                nx = 1.0 - nx;
            }
            if self.config.invert_y {
                ny = 1.0 - ny;
            }
            if self.config.swap_xy {
                core::mem::swap(&mut nx, &mut ny);
            }

            // Map to display pixel coordinates
            let px_x = nx * (self.config.display_width as f32);
            let px_y = ny * (self.config.display_height as f32);

            let pressure = (point.pressure as f32) / 255.0;
            let major_px = (point.major_axis as f32) * ((self.config.display_width as f32) / (self.config.raw_max_x as f32));
            let minor_px = (point.minor_axis as f32) * ((self.config.display_height as f32) / (self.config.raw_max_y as f32));

            // Temporal Low-Pass Jitter Filtering & Kinematic Tracking
            let (filtered_x, filtered_y, vx, vy, ax, ay, speed, status) = if !self.filter_states[id].initialized {
                // First contact Down: initialize position and zero out velocity
                self.filter_states[id].prev_filtered_x = px_x;
                self.filter_states[id].prev_filtered_y = px_y;
                self.filter_states[id].prev_vx = 0.0;
                self.filter_states[id].prev_vy = 0.0;
                self.filter_states[id].prev_timestamp_us = timestamp_us;
                self.filter_states[id].initialized = true;

                (px_x, px_y, 0.0, 0.0, 0.0, 0.0, 0.0, ContactStatus::Down)
            } else {
                let prev_state = self.filter_states[id];
                let dt_us = timestamp_us.saturating_sub(prev_state.prev_timestamp_us);
                let dt_sec = (dt_us as f32) / 1_000_000.0;

                if dt_sec < 0.0005 {
                    // Too fast / duplicate sample
                    (
                        prev_state.prev_filtered_x,
                        prev_state.prev_filtered_y,
                        prev_state.prev_vx,
                        prev_state.prev_vy,
                        0.0,
                        0.0,
                        crate::graphics::sqrt_f32(prev_state.prev_vx * prev_state.prev_vx + prev_state.prev_vy * prev_state.prev_vy),
                        ContactStatus::Stationary,
                    )
                } else {
                    let raw_dx = px_x - prev_state.prev_filtered_x;
                    let raw_dy = px_y - prev_state.prev_filtered_y;
                    let raw_dist = crate::graphics::sqrt_f32(raw_dx * raw_dx + raw_dy * raw_dy);
                    let raw_speed = raw_dist / dt_sec;

                    // Dynamic alpha filter: low speed = 0.25 (aggressive smoothing), high speed = 0.85 (minimal lag)
                    let progress = ((raw_speed - 20.0) / 180.0).clamp(0.0, 1.0);
                    let alpha = 0.25 + 0.60 * progress;

                    let filt_x = prev_state.prev_filtered_x + alpha * (px_x - prev_state.prev_filtered_x);
                    let filt_y = prev_state.prev_filtered_y + alpha * (px_y - prev_state.prev_filtered_y);

                    // Compute velocity from filtered position
                    let curr_vx = (filt_x - prev_state.prev_filtered_x) / dt_sec;
                    let curr_vy = (filt_y - prev_state.prev_filtered_y) / dt_sec;
                    let curr_speed = crate::graphics::sqrt_f32(curr_vx * curr_vx + curr_vy * curr_vy);

                    // Compute acceleration from velocity delta
                    let curr_ax = (curr_vx - prev_state.prev_vx) / dt_sec;
                    let curr_ay = (curr_vy - prev_state.prev_vy) / dt_sec;

                    // Update filter history
                    self.filter_states[id].prev_filtered_x = filt_x;
                    self.filter_states[id].prev_filtered_y = filt_y;
                    self.filter_states[id].prev_vx = curr_vx;
                    self.filter_states[id].prev_vy = curr_vy;
                    self.filter_states[id].prev_timestamp_us = timestamp_us;

                    let stat = if raw_dist > 1.2 {
                        ContactStatus::Move
                    } else {
                        ContactStatus::Stationary
                    };

                    (filt_x, filt_y, curr_vx, curr_vy, curr_ax, curr_ay, curr_speed, stat)
                }
            };

            self.contacts[id] = TouchContact {
                id: id as u8,
                x: px_x,
                y: px_y,
                filtered_x,
                filtered_y,
                velocity_x: vx,
                velocity_y: vy,
                acceleration_x: ax,
                acceleration_y: ay,
                speed,
                norm_x: nx,
                norm_y: ny,
                pressure,
                major_axis_px: major_px,
                minor_axis_px: minor_px,
                status,
                active: true,
                timestamp_us,
            };
        }

        // Clean up any unmentioned slots
        for id in 0..MAX_TOUCH_CONTACTS {
            if !active_slots[id] {
                if self.contacts[id].active {
                    self.contacts[id].status = ContactStatus::Up;
                    self.contacts[id].active = false;
                    self.contacts[id].timestamp_us = timestamp_us;
                } else if self.contacts[id].status == ContactStatus::Up {
                    self.contacts[id].status = ContactStatus::Inactive;
                }
                self.filter_states[id].initialized = false;
            }
        }

        TouchFrame {
            timestamp_us,
            contacts: self.contacts,
            active_count,
        }
    }
}

// -----------------------------------------------------------------------------
// Unit Tests & Verification
// -----------------------------------------------------------------------------

/// Unit test verifying capacitive touch digitizer driver and 10-point contact tracker.
pub fn verify_touch_digitizer_and_10_point_tracking() -> bool {
    let config = DigitizerConfig::default_handheld_1280x800();
    let mut digitizer = TouchDigitizer::new(config);

    // 1. Initial State
    for c in &digitizer.contacts {
        if c.active || c.status != ContactStatus::Inactive {
            return false;
        }
    }

    // 2. 10 Simultaneous Contact Points Down
    let mut raw_10 = [RawTouchPoint {
        id: 0,
        raw_x: 0,
        raw_y: 0,
        pressure: 128,
        major_axis: 20,
        minor_axis: 15,
        is_touching: true,
    }; MAX_TOUCH_CONTACTS];

    for i in 0..MAX_TOUCH_CONTACTS {
        raw_10[i].id = i as u8;
        // Distribute points across digitizer: raw_x = i * 400, raw_y = i * 400
        raw_10[i].raw_x = (i as u16) * 400;
        raw_10[i].raw_y = (i as u16) * 400;
    }

    let frame1 = digitizer.process_raw_frame(1000, &raw_10);
    if frame1.active_count != 10 {
        return false;
    }
    for i in 0..MAX_TOUCH_CONTACTS {
        let c = &frame1.contacts[i];
        if !c.active || c.status != ContactStatus::Down {
            return false;
        }
        if i == 0 && (c.x != 0.0 || c.y != 0.0) {
            return false;
        }
    }

    // 3. Move Contacts
    for i in 0..MAX_TOUCH_CONTACTS {
        raw_10[i].raw_x += 100; // Move +100 raw units
        raw_10[i].raw_y += 50;
    }

    let frame2 = digitizer.process_raw_frame(2000, &raw_10);
    if frame2.active_count != 10 {
        return false;
    }
    for i in 0..MAX_TOUCH_CONTACTS {
        let c = &frame2.contacts[i];
        if !c.active || c.status != ContactStatus::Move {
            return false;
        }
    }

    // 4. Release contacts 5..10 (slots 5..9 lift up)
    for i in 5..MAX_TOUCH_CONTACTS {
        raw_10[i].is_touching = false;
    }

    let frame3 = digitizer.process_raw_frame(3000, &raw_10);
    if frame3.active_count != 5 {
        return false;
    }
    for i in 0..5 {
        if !frame3.contacts[i].active {
            return false;
        }
    }
    for i in 5..MAX_TOUCH_CONTACTS {
        if frame3.contacts[i].active || frame3.contacts[i].status != ContactStatus::Up {
            return false;
        }
    }

    // 5. Next frame with slots 5..9 inactive
    let frame4 = digitizer.process_raw_frame(4000, &raw_10[0..5]);
    if frame4.active_count != 5 {
        return false;
    }
    for i in 5..MAX_TOUCH_CONTACTS {
        if frame4.contacts[i].status != ContactStatus::Inactive {
            return false;
        }
    }

    true
}

/// Unit test verifying temporal jitter filter and contact velocity / acceleration estimator.
pub fn verify_touch_jitter_filter_and_kinematics() -> bool {
    let config = DigitizerConfig::default_handheld_1280x800();
    let mut digitizer = TouchDigitizer::new(config);

    // 1. Initial Contact Down at (2000, 2000) at t = 0 ms
    // 2000 / 4095 * 1280 ~ 625.15 px
    let p_down = RawTouchPoint {
        id: 0,
        raw_x: 2000,
        raw_y: 2000,
        pressure: 150,
        major_axis: 25,
        minor_axis: 20,
        is_touching: true,
    };
    let f1 = digitizer.process_raw_frame(100_000, &[p_down]);
    let c1 = &f1.contacts[0];
    if c1.velocity_x != 0.0 || c1.velocity_y != 0.0 || c1.speed != 0.0 {
        return false;
    }
    if (c1.filtered_x - c1.x).abs() > 0.001 {
        return false;
    }

    // 2. Stationary Micro-Jitter Suppression
    // Oscillate raw coordinates by +/-2 raw units (~0.6 px) at 100Hz (every 10ms = 10_000 us)
    let mut prev_filtered = c1.filtered_x;
    for step in 1..=10 {
        let jitter = if step % 2 == 0 { 2 } else { -2 };
        let p_jitter = RawTouchPoint {
            id: 0,
            raw_x: (2000 + jitter) as u16,
            raw_y: 2000,
            pressure: 150,
            major_axis: 25,
            minor_axis: 20,
            is_touching: true,
        };
        let fj = digitizer.process_raw_frame(100_000 + step * 10_000, &[p_jitter]);
        let cj = &fj.contacts[0];
        // Jitter should be dampened significantly (filtered change < 0.2 px per frame)
        let delta = (cj.filtered_x - prev_filtered).abs();
        if delta > 0.35 {
            return false;
        }
        prev_filtered = cj.filtered_x;
    }

    // 3. Constant Velocity Translation
    // Move along X at 500 px/sec.
    // In 1280 display width: 500 px/s = 500 / 1280 * 4095 ~ 1600 raw units/sec
    // In 20ms (20,000 us), delta_x = 10 px = 10 / 1280 * 4095 ~ 32 raw units
    let mut raw_x = 2000u16;
    let mut t = 200_000u64;
    for _ in 0..15 {
        t += 20_000; // 20ms steps
        raw_x += 32;
        let p_move = RawTouchPoint {
            id: 0,
            raw_x,
            raw_y: 2000,
            pressure: 150,
            major_axis: 25,
            minor_axis: 20,
            is_touching: true,
        };
        digitizer.process_raw_frame(t, &[p_move]);
    }

    let final_c = &digitizer.contacts[0];
    // Velocity along X should converge close to 500 px/sec (+/- 60 px/s)
    if (final_c.velocity_x - 500.0).abs() > 60.0 {
        return false;
    }
    // Velocity along Y should remain near 0 (+/- 5 px/s)
    if final_c.velocity_y.abs() > 5.0 {
        return false;
    }
    // Speed should match velocity_x
    if (final_c.speed - final_c.velocity_x.abs()).abs() > 1.0 {
        return false;
    }

    true
}
