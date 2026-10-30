//! Vibration waveform synthesizer and ADSR envelope generator for gamepad haptics.

// Waveform types and frequency bounds.

/// Supported haptic vibration waveform profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaveformType {
    /// Pure periodic sine wave (smooth, low-frequency hum).
    Sine,
    /// Sharp square wave (harsh, mechanical vibration).
    Square,
    /// Linear rising/falling triangle wave (tactile click or progressive ramp).
    Triangle,
    /// Pulsed envelope (rapid intermittent haptic bursts).
    PulsedEnvelope { pulse_period_ms: u16, duty_cycle_percent: u8 },
    /// Exponential-like decay burst (gunshot / impact recoil).
    DecayBurst,
}

/// Frequency limits supported by gamepad ERM and linear resonant actuators (LRA).
pub const HAPTIC_MIN_FREQUENCY_HZ: u16 = 10;
pub const HAPTIC_MAX_FREQUENCY_HZ: u16 = 500;
pub const HAPTIC_DEFAULT_SAMPLE_RATE_HZ: u32 = 1000; // 1 ms evaluation step

// ADSR envelope configuration.

/// Attack-Decay-Sustain-Release amplitude envelope parameters in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdsrEnvelope {
    /// Attack duration in milliseconds (ramp up from 0 to peak).
    pub attack_ms: u16,
    /// Decay duration in milliseconds (ramp down from peak to sustain level).
    pub decay_ms: u16,
    /// Sustain amplitude level (0 to 255).
    pub sustain_level: u8,
    /// Release duration in milliseconds (ramp down from sustain level to 0).
    pub release_ms: u16,
}

impl AdsrEnvelope {
    pub const fn new(attack_ms: u16, decay_ms: u16, sustain_level: u8, release_ms: u16) -> Self {
        Self {
            attack_ms,
            decay_ms,
            sustain_level,
            release_ms,
        }
    }

    /// Default snappy impact preset (e.g. gun recoil, button click).
    pub const fn impact() -> Self {
        Self {
            attack_ms: 5,
            decay_ms: 25,
            sustain_level: 60,
            release_ms: 30,
        }
    }

    /// Smooth sustained rumble preset (e.g. driving on gravel, engine revving).
    pub const fn continuous() -> Self {
        Self {
            attack_ms: 50,
            decay_ms: 50,
            sustain_level: 200,
            release_ms: 100,
        }
    }

    /// Evaluates the ADSR amplitude multiplier (0 to 255) at a given elapsed time `t_ms`
    /// for an effect with total active duration `total_duration_ms`.
    pub fn evaluate(&self, t_ms: u16, total_duration_ms: u16) -> u8 {
        if t_ms >= total_duration_ms {
            return 0;
        }

        // Release phase occurs during the last `release_ms` of total_duration_ms
        let release_start_ms = total_duration_ms.saturating_sub(self.release_ms);
        if t_ms >= release_start_ms && self.release_ms > 0 {
            let release_elapsed = t_ms - release_start_ms;
            let factor = ((self.release_ms.saturating_sub(release_elapsed)) as u32 * 255) / (self.release_ms as u32);
            return ((self.sustain_level as u32 * factor) / 255).min(255) as u8;
        }

        // 1. Attack phase
        if t_ms < self.attack_ms && self.attack_ms > 0 {
            let ramp = (t_ms as u32 * 255) / (self.attack_ms as u32);
            return ramp.min(255) as u8;
        }

        // 2. Decay phase
        let decay_start = self.attack_ms;
        let decay_end = self.attack_ms + self.decay_ms;
        if t_ms < decay_end && self.decay_ms > 0 {
            let elapsed_decay = t_ms - decay_start;
            let delta = 255u32.saturating_sub(self.sustain_level as u32);
            let drop = (elapsed_decay as u32 * delta) / (self.decay_ms as u32);
            return (255u32.saturating_sub(drop)).min(255) as u8;
        }

        // 3. Sustain phase
        self.sustain_level
    }
}

// Vibration waveform synthesizer.

/// Integer-based lookup table for sin(x) across 0..=90 degrees (quarter cycle, 0..255 range).
/// 16 samples covering quarter phase.
const SINE_QUARTER_TABLE: [u8; 17] = [
    0, 25, 49, 74, 97, 120, 141, 161, 180, 197, 212, 225, 236, 244, 250, 254, 255,
];

/// Computes integer sine approximation yielding normalized 0..=255 amplitude.
pub fn integer_sine_u8(phase_256: u8) -> u8 {
    // phase in 0..256 corresponds to 0..2pi
    // Quadrant 0: 0..64
    // Quadrant 1: 64..128
    // Quadrant 2: 128..192
    // Quadrant 3: 192..256
    let quad = phase_256 / 64;
    let rem = phase_256 % 64;

    let quarter_sample = match quad {
        0 => (rem * 16) / 64,
        1 => ((64 - rem) * 16) / 64,
        2 => (rem * 16) / 64,
        _ => ((64 - rem) * 16) / 64,
    } as usize;

    let magnitude = SINE_QUARTER_TABLE[quarter_sample.min(16)];
    if quad < 2 {
        // Positive half-wave: 128 + (magnitude / 2)
        128u8.saturating_add(magnitude / 2)
    } else {
        // Negative half-wave: 128 - (magnitude / 2)
        128u8.saturating_sub(magnitude / 2)
    }
}

