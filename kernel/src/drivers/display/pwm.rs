//! Hardware PWM Timer Frequency Synthesizer & Duty-Cycle Modulator.
//!
//! Provides precision hardware PWM timer generation for handheld PC display backlights:
//! - Programmable frequencies from 200Hz (legacy low-power) up to 25kHz (ultrasonic anti-flicker mode)
//! - 16-bit duty-cycle resolution (0..65535, corresponding to 0.0% to 100.0%)
//! - Inverted polarity support for active-low LED driver ICs (e.g. LP8556, TPS61165, RT8540)
//! - Glitch-free double-buffered duty cycle and frequency shadow updates

// -----------------------------------------------------------------------------
// Constants and Error Types
// -----------------------------------------------------------------------------

/// Minimum supported PWM frequency in Hz (200 Hz).
pub const PWM_MIN_FREQ_HZ: u32 = 200;

/// Maximum supported PWM frequency in Hz (25 kHz ultrasonic anti-flicker mode).
pub const PWM_MAX_FREQ_HZ: u32 = 25_000;

/// Default handheld SoC PWM timer base clock (24 MHz).
pub const DEFAULT_PWM_BASE_CLOCK_HZ: u64 = 24_000_000;

/// Maximum 16-bit duty cycle value (100.0%).
pub const PWM_DUTY_MAX: u16 = 65535;

/// PWM signal polarity configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PwmPolarity {
    /// Active-high: signal is HIGH during active duty pulse.
    Normal,
    /// Active-low: signal is LOW during active duty pulse (inverted for active-low drivers).
    Inverted,
}

/// Errors occurring during PWM configuration and modulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PwmError {
    FrequencyOutOfRange { requested_hz: u32, min_hz: u32, max_hz: u32 },
    InvalidClockFrequency,
}

// -----------------------------------------------------------------------------
// PWM Timer Synthesizer
// -----------------------------------------------------------------------------

/// Hardware PWM timer frequency synthesizer and prescaler calculator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PwmTimerSynthesizer {
    /// Base input clock frequency feeding the hardware timer block (in Hz).
    pub base_clock_hz: u64,
    /// Target output PWM switching frequency (in Hz).
    pub target_freq_hz: u32,
    /// Hardware timer clock prescaler divisor (1..65536).
    pub prescaler: u32,
    /// Total counter period cycles per PWM wave cycle.
    pub period_cycles: u32,
}

impl PwmTimerSynthesizer {
    /// Synthesizes PWM timer parameters for a given base clock and target frequency.
    pub fn new(base_clock_hz: u64, target_freq_hz: u32) -> Result<Self, PwmError> {
        if base_clock_hz == 0 {
            return Err(PwmError::InvalidClockFrequency);
        }

        if target_freq_hz < PWM_MIN_FREQ_HZ || target_freq_hz > PWM_MAX_FREQ_HZ {
            return Err(PwmError::FrequencyOutOfRange {
                requested_hz: target_freq_hz,
                min_hz: PWM_MIN_FREQ_HZ,
                max_hz: PWM_MAX_FREQ_HZ,
            });
        }

        // Calculate prescaler and period cycles.
        // We want period_cycles to fit comfortably in a 16-bit or 32-bit hardware counter (<= 65535).
        let raw_period = base_clock_hz / (target_freq_hz as u64);
        let mut prescaler = 1u32;
        let mut period_cycles = raw_period as u32;

        while period_cycles > 65535 {
            prescaler = prescaler.saturating_mul(2);
            period_cycles = (base_clock_hz / ((target_freq_hz as u64) * (prescaler as u64))) as u32;
        }

        if period_cycles == 0 {
            period_cycles = 1;
        }

        Ok(Self {
            base_clock_hz,
            target_freq_hz,
            prescaler,
            period_cycles,
        })
    }

    /// Computes the active high cycle count for a given 16-bit duty and polarity.
    pub fn duty_to_active_cycles(&self, duty_16: u16, polarity: PwmPolarity) -> u32 {
        let nominal_cycles = ((self.period_cycles as u64 * duty_16 as u64) / (PWM_DUTY_MAX as u64)) as u32;
        match polarity {
            PwmPolarity::Normal => nominal_cycles,
            PwmPolarity::Inverted => self.period_cycles.saturating_sub(nominal_cycles),
        }
    }

