//! Linear Resonant Actuator (LRA) driver and dynamic resonance frequency tracking PLL.

// Constants and configuration.

/// Minimum supported LRA resonance tracking frequency in Hz.
pub const LRA_MIN_RESONANT_FREQ_HZ: f32 = 150.0;

/// Maximum supported LRA resonance tracking frequency in Hz.
pub const LRA_MAX_RESONANT_FREQ_HZ: f32 = 250.0;

/// Nominal center frequency for handheld PC LRA actuators (typically ~175Hz - 205Hz).
pub const LRA_NOMINAL_CENTER_FREQ_HZ: f32 = 200.0;

/// Maximum back-EMF noise deadband around zero (millivolts).
pub const BACK_EMF_NOISE_DEADBAND_MV: i16 = 15;

/// Configuration parameters for the LRA resonant tracking Phase-Locked Loop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LraPllConfig {
    /// Nominal initial resonant frequency in Hz.
    pub nominal_freq_hz: f32,
    /// Proportional gain factor for phase error correction.
    pub kp: f32,
    /// Integral gain factor for steady-state frequency drift.
    pub ki: f32,
    /// Phase error deadband in radians below which no adjustment is made.
    pub phase_deadband_rad: f32,
    /// Phase lock threshold in radians (e.g. ~5.7° = 0.10 rad).
    pub lock_threshold_rad: f32,
    /// Consecutive locked half-cycles required to declare PLL locked state.
    pub lock_consecutive_threshold: u16,
}

impl LraPllConfig {
    pub const fn default_handheld() -> Self {
        Self {
            nominal_freq_hz: LRA_NOMINAL_CENTER_FREQ_HZ,
            kp: 2.5,
            ki: 0.3,
            phase_deadband_rad: 0.015,
            lock_threshold_rad: 0.08,
            lock_consecutive_threshold: 6,
        }
    }
}

// Back-EMF zero-crossing detector.

/// Raw Back-EMF ADC voltage sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackEmfSample {
    /// Timestamp of ADC capture in microseconds.
    pub timestamp_us: u64,
    /// Measured coil back-EMF voltage in millivolts.
    pub voltage_mv: i16,
}

/// Transition direction of the back-EMF waveform through zero volts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZeroCrossingDirection {
    /// Voltage transitioned from negative to positive.
    Rising,
    /// Voltage transitioned from positive to negative.
    Falling,
}

/// Output event when back-EMF crosses zero.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZeroCrossingEvent {
    /// Interpolated exact microsecond timestamp of zero-crossing.
    pub crossing_time_us: f32,
    /// Direction of the crossing.
    pub direction: ZeroCrossingDirection,
    /// Measured peak-to-peak amplitude estimate in millivolts.
    pub peak_voltage_mv: i16,
}

/// Detects zero-crossings in back-EMF ADC voltage during actuator coasting intervals.
#[derive(Debug, Clone)]
pub struct BackEmfZeroCrossingDetector {
    /// Previous valid voltage sample.
    prev_sample: Option<BackEmfSample>,
    /// Recent peak positive voltage in current half-cycle.
    peak_positive_mv: i16,
    /// Recent peak negative voltage in current half-cycle.
    peak_negative_mv: i16,
    /// Noise deadband in millivolts.
    noise_deadband_mv: i16,
}

impl BackEmfZeroCrossingDetector {
    pub const fn new(noise_deadband_mv: i16) -> Self {
        Self {
            prev_sample: None,
            peak_positive_mv: 0,
            peak_negative_mv: 0,
            noise_deadband_mv,
        }
    }