/// Waveform synthesis generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaveformSynthesizer {
    pub waveform: WaveformType,
    pub frequency_hz: u16,
    pub envelope: AdsrEnvelope,
    pub total_duration_ms: u16,
    pub gain: u8, // Overall volume/gain scale (0 to 255)
}

impl WaveformSynthesizer {
    pub fn new(
        waveform: WaveformType,
        frequency_hz: u16,
        envelope: AdsrEnvelope,
        total_duration_ms: u16,
        gain: u8,
    ) -> Self {
        Self {
            waveform,
            frequency_hz: frequency_hz.clamp(HAPTIC_MIN_FREQUENCY_HZ, HAPTIC_MAX_FREQUENCY_HZ),
            envelope,
            total_duration_ms,
            gain,
        }
    }

    /// Evaluates raw base waveform amplitude at given phase (0..255).
    pub fn evaluate_base_wave(&self, t_ms: u16) -> u8 {
        if self.frequency_hz == 0 {
            return 0;
        }

        // Calculate phase in [0..255]
        let period_ms = (1000 / self.frequency_hz as u32).max(1);
        let phase = (((t_ms as u32 % period_ms) * 256) / period_ms).min(255) as u8;

        match self.waveform {
            WaveformType::Sine => integer_sine_u8(phase),
            WaveformType::Square => {
                if phase < 128 {
                    255
                } else {
                    0
                }
            }
            WaveformType::Triangle => {
                if phase < 128 {
                    (phase as u16 * 2).min(255) as u8
                } else {
                    ((255 - phase as u16) * 2).min(255) as u8
                }
            }
            WaveformType::PulsedEnvelope { pulse_period_ms, duty_cycle_percent } => {
                let p = pulse_period_ms.max(1) as u32;
                let duty_threshold = (p * duty_cycle_percent.min(100) as u32) / 100;
                let rem = (t_ms as u32) % p;
                if rem < duty_threshold {
                    255
                } else {
                    0
                }
            }
            WaveformType::DecayBurst => {
                let decay_step = ((self.total_duration_ms.saturating_sub(t_ms)) as u32 * 255)
                    / (self.total_duration_ms.max(1) as u32);
                decay_step.min(255) as u8
            }
        }
    }

    /// Evaluates full synthesized vibration motor output (0..255) for time `t_ms`.
    pub fn sample_amplitude(&self, t_ms: u16) -> u8 {
        if t_ms >= self.total_duration_ms || self.gain == 0 {
            return 0;
        }

        let base = self.evaluate_base_wave(t_ms) as u32;
        let env = self.envelope.evaluate(t_ms, self.total_duration_ms) as u32;
        let scaled = (base * env) / 255;
        let final_out = (scaled * self.gain as u32) / 255;

        final_out.min(255) as u8
    }
}

// Unit tests and verification.

/// Unit test verifying vibration waveform synthesis, ADSR envelopes, and gain scaling.
pub fn verify_waveform_synthesizer() -> bool {
    // 1. Verify ADSR Envelope calculation
    let adsr = AdsrEnvelope::new(10, 20, 128, 20);
    // At t=0 ms (attack beginning) => 0
    if adsr.evaluate(0, 100) != 0 {
        return false;
    }
    // At t=10 ms (attack peak) => 255
    if adsr.evaluate(10, 100) != 255 {
        return false;
    }
    // At t=30 ms (decay ended, sustain entered) => 128
    if adsr.evaluate(30, 100) != 128 {
        return false;
    }
    // At t=70 ms (sustain holding) => 128
    if adsr.evaluate(70, 100) != 128 {
        return false;
    }
    // At t=90 ms (release midway, release_start = 80ms) => ~64
    let rel_val = adsr.evaluate(90, 100);
    if !(60..=70).contains(&rel_val) {
        return false;
    }
    // At t=100 ms (completed) => 0
    if adsr.evaluate(100, 100) != 0 {
        return false;
    }

    // 2. Verify Square Waveform Synthesizer
    let synth = WaveformSynthesizer::new(WaveformType::Square, 50, AdsrEnvelope::continuous(), 200, 255);
    // Frequency 50Hz => period 20ms. First half (0..10ms) should be non-zero
    let sample_hi = synth.sample_amplitude(5);
    if sample_hi == 0 {
        return false;
    }

    // 3. Verify Gain scaling
    let half_gain = WaveformSynthesizer::new(WaveformType::Square, 50, AdsrEnvelope::continuous(), 200, 128);
    let sample_half = half_gain.sample_amplitude(5);
    if sample_half >= sample_hi {
        return false;
    }

    // 4. Verify Decay Burst
    let burst = WaveformSynthesizer::new(WaveformType::DecayBurst, 100, AdsrEnvelope::new(0, 0, 255, 0), 50, 255);
    let b0 = burst.sample_amplitude(0);
    let b25 = burst.sample_amplitude(25);
    let b49 = burst.sample_amplitude(49);
    if b0 <= b25 || b25 <= b49 {
        return false;
    }

    true
}