    /// Calculates the period duration in nanoseconds.
    pub fn period_ns(&self) -> u64 {
        1_000_000_000u64 / (self.target_freq_hz as u64)
    }

    /// Calculates the active pulse width in nanoseconds for a given 16-bit duty and polarity.
    pub fn active_pulse_width_ns(&self, duty_16: u16, polarity: PwmPolarity) -> u64 {
        let active_cycles = self.duty_to_active_cycles(duty_16, polarity);
        if self.period_cycles == 0 {
            0
        } else {
            (self.period_ns() * (active_cycles as u64)) / (self.period_cycles as u64)
        }
    }
}

// -----------------------------------------------------------------------------
// PWM Modulator
// -----------------------------------------------------------------------------

/// Hardware PWM channel modulator with double-buffered shadow register updates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PwmModulator {
    /// Hardware PWM channel index (e.g. 0 for display backlight).
    pub channel_id: u8,
    /// Frequency synthesizer configuration.
    pub synthesizer: PwmTimerSynthesizer,
    /// Output signal polarity.
    pub polarity: PwmPolarity,
    /// Modulator hardware enable state.
    pub enabled: bool,
    /// Currently latched hardware duty cycle (0..65535).
    pub active_duty_16: u16,
    /// Double-buffered shadow duty cycle awaiting end-of-period latch.
    pub shadow_duty_16: u16,
    /// Pending synthesizer updates for glitch-free frequency switching.
    pub pending_synthesizer: Option<PwmTimerSynthesizer>,
}

impl PwmModulator {
    /// Creates and initializes a hardware PWM channel.
    pub fn new(
        channel_id: u8,
        base_clock_hz: u64,
        target_freq_hz: u32,
        polarity: PwmPolarity,
    ) -> Result<Self, PwmError> {
        let synthesizer = PwmTimerSynthesizer::new(base_clock_hz, target_freq_hz)?;
        Ok(Self {
            channel_id,
            synthesizer,
            polarity,
            enabled: false,
            active_duty_16: 0,
            shadow_duty_16: 0,
            pending_synthesizer: None,
        })
    }

    /// Stages a 16-bit duty cycle update into the shadow register.
    pub fn set_duty_cycle_16(&mut self, duty: u16) {
        self.shadow_duty_16 = duty;
    }

    /// Stages a floating-point percentage duty cycle update (0.0 to 100.0%).
    pub fn set_duty_percent(&mut self, percent: f32) {
        let clamped = percent.clamp(0.0, 100.0);
        let duty_16 = ((clamped / 100.0) * (PWM_DUTY_MAX as f32) + 0.5) as u16;
        self.set_duty_cycle_16(duty_16);
    }

    /// Stages a new PWM frequency into shadow synthesizer.
    pub fn set_frequency(&mut self, freq_hz: u32) -> Result<(), PwmError> {
        let new_synth = PwmTimerSynthesizer::new(self.synthesizer.base_clock_hz, freq_hz)?;
        self.pending_synthesizer = Some(new_synth);
        Ok(())
    }

    /// Sets the output polarity.
    pub fn set_polarity(&mut self, polarity: PwmPolarity) {
        self.polarity = polarity;
    }

    /// Enables the PWM timer output.
    pub fn enable(&mut self) {
        self.enabled = true;
    }

    /// Disables the PWM timer output.
    pub fn disable(&mut self) {
        self.enabled = false;
    }

    /// Latches shadow registers into active hardware registers at period boundary.
    /// This guarantees glitch-free, monotonic transitions without truncated pulses.
    pub fn commit_shadow_registers(&mut self) {
        self.active_duty_16 = self.shadow_duty_16;
        if let Some(new_synth) = self.pending_synthesizer.take() {
            self.synthesizer = new_synth;
        }
    }

    /// Returns the current active duty cycle as a normalized ratio [0.0..1.0].
    pub fn active_duty_ratio(&self) -> f32 {
        if !self.enabled {
            return 0.0;
        }
        (self.active_duty_16 as f32) / (PWM_DUTY_MAX as f32)
    }

    /// Returns the active pulse width in nanoseconds.
    pub fn active_pulse_width_ns(&self) -> u64 {
        if !self.enabled {
            return 0;
        }
        self.synthesizer.active_pulse_width_ns(self.active_duty_16, self.polarity)
    }