    /// Feeds a new back-EMF ADC sample and returns a zero-crossing event if detected.
    pub fn process_sample(&mut self, sample: BackEmfSample) -> Option<ZeroCrossingEvent> {
        if sample.voltage_mv > self.peak_positive_mv {
            self.peak_positive_mv = sample.voltage_mv;
        }
        if sample.voltage_mv < self.peak_negative_mv {
            self.peak_negative_mv = sample.voltage_mv;
        }

        let prev = match self.prev_sample {
            Some(p) => p,
            None => {
                self.prev_sample = Some(sample);
                return None;
            }
        };

        self.prev_sample = Some(sample);

        // Detect zero-crossing transition: signs differ and exceed noise deadband
        let prev_v = prev.voltage_mv;
        let curr_v = sample.voltage_mv;

        if prev_v < -self.noise_deadband_mv && curr_v > self.noise_deadband_mv {
            // Rising zero crossing (- to +)
            // Linear sub-microsecond interpolation: t_zc = t0 + (0 - v0) / (v1 - v0) * dt
            let dt = (sample.timestamp_us - prev.timestamp_us) as f32;
            let dv = (curr_v - prev_v) as f32;
            let fraction = (-prev_v as f32) / dv;
            let crossing_time = (prev.timestamp_us as f32) + fraction * dt;

            let event = ZeroCrossingEvent {
                crossing_time_us: crossing_time,
                direction: ZeroCrossingDirection::Rising,
                peak_voltage_mv: self.peak_positive_mv - self.peak_negative_mv,
            };

            // Reset half-cycle peaks
            self.peak_negative_mv = 0;
            Some(event)
        } else if prev_v > self.noise_deadband_mv && curr_v < -self.noise_deadband_mv {
            // Falling zero crossing (+ to -)
            let dt = (sample.timestamp_us - prev.timestamp_us) as f32;
            let dv = (curr_v - prev_v) as f32;
            let fraction = (-prev_v as f32) / dv;
            let crossing_time = (prev.timestamp_us as f32) + fraction * dt;

            let event = ZeroCrossingEvent {
                crossing_time_us: crossing_time,
                direction: ZeroCrossingDirection::Falling,
                peak_voltage_mv: self.peak_positive_mv - self.peak_negative_mv,
            };

            self.peak_positive_mv = 0;
            Some(event)
        } else {
            None
        }
    }

    /// Resets the detector state.
    pub fn reset(&mut self) {
        self.prev_sample = None;
        self.peak_positive_mv = 0;
        self.peak_negative_mv = 0;
    }
}

// Resonant frequency tracking PLL.

/// Phase-Locked Loop (PLL) continuously tuning LRA drive frequency to mechanical resonance.
#[derive(Debug, Clone)]
pub struct LraPllTracker {
    /// PLL configuration.
    pub config: LraPllConfig,
    /// Current tuned drive frequency in Hz.
    pub current_drive_freq_hz: f32,
    /// Accumulated integral phase error for steady-state correction.
    pub integral_error: f32,
    /// Last recorded phase error in radians.
    pub last_phase_error_rad: f32,
    /// Timestamp of the last detected zero-crossing event in microseconds.
    pub last_crossing_time_us: f32,
    /// Number of consecutive cycles with phase error within lock threshold.
    pub consecutive_locked_cycles: u16,
    /// Whether the PLL has achieved frequency lock on the LRA resonance.
    pub is_locked: bool,
    /// Total zero crossings processed.
    pub total_crossings: u32,
}

impl LraPllTracker {
    /// Initializes an LRA PLL tracker with the specified configuration.
    pub fn new(config: LraPllConfig) -> Self {
        Self {
            config,
            current_drive_freq_hz: config.nominal_freq_hz,
            integral_error: 0.0,
            last_phase_error_rad: 0.0,
            last_crossing_time_us: 0.0,
            consecutive_locked_cycles: 0,
            is_locked: false,
            total_crossings: 0,
        }
    }

