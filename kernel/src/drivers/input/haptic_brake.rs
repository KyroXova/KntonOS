//! Dynamic Active Reverse-Phase Braking Pulse Synthesizer for Sub-5ms Haptic Cutoff.
//!
//! Eliminates mechanical ringing ("buzz trailing") in high-Q Linear Resonant Actuators (LRAs):
//! - Injects a 180° phase-inverted electromagnetic damping pulse immediately upon effect termination
//! - Actively decelerates the oscillating magnetic mass within a single half-cycle (sub-5ms cutoff)
//! - Automatically transitions from active reverse braking to low-impedance coil shorting (dynamic quenching)

// -----------------------------------------------------------------------------
// Braking State Machine & Parameters
// -----------------------------------------------------------------------------

/// Operating state of the active braking pulse synthesizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrakingState {
    /// Actuator is in normal drive or idle (no braking pulse active).
    Idle,
    /// Active reverse-phase (180° inverted) electromagnetic damping pulse in progress.
    ActiveBraking,
    /// Damping pulse complete; coil is quenched (shorted to ground for passive back-EMF absorption).
    Quenched,
}

/// Damping mechanism mode configured for haptic termination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DampingMode {
    /// Active 180° reverse-phase overdrive pulse followed by coil quenching (fastest, sub-5ms).
    ActiveReversePhase,
    /// Passive low-impedance coil shorting without active reverse drive (~15-20ms).
    PassiveCoilShort,
    /// High-impedance natural mechanical coasting (slowest, 40-80ms buzz trail).
    Coasting,
}

/// Configuration parameters for active dynamic braking.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrakingConfig {
    /// Damping operational strategy.
    pub damping_mode: DampingMode,
    /// Reverse-phase braking amplitude multiplier relative to terminating drive amplitude (e.g. 1.0x - 1.25x).
    pub overdrive_multiplier: f32,
    /// Braking pulse duration as a fraction of the resonant period (default 0.50 = 180° half-cycle).
    pub pulse_duration_cycles: f32,
    /// Quench hold time in microseconds after active braking completes (e.g. 2000 us = 2ms).
    pub quench_hold_us: u32,
}

impl BrakingConfig {
    pub const fn default_handheld() -> Self {
        Self {
            damping_mode: DampingMode::ActiveReversePhase,
            overdrive_multiplier: 1.15,
            pulse_duration_cycles: 0.50, // Exactly 180° inverse half-cycle
            quench_hold_us: 2000,
        }
    }
}

/// Instantaneous drive output emitted by the braking synthesizer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrakingOutput {
    /// Output drive amplitude [0..255].
    pub amplitude: u8,
    /// Phase inversion angle in degrees (180.0° during active reverse phase).
    pub phase_inversion_deg: f32,
    /// Current braking state.
    pub state: BrakingState,
    /// Whether the voice coil is currently shorted for dynamic quenching.
    pub coil_shorted: bool,
}

impl BrakingOutput {
    pub const fn idle() -> Self {
        Self {
            amplitude: 0,
            phase_inversion_deg: 0.0,
            state: BrakingState::Idle,
            coil_shorted: false,
        }
    }
}

// -----------------------------------------------------------------------------
// Braking Pulse Synthesizer
// -----------------------------------------------------------------------------

/// Dynamic active reverse-phase braking pulse synthesizer for a single LRA actuator.
#[derive(Debug, Clone)]
pub struct BrakingPulseSynthesizer {
    /// Synthesizer configuration.
    pub config: BrakingConfig,
    /// Current braking state.
    pub state: BrakingState,
    /// Resonant frequency of the actuator being braked (in Hz).
    pub resonant_freq_hz: f32,
    /// Calculated total active reverse braking duration in microseconds.
    pub active_brake_duration_us: u32,
    /// Elapsed time in current braking state in microseconds.
    pub elapsed_in_state_us: u32,
    /// Commanded reverse braking amplitude [0..255].
    pub brake_amplitude: u8,
    /// Total successful brake quenches executed.
    pub quenches_completed: u32,
}

impl BrakingPulseSynthesizer {
    /// Creates a new braking synthesizer with the specified configuration.
    pub fn new(config: BrakingConfig) -> Self {
        Self {
            config,
            state: BrakingState::Idle,
            resonant_freq_hz: 200.0,
            active_brake_duration_us: 0,
            elapsed_in_state_us: 0,
            brake_amplitude: 0,
            quenches_completed: 0,
        }
    }

    /// Triggers an active reverse-phase braking pulse upon effect cutoff.
    ///
    /// Evaluates the actuator's resonant frequency $f_0$ and terminating vibration amplitude,
    /// computing an exact half-cycle ($180^\circ$) reverse pulse:
    ///   $T_{\text{brake}} = \frac{10^6 \cdot \text{cycles}}{f_0} \text{ microseconds}$
    pub fn trigger_braking(&mut self, terminating_amplitude: u8, resonant_freq_hz: f32) -> BrakingOutput {
        if terminating_amplitude == 0 || resonant_freq_hz <= 0.0 {
            self.state = BrakingState::Idle;
            return BrakingOutput::idle();
        }

        self.resonant_freq_hz = resonant_freq_hz.clamp(150.0, 250.0);
        self.elapsed_in_state_us = 0;

        match self.config.damping_mode {
            DampingMode::ActiveReversePhase => {
                // Calculate half-period in microseconds: 1_000_000 * 0.5 / f0 = 500_000 / f0
                let half_period_us = ((1_000_000.0 * self.config.pulse_duration_cycles) / self.resonant_freq_hz) as u32;
                self.active_brake_duration_us = half_period_us;

                // Scale overdrive braking amplitude
                let target_amp = (terminating_amplitude as f32) * self.config.overdrive_multiplier + 0.5;
                self.brake_amplitude = (target_amp as u8).min(255);
                self.state = BrakingState::ActiveBraking;

                BrakingOutput {
                    amplitude: self.brake_amplitude,
                    phase_inversion_deg: 180.0,
                    state: BrakingState::ActiveBraking,
                    coil_shorted: false,
                }
            }
            DampingMode::PassiveCoilShort => {
                self.state = BrakingState::Quenched;
                self.brake_amplitude = 0;
                BrakingOutput {
                    amplitude: 0,
                    phase_inversion_deg: 0.0,
                    state: BrakingState::Quenched,
                    coil_shorted: true,
                }
            }
            DampingMode::Coasting => {
                self.state = BrakingState::Idle;
                self.brake_amplitude = 0;
                BrakingOutput::idle()
            }
        }
    }