    /// Returns the current hardware cycle count for the active pulse.
    pub fn active_cycles(&self) -> u32 {
        if !self.enabled {
            return 0;
        }
        self.synthesizer.duty_to_active_cycles(self.active_duty_16, self.polarity)
    }
}

// -----------------------------------------------------------------------------
// Unit Tests & Verification
// -----------------------------------------------------------------------------

/// Unit test verifying hardware PWM timer frequency synthesizer and duty-cycle modulator.
pub fn verify_pwm_frequency_and_duty_cycle() -> bool {
    // 1. Verify 24MHz base clock to 25kHz ultrasonic anti-flicker mode
    let synth_25k = match PwmTimerSynthesizer::new(DEFAULT_PWM_BASE_CLOCK_HZ, 25_000) {
        Ok(s) => s,
        Err(_) => return false,
    };
    // 24_000_000 / 25_000 = 960 cycles. Prescaler should be 1.
    if synth_25k.period_cycles != 960 || synth_25k.prescaler != 1 {
        return false;
    }
    // Period for 25kHz is 40,000 ns (40 microseconds)
    if synth_25k.period_ns() != 40_000 {
        return false;
    }

    // 2. Verify 200Hz legacy low-power mode
    let synth_200 = match PwmTimerSynthesizer::new(DEFAULT_PWM_BASE_CLOCK_HZ, 200) {
        Ok(s) => s,
        Err(_) => return false,
    };
    // 24_000_000 / 200 = 120_000 cycles. Needs prescaler >= 2 to fit within 65535.
    if synth_200.prescaler != 2 || synth_200.period_cycles != 60_000 {
        return false;
    }
    // Period for 200Hz is 5,000,000 ns (5 milliseconds)
    if synth_200.period_ns() != 5_000_000 {
        return false;
    }

    // 3. Verify Out-of-Range Frequency Rejection
    if PwmTimerSynthesizer::new(DEFAULT_PWM_BASE_CLOCK_HZ, 100).is_ok() {
        return false; // Below 200 Hz
    }
    if PwmTimerSynthesizer::new(DEFAULT_PWM_BASE_CLOCK_HZ, 30_000).is_ok() {
        return false; // Above 25 kHz
    }

    // 4. Verify 16-Bit Resolution Duty-Cycle Modulator with Normal Polarity
    let mut modulator = match PwmModulator::new(
        0,
        DEFAULT_PWM_BASE_CLOCK_HZ,
        25_000,
        PwmPolarity::Normal,
    ) {
        Ok(m) => m,
        Err(_) => return false,
    };

    modulator.enable();
    // Initially duty is 0
    modulator.commit_shadow_registers();
    if modulator.active_duty_ratio() != 0.0 || modulator.active_pulse_width_ns() != 0 {
        return false;
    }

    // Set 50% duty (32768)
    modulator.set_duty_percent(50.0);
    // Before commit, active duty remains 0
    if modulator.active_cycles() != 0 {
        return false;
    }
    modulator.commit_shadow_registers();
    // At 50%, active cycles should be ~480 (half of 960)
    let cycles_50 = modulator.active_cycles();
    if cycles_50 < 479 || cycles_50 > 481 {
        return false;
    }
    // Pulse width should be ~20,000 ns (20 us)
    let pw_ns = modulator.active_pulse_width_ns();
    if pw_ns < 19_900 || pw_ns > 20_100 {
        return false;
    }

    // 5. Verify Inverted Polarity for Active-Low LED Drivers
    let mut inv_mod = match PwmModulator::new(
        1,
        DEFAULT_PWM_BASE_CLOCK_HZ,
        25_000,
        PwmPolarity::Inverted,
    ) {
        Ok(m) => m,
        Err(_) => return false,
    };
    inv_mod.enable();
    // At 25% duty on Inverted polarity, active low cycles should be (100% - 25%) = 75% of period
    inv_mod.set_duty_percent(25.0);
    inv_mod.commit_shadow_registers();
    // 75% of 960 = 720 cycles
    let inv_cycles = inv_mod.active_cycles();
    if inv_cycles < 718 || inv_cycles > 722 {
        return false;
    }

    true
}