    /// Updates PLL state when a zero-crossing event is detected, tuning drive frequency to resonance.
    pub fn process_zero_crossing(&mut self, event: ZeroCrossingEvent) {
        self.total_crossings += 1;

        if self.last_crossing_time_us <= 0.0 {
            self.last_crossing_time_us = event.crossing_time_us;
            return;
        }

        let measured_half_period_us = event.crossing_time_us - self.last_crossing_time_us;
        self.last_crossing_time_us = event.crossing_time_us;

        if measured_half_period_us <= 0.0 {
            return;
        }

        // Expected half-period based on current oscillator drive frequency:
        // T_half_us = 1_000_000 / (2 * f_drive)
        let expected_half_period_us = 500_000.0 / self.current_drive_freq_hz;

        // Phase error in radians: delta_phi = (measured - expected) / expected * PI.
        let timing_error_us = measured_half_period_us - expected_half_period_us;
        let phase_error_rad = (timing_error_us / expected_half_period_us) * core::f32::consts::PI;
        self.last_phase_error_rad = phase_error_rad;

        // Check if within phase deadband
        if phase_error_rad.abs() > self.config.phase_deadband_rad {
            // Apply PI loop filter to frequency:
            // If phase_error < 0 (measured faster than drive), we need to INCREASE drive frequency.
            // Hence: delta_f = - (Kp * phase_error + Ki * integral_error)
            self.integral_error += phase_error_rad;
            // Anti-windup clamping on integral error (+/- 5.0 rad)
            self.integral_error = self.integral_error.clamp(-10.0, 10.0);

            let freq_adjustment = -(self.config.kp * phase_error_rad + self.config.ki * self.integral_error);
            let new_freq = self.current_drive_freq_hz + freq_adjustment;
            self.current_drive_freq_hz = new_freq.clamp(LRA_MIN_RESONANT_FREQ_HZ, LRA_MAX_RESONANT_FREQ_HZ);
        }

        // Evaluate lock status
        if phase_error_rad.abs() <= self.config.lock_threshold_rad {
            self.consecutive_locked_cycles = self.consecutive_locked_cycles.saturating_add(1);
            if self.consecutive_locked_cycles >= self.config.lock_consecutive_threshold {
                self.is_locked = true;
            }
        } else {
            self.consecutive_locked_cycles = 0;
            self.is_locked = false;
        }
    }

    /// Resets the PLL frequency to the nominal configuration.
    pub fn reset(&mut self) {
        self.current_drive_freq_hz = self.config.nominal_freq_hz;
        self.integral_error = 0.0;
        self.last_phase_error_rad = 0.0;
        self.last_crossing_time_us = 0.0;
        self.consecutive_locked_cycles = 0;
        self.is_locked = false;
        self.total_crossings = 0;
    }
}

// Unit tests and verification.

