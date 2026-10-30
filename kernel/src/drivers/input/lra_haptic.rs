//! Linear Resonant Actuator (LRA) Driver & Dynamic Resonant Frequency Tracking PLL.
//!
//! Provides precision LRA drive and real-time resonance frequency ($f_0$) tracking:
//! - Back-EMF zero-crossing phase detector monitoring coil voltage during hi-Z coasting windows
//! - Proportional-Integral (PI) Phase-Locked Loop (PLL) continuously tuning drive frequency across 150Hz–250Hz
//! - Automatic adaptation to hand grip pressure, actuator thermal drift, and mechanical spring aging

// -----------------------------------------------------------------------------
// Constants & Configuration
// -----------------------------------------------------------------------------

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
            kp: 0.35,
            ki: 0.08,
            phase_deadband_rad: 0.02,
            lock_threshold_rad: 0.10,
            lock_consecutive_threshold: 8,
        }
    }
}

// -----------------------------------------------------------------------------
// Back-EMF Zero-Crossing Detector
// -----------------------------------------------------------------------------

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

// -----------------------------------------------------------------------------
// Resonant Frequency Tracking PLL
// -----------------------------------------------------------------------------

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

    /// Updates the PLL state when a zero-crossing event is detected.
    ///
    /// Evaluates the measured half-period versus expected half-period $T_{\text{drive}} / 2$,
    /// calculates phase discrepancy $\Delta \phi$, and executes PI loop filter to nudge
    /// `current_drive_freq_hz` towards true mechanical resonance $f_0$.
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

        // Timing error in microseconds:
        // If measured < expected, actuator is oscillating faster -> resonance f0 is higher -> timing error < 0
        // Phase error in radians: delta_phi = (measured - expected) / expected * PI
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
            self.integral_error = self.integral_error.clamp(-5.0, 5.0);

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

// -----------------------------------------------------------------------------
// Unit Tests & Verification
// -----------------------------------------------------------------------------

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

    // 2. Verify Resonant Frequency Tracking PLL
    // Actuator true mechanical resonance f0 = 220 Hz (e.g. stiffer spring / tight grip).
    // Initial oscillator drive frequency = 200 Hz.
    // True half-period at 220 Hz: 1_000_000 / (2 * 220) = 2272.72 us.
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
    for i in 0..30 {
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