    /// Advances the braking timeline by `dt_us` microseconds and evaluates the braking drive output.
    pub fn step(&mut self, dt_us: u32) -> BrakingOutput {
        match self.state {
            BrakingState::Idle => BrakingOutput::idle(),
            BrakingState::ActiveBraking => {
                self.elapsed_in_state_us += dt_us;
                if self.elapsed_in_state_us >= self.active_brake_duration_us {
                    // Active reverse pulse complete; transition to quenched (coil shorted)
                    self.state = BrakingState::Quenched;
                    self.elapsed_in_state_us = 0;
                    self.brake_amplitude = 0;

                    BrakingOutput {
                        amplitude: 0,
                        phase_inversion_deg: 0.0,
                        state: BrakingState::Quenched,
                        coil_shorted: true,
                    }
                } else {
                    // Continue active reverse-phase pulse
                    BrakingOutput {
                        amplitude: self.brake_amplitude,
                        phase_inversion_deg: 180.0,
                        state: BrakingState::ActiveBraking,
                        coil_shorted: false,
                    }
                }
            }
            BrakingState::Quenched => {
                self.elapsed_in_state_us += dt_us;
                if self.elapsed_in_state_us >= self.config.quench_hold_us {
                    // Quench hold elapsed; return to idle
                    self.state = BrakingState::Idle;
                    self.elapsed_in_state_us = 0;
                    self.quenches_completed += 1;
                    BrakingOutput::idle()
                } else {
                    BrakingOutput {
                        amplitude: 0,
                        phase_inversion_deg: 0.0,
                        state: BrakingState::Quenched,
                        coil_shorted: true,
                    }
                }
            }
        }
    }

    /// Returns true if currently actively decelerating or quenching the actuator.
    pub fn is_braking_active(&self) -> bool {
        self.state != BrakingState::Idle
    }

    /// Resets the synthesizer to idle.
    pub fn reset(&mut self) {
        self.state = BrakingState::Idle;
        self.elapsed_in_state_us = 0;
        self.brake_amplitude = 0;
    }
}

// -----------------------------------------------------------------------------
// Unit Tests & Verification
// -----------------------------------------------------------------------------

/// Unit test verifying dynamic active reverse-phase braking pulse synthesizer for sub-5ms haptic cutoff.
pub fn verify_active_reverse_phase_braking() -> bool {
    let config = BrakingConfig::default_handheld();
    let mut synth = BrakingPulseSynthesizer::new(config);

    if synth.is_braking_active() {
        return false;
    }

    // 1. Trigger Braking for Palm LRA at 175 Hz (terminating amplitude = 200)
    // Resonant period T = 1_000_000 / 175 = 5714 us
    // Half-cycle pulse duration = 2857 us (< 3.0 ms, well under 5ms cutoff requirement)
    let initial_out = synth.trigger_braking(200, 175.0);
    if initial_out.state != BrakingState::ActiveBraking {
        return false;
    }
    if initial_out.phase_inversion_deg != 180.0 {
        return false;
    }
    // Overdrive multiplier is 1.15 -> 200 * 1.15 = 230
    if initial_out.amplitude != 230 {
        return false;
    }
    // Active brake duration must be strictly sub-5ms (< 5000 us)
    if synth.active_brake_duration_us >= 5000 || synth.active_brake_duration_us < 2000 {
        return false;
    }

    // 2. Step through active braking: 1000 us step
    let step1 = synth.step(1000);
    if step1.state != BrakingState::ActiveBraking || step1.phase_inversion_deg != 180.0 {
        return false;
    }
    if step1.amplitude != 230 {
        return false;
    }

    // Advance past active brake duration (another 2000 us -> total 3000 us >= 2857 us)
    let step2 = synth.step(2000);
    // Must immediately transition to Quenched state with coil shorted
    if step2.state != BrakingState::Quenched || !step2.coil_shorted || step2.amplitude != 0 {
        return false;
    }

    // Advance quench hold duration (2000 us)
    let step3 = synth.step(2000);
    // Must transition to Idle
    if step3.state != BrakingState::Idle || synth.is_braking_active() {
        return false;
    }
    if synth.quenches_completed != 1 {
        return false;
    }

    // 3. Trigger Braking for Trigger Micro-LRA at 235 Hz
    // T_half = 500_000 / 235 = 2127 us (2.13 ms)
    let trig_out = synth.trigger_braking(255, 235.0);
    if trig_out.state != BrakingState::ActiveBraking {
        return false;
    }
    // 255 * 1.15 = clamped to 255
    if trig_out.amplitude != 255 {
        return false;
    }
    if (synth.active_brake_duration_us as i32 - 2127).abs() > 5 {
        return false;
    }
    // Step 2200 us -> Quenched
    let trig_step = synth.step(2200);
    if trig_step.state != BrakingState::Quenched {
        return false;
    }

    true
}