/// Unit test verifying LRA back-EMF zero-crossing detector and resonant frequency tracking PLL.
pub fn verify_lra_back_emf_and_pll_tracking() -> bool {
    // 1. Verify Back-EMF Zero-Crossing Detector
    let mut detector = BackEmfZeroCrossingDetector::new(BACK_EMF_NOISE_DEADBAND_MV);

    // Feed negative sample
    let s1 = BackEmfSample { timestamp_us: 1000, voltage_mv: -50 };
    if detector.process_sample(s1).is_some() {
        return false;
    }

    // Feed positive sample: rising crossing between 1000us (-50mV) and 1100us (+50mV)
    // Interpolated crossing should be at 1050us
    let s2 = BackEmfSample { timestamp_us: 1100, voltage_mv: 50 };
    let event = match detector.process_sample(s2) {
        Some(e) => e,
        None => return false,
    };
    if event.direction != ZeroCrossingDirection::Rising {
        return false;
    }
    if (event.crossing_time_us - 1050.0).abs() > 0.1 {
        return false;
    }

    // Noise rejection test: small signal within +/-15mV deadband should NOT trigger crossing
    let s3 = BackEmfSample { timestamp_us: 1200, voltage_mv: 10 };
    let s4 = BackEmfSample { timestamp_us: 1300, voltage_mv: -10 };
    detector.process_sample(s3);
    if detector.process_sample(s4).is_some() {
        return false;
    }

    // Verify resonant frequency tracking PLL convergence from 200 Hz to 220 Hz.
    let config = LraPllConfig::default_handheld();
    let mut pll = LraPllTracker::new(config);

    if (pll.current_drive_freq_hz - 200.0).abs() > 0.01 {
        return false;
    }
    if pll.is_locked {
        return false;
    }

    // Simulate 30 half-cycles at 220 Hz
    let true_half_period_us = 500_000.0 / 220.0; // 2272.727 us
    let mut current_time = 0.0;
    for i in 0..40 {
        current_time += true_half_period_us;
        let zc = ZeroCrossingEvent {
            crossing_time_us: current_time,
            direction: if i % 2 == 0 { ZeroCrossingDirection::Rising } else { ZeroCrossingDirection::Falling },
            peak_voltage_mv: 120,
        };
        pll.process_zero_crossing(zc);
    }

    // PLL must have converged towards 220 Hz (within +/-2 Hz) and declared lock
    if (pll.current_drive_freq_hz - 220.0).abs() > 2.0 {
        return false;
    }
    if !pll.is_locked {
        return false;
    }

    // 3. Frequency Bounds Clamp Test
    // If external vibration is 300 Hz (out of band), PLL must clamp to 250 Hz max
    let out_of_band_half_period = 500_000.0 / 300.0;
    for _ in 0..40 {
        current_time += out_of_band_half_period;
        let zc = ZeroCrossingEvent {
            crossing_time_us: current_time,
            direction: ZeroCrossingDirection::Rising,
            peak_voltage_mv: 100,
        };
        pll.process_zero_crossing(zc);
    }
    if pll.current_drive_freq_hz > LRA_MAX_RESONANT_FREQ_HZ {
        return false;
    }

    true
}


// Multi-zone spatial haptic routing.

/// Physical haptic actuator zone on the handheld device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HapticZone {
    LeftPalm = 0,
    RightPalm = 1,
    LeftTrigger = 2,
    RightTrigger = 3,
}

/// Total number of discrete physical haptic actuator zones.
pub const HAPTIC_ZONE_COUNT: usize = 4;

/// Bitmask for selecting one or more haptic zones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HapticZoneMask(pub u8);

impl HapticZoneMask {
    pub const LEFT_PALM: Self = Self(1 << 0);
    pub const RIGHT_PALM: Self = Self(1 << 1);
    pub const LEFT_TRIGGER: Self = Self(1 << 2);
    pub const RIGHT_TRIGGER: Self = Self(1 << 3);

    pub const ALL_PALMS: Self = Self((1 << 0) | (1 << 1));
    pub const ALL_TRIGGERS: Self = Self((1 << 2) | (1 << 3));
    pub const ALL_ZONES: Self = Self(0x0F);

    pub const fn contains(&self, zone: HapticZone) -> bool {
        (self.0 & (1 << (zone as u8))) != 0
    }

    pub const fn union(&self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// Instantaneous drive output commanded to a single zone's LRA driver IC.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoneDriveOutput {
    /// Actuator zone identifier.
    pub zone: HapticZone,
    /// Commanded amplitude level [0..255].
    pub amplitude: u8,
    /// Current drive frequency in Hz (dynamically tracked by PLL).
    pub frequency_hz: f32,
    /// Commanded phase offset in degrees (0.0 to 360.0).
    pub phase_deg: f32,
    /// Whether this zone is actively vibrating.
    pub active: bool,
}

impl ZoneDriveOutput {
    pub const fn off(zone: HapticZone) -> Self {
        Self {
            zone,
            amplitude: 0,
            frequency_hz: LRA_NOMINAL_CENTER_FREQ_HZ,
            phase_deg: 0.0,
            active: false,
        }
    }
}

/// Spatial haptic command describing a localized or wide tactile sensation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpatialHapticCommand {
    /// Base vibration intensity [0..255].
    pub base_intensity: u8,
    /// Stereo horizontal pan [-1.0 (full left) .. +1.0 (full right)].
    pub pan_x: f32,
    /// Zone emphasis: 0.0 = pure palm grips, 1.0 = pure resistive triggers, 0.5 = balanced.
    pub trigger_emphasis: f32,
    /// Target zone mask selecting which actuators receive this effect.
    pub target_mask: HapticZoneMask,
}

impl SpatialHapticCommand {
    pub const fn new(
        base_intensity: u8,
        pan_x: f32,
        trigger_emphasis: f32,
        target_mask: HapticZoneMask,
    ) -> Self {
        Self {
            base_intensity,
            pan_x,
            trigger_emphasis,
            target_mask,
        }
    }

    pub const fn center_all(intensity: u8) -> Self {
        Self {
            base_intensity: intensity,
            pan_x: 0.0,
            trigger_emphasis: 0.0,
            target_mask: HapticZoneMask::ALL_ZONES,
        }
    }

    pub const fn trigger_click(intensity: u8, right_trigger: bool) -> Self {
        let mask = if right_trigger {
            HapticZoneMask::RIGHT_TRIGGER
        } else {
            HapticZoneMask::LEFT_TRIGGER
        };
        let pan = if right_trigger { 1.0 } else { -1.0 };
        Self {
            base_intensity: intensity,
            pan_x: pan,
            trigger_emphasis: 1.0,
            target_mask: mask,
        }
    }
}

/// Multi-zone spatial haptic routing governor managing 4 physical LRA actuators.
#[derive(Debug, Clone)]
pub struct MultiZoneHapticRouter {
    /// Dedicated PLL tracker for each of the 4 zones.
    pub plls: [LraPllTracker; HAPTIC_ZONE_COUNT],
    /// Current drive outputs for each zone.
    pub outputs: [ZoneDriveOutput; HAPTIC_ZONE_COUNT],
}

impl MultiZoneHapticRouter {
    /// Initializes multi-zone router with default handheld LRA resonant frequencies.
    /// Palms default to 175 Hz (deep rumble), triggers default to 235 Hz (crisp click).
    pub fn new() -> Self {
        let mut palm_cfg = LraPllConfig::default_handheld();
        palm_cfg.nominal_freq_hz = 175.0;

        let mut trigger_cfg = LraPllConfig::default_handheld();
        trigger_cfg.nominal_freq_hz = 235.0;

        Self {
            plls: [
                LraPllTracker::new(palm_cfg),
                LraPllTracker::new(palm_cfg),
                LraPllTracker::new(trigger_cfg),
                LraPllTracker::new(trigger_cfg),
            ],
            outputs: [
                ZoneDriveOutput::off(HapticZone::LeftPalm),
                ZoneDriveOutput::off(HapticZone::RightPalm),
                ZoneDriveOutput::off(HapticZone::LeftTrigger),
                ZoneDriveOutput::off(HapticZone::RightTrigger),
            ],
        }
    }

    /// Evaluates spatial routing matrix for a given spatial haptic command.
    pub fn route_command(&mut self, cmd: SpatialHapticCommand) -> [ZoneDriveOutput; HAPTIC_ZONE_COUNT] {
        let pan = cmd.pan_x.clamp(-1.0, 1.0);
        let emphasis = cmd.trigger_emphasis.clamp(0.0, 1.0);

        // Left / Right spatial panning gains
        let left_gain = if pan <= 0.0 { 1.0 } else { 1.0 - pan };
        let right_gain = if pan >= 0.0 { 1.0 } else { 1.0 + pan };

        // Palm vs Trigger emphasis gains
        let palm_gain = 1.0 - emphasis;
        let trigger_gain = emphasis;

        let zones = [
            (HapticZone::LeftPalm, left_gain * palm_gain),
            (HapticZone::RightPalm, right_gain * palm_gain),
            (HapticZone::LeftTrigger, left_gain * trigger_gain),
            (HapticZone::RightTrigger, right_gain * trigger_gain),
        ];

        for (idx, &(zone, spatial_gain)) in zones.iter().enumerate() {
            let active = cmd.target_mask.contains(zone) && spatial_gain > 0.001 && cmd.base_intensity > 0;
            let amp = if active {
                let scaled = (cmd.base_intensity as f32) * spatial_gain + 0.5;
                (scaled as u8).min(255)
            } else {
                0
            };

            self.outputs[idx] = ZoneDriveOutput {
                zone,
                amplitude: amp,
                frequency_hz: self.plls[idx].current_drive_freq_hz,
                phase_deg: 0.0,
                active: amp > 0,
            };
        }

        self.outputs
    }

    /// Feeds a back-EMF zero-crossing event to the corresponding zone's PLL tracker.
    pub fn update_zone_resonance(&mut self, zone: HapticZone, event: ZeroCrossingEvent) {
        let idx = zone as usize;
        if idx < HAPTIC_ZONE_COUNT {
            self.plls[idx].process_zero_crossing(event);
            self.outputs[idx].frequency_hz = self.plls[idx].current_drive_freq_hz;
        }
    }

    /// Halts all active actuators.
    pub fn stop_all(&mut self) {
        for idx in 0..HAPTIC_ZONE_COUNT {
            self.outputs[idx].amplitude = 0;
            self.outputs[idx].active = false;
        }
    }
}

/// Unit test verifying multi-zone spatial haptic routing.
pub fn verify_multizone_spatial_routing() -> bool {
    let mut router = MultiZoneHapticRouter::new();

    // 1. Initial State Check
    // Palms should be 175 Hz, Triggers should be 235 Hz
    if (router.plls[HapticZone::LeftPalm as usize].current_drive_freq_hz - 175.0).abs() > 0.1 {
        return false;
    }
    if (router.plls[HapticZone::RightTrigger as usize].current_drive_freq_hz - 235.0).abs() > 0.1 {
        return false;
    }

    // 2. Full Center Palm Rumble (pan = 0.0, emphasis = 0.0)
    let cmd_palm = SpatialHapticCommand::new(200, 0.0, 0.0, HapticZoneMask::ALL_ZONES);
    let out = router.route_command(cmd_palm);
    // Both Left and Right palms should receive full 200 intensity
    if out[HapticZone::LeftPalm as usize].amplitude != 200 || out[HapticZone::RightPalm as usize].amplitude != 200 {
        return false;
    }
    // Triggers should be 0 because trigger_emphasis is 0.0
    if out[HapticZone::LeftTrigger as usize].amplitude != 0 || out[HapticZone::RightTrigger as usize].amplitude != 0 {
        return false;
    }

    // 3. Hard Left Stereo Panning (pan = -1.0, emphasis = 0.0)
    let cmd_left = SpatialHapticCommand::new(200, -1.0, 0.0, HapticZoneMask::ALL_PALMS);
    let out_left = router.route_command(cmd_left);
    if out_left[HapticZone::LeftPalm as usize].amplitude != 200 {
        return false;
    }
    if out_left[HapticZone::RightPalm as usize].amplitude != 0 {
        return false;
    }

    // 4. Isolated Right Trigger Tactical Click (pan = 1.0, emphasis = 1.0)
    let cmd_r_trig = SpatialHapticCommand::trigger_click(255, true);
    let out_trig = router.route_command(cmd_r_trig);
    if out_trig[HapticZone::RightTrigger as usize].amplitude != 255 {
        return false;
    }
    if out_trig[HapticZone::LeftTrigger as usize].amplitude != 0 {
        return false;
    }
    if out_trig[HapticZone::LeftPalm as usize].amplitude != 0 || out_trig[HapticZone::RightPalm as usize].amplitude != 0 {
        return false;
    }
    // Frequency for Right Trigger should match its tuned 235 Hz
    if (out_trig[HapticZone::RightTrigger as usize].frequency_hz - 235.0).abs() > 0.1 {
        return false;
    }

    true
}
