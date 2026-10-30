//! Kernel-Direct Real-Time In-Game Audio Dynamic Range Compressor & Hearing Safety Limiter.
//!
//! Provides bare-metal low-latency dynamic range control (DRC) for handheld game audio:
//! - Configurable soft-knee compression curve with variable threshold, ratio, and makeup gain.
//! - Sub-millisecond lookahead buffer preventing transient overshoot.
//! - Branching peak/RMS envelope detector with sub-millisecond attack and smooth release.
//! - Hearing safety brickwall true-peak limiter clamping sound at -1.0 dBFS ceiling.
//! - Footstep and ambient whisper boost mode for tactical esports gameplay.

#![allow(dead_code)]

extern crate alloc;

use crate::audio::hrtf::AUDIO_SAMPLE_RATE;
use crate::audio::voice_dsp::log10_f32;

// -----------------------------------------------------------------------------
// Pure no_std Floating Point Math Helpers
// -----------------------------------------------------------------------------

/// Computes e^x using range reduction and 6th-order Taylor polynomial.
/// Valid across [-80.0, 80.0].
pub fn exp_f32(x: f32) -> f32 {
    if x <= -80.0 {
        return 0.0;
    }
    if x >= 80.0 {
        return f32::MAX;
    }

    // x = k * ln(2) + r, where ln(2) ~= 0.69314718
    let inv_ln2 = 1.44269504;
    let k = (x * inv_ln2 + (if x >= 0.0 { 0.5 } else { -0.5 })) as i32;
    let r = x - (k as f32) * 0.69314718;

    // Taylor series for e^r: 1 + r + r^2/2 + r^3/6 + r^4/24 + r^5/120 + r^6/720
    let r2 = r * r;
    let r3 = r2 * r;
    let r4 = r2 * r2;
    let r5 = r4 * r;
    let r6 = r3 * r3;

    let poly = 1.0 + r + (r2 * 0.5) + (r3 * 0.16666667) + (r4 * 0.041666668)
        + (r5 * 0.008333334) + (r6 * 0.001388889);

    // Multiply by 2^k using IEEE-754 bit manipulation
    let exp_bits = ((k + 127).clamp(1, 254) as u32) << 23;
    let scale = f32::from_bits(exp_bits);

    poly * scale
}



/// Converts linear amplitude to decibels full scale (dBFS).
pub fn linear_to_db(lin: f32) -> f32 {
    let abs_val = if lin < 0.0 { -lin } else { lin };
    if abs_val <= 1e-6 {
        -120.0
    } else {
        20.0 * log10_f32(abs_val)
    }
}

/// Converts decibels (dBFS) to linear amplitude ratio.
pub fn db_to_linear(db: f32) -> f32 {
    if db <= -120.0 {
        0.0
    } else {
        exp_f32(db * 0.11512925)
    }
}

// -----------------------------------------------------------------------------
// 1. Dynamic Range Compressor Parameters & Gain Curve
// -----------------------------------------------------------------------------

/// Parameterizable configuration for dynamic range compression and peak limiting.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct CompressorParameters {
    /// Threshold level in dBFS above which compression begins (-60.0 to 0.0 dB).
    pub threshold_db: f32,
    /// Compression ratio R (e.g. 4.0 for 4:1, 20.0 for 20:1, up to 100.0).
    pub ratio: f32,
    /// Soft-knee width in dB (0.0 for sharp hard knee, 1.0 to 12.0 for gradual transition).
    pub knee_width_db: f32,
    /// Attack time constant in milliseconds (0.1 ms to 50.0 ms).
    pub attack_time_ms: f32,
    /// Release time constant in milliseconds (5.0 ms to 500.0 ms).
    pub release_time_ms: f32,
    /// Post-compression makeup gain boost in dB (0.0 to 24.0 dB).
    pub makeup_gain_db: f32,
    /// System audio sample rate in Hz.
    pub sample_rate: f32,
}

impl CompressorParameters {
    /// Balanced dynamic range compression preset for handheld gaming.
    pub const fn default_gaming() -> Self {
        Self {
            threshold_db: -18.0,
            ratio: 4.0,
            knee_width_db: 6.0,
            attack_time_ms: 5.0,
            release_time_ms: 100.0,
            makeup_gain_db: 3.0,
            sample_rate: AUDIO_SAMPLE_RATE,
        }
    }

    /// Esports Footstep / Quiet Detail Booster preset.
    /// Lowers threshold and expands subtle sounds below -30 dBFS while controlling loud explosions.
    pub const fn esports_footstep_boost() -> Self {
        Self {
            threshold_db: -28.0,
            ratio: 3.5,
            knee_width_db: 8.0,
            attack_time_ms: 2.0,
            release_time_ms: 80.0,
            makeup_gain_db: 8.0,
            sample_rate: AUDIO_SAMPLE_RATE,
        }
    }

    /// Hearing Safety Brickwall Limiter preset.
    /// Fast attack, infinite ratio, clamping ear-damaging transients to -1.0 dBFS.
    pub const fn hearing_safety_limiter() -> Self {
        Self {
            threshold_db: -1.0,
            ratio: 50.0,
            knee_width_db: 0.5,
            attack_time_ms: 0.2,
            release_time_ms: 50.0,
            makeup_gain_db: 0.0,
            sample_rate: AUDIO_SAMPLE_RATE,
        }
    }

    /// Evaluates the static soft-knee compression characteristic:
    /// Maps input level in dBFS to compressed output level in dBFS.
    pub fn calculate_static_output_db(&self, input_db: f32) -> f32 {
        let t = self.threshold_db;
        let r = if self.ratio < 1.0 { 1.0 } else { self.ratio };
        let w = if self.knee_width_db < 0.0 { 0.0 } else { self.knee_width_db };

        if w <= 0.0 {
            // Hard knee
            if input_db <= t {
                input_db
            } else {
                t + (input_db - t) / r
            }
        } else {
            let half_w = w * 0.5;
            if input_db < (t - half_w) {
                // Below knee: linear 1:1 response
                input_db
            } else if input_db > (t + half_w) {
                // Above knee: full ratio compression
                t + (input_db - t) / r
            } else {
                // Inside knee region: quadratic Hermite spline interpolation
                let delta = input_db - t + half_w;
                input_db + ((1.0 / r - 1.0) * delta * delta) / (2.0 * w)
            }
        }
    }

    /// Calculates static gain reduction in dB:
    /// GR(dB) = Output(dB) - Input(dB) <= 0.0 dB.
    pub fn calculate_gain_reduction_db(&self, input_db: f32) -> f32 {
        let output_db = self.calculate_static_output_db(input_db);
        output_db - input_db
    }
}

// -----------------------------------------------------------------------------
// 2. Envelope Detection State & Smoothing Filter
// -----------------------------------------------------------------------------

/// Branching peak envelope detector with decoupled attack and release time constants.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnvelopeDetector {
    pub attack_coeff: f32,
    pub release_coeff: f32,
    pub envelope_db: f32,
}

impl EnvelopeDetector {
    /// Creates a new envelope detector initialized to complete silence (-120 dBFS).
    pub fn new(sample_rate: f32, attack_ms: f32, release_ms: f32) -> Self {
        let mut detector = Self {
            attack_coeff: 0.0,
            release_coeff: 0.0,
            envelope_db: -120.0,
        };
        detector.update_coefficients(sample_rate, attack_ms, release_ms);
        detector
    }

    /// Recalculates one-pole smoothing coefficients:
    /// coeff = exp(-1.0 / (sample_rate * (time_ms / 1000.0)))
    pub fn update_coefficients(&mut self, sample_rate: f32, attack_ms: f32, release_ms: f32) {
        let safe_sr = if sample_rate <= 0.0 { AUDIO_SAMPLE_RATE } else { sample_rate };
        let safe_att = if attack_ms < 0.05 { 0.05 } else { attack_ms };
        let safe_rel = if release_ms < 1.0 { 1.0 } else { release_ms };

        let att_samples = safe_sr * (safe_att * 0.001);
        let rel_samples = safe_sr * (safe_rel * 0.001);

        self.attack_coeff = exp_f32(-1.0 / att_samples);
        self.release_coeff = exp_f32(-1.0 / rel_samples);
    }

    /// Resets envelope detector state.
    pub fn reset(&mut self) {
        self.envelope_db = -120.0;
    }

    /// Advances envelope detector with target level in dBFS and returns smoothed envelope value.
    pub fn step(&mut self, target_db: f32) -> f32 {
        if target_db > self.envelope_db {
            // Attack phase: signal is rising
            self.envelope_db = self.attack_coeff * self.envelope_db + (1.0 - self.attack_coeff) * target_db;
        } else {
            // Release phase: signal is decaying
            self.envelope_db = self.release_coeff * self.envelope_db + (1.0 - self.release_coeff) * target_db;
        }
        self.envelope_db
    }
}

// -----------------------------------------------------------------------------
// Unit Tests for Commit 1
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn test_math_helpers_and_conversions() {
        // Test exponential
        let e0 = exp_f32(0.0);
        assert!((e0 - 1.0).abs() < 1e-4);

        let e1 = exp_f32(1.0);
        assert!((e1 - 2.71828).abs() < 1e-3);

        // Test log and dB conversion
        let lin = 1.0;
        let db0 = linear_to_db(lin);
        assert!((db0 - 0.0).abs() < 1e-3);

        let half_amp = 0.5;
        let db_half = linear_to_db(half_amp);
        // 20 * log10(0.5) ~= -6.02 dB
        assert!((db_half - (-6.0206)).abs() < 0.05);

        let restored_half = db_to_linear(db_half);
        assert!((restored_half - 0.5).abs() < 1e-3);
    }

    #[test]
    pub fn test_compressor_static_curve() {
        let params = CompressorParameters::default_gaming();
        assert_eq!(params.threshold_db, -18.0);
        assert_eq!(params.ratio, 4.0);

        // Well below threshold (e.g. -30 dB): no compression, gain reduction = 0 dB
        let gr_low = params.calculate_gain_reduction_db(-30.0);
        assert!((gr_low - 0.0).abs() < 1e-3);

        // Well above threshold (e.g. -6 dB): input is 12 dB above threshold
        // Compressed output = -18 + 12 / 4 = -15 dB
        // Gain reduction = -15 - (-6) = -9 dB
        let gr_high = params.calculate_gain_reduction_db(-6.0);
        assert!((gr_high - (-9.0)).abs() < 0.1);

        // Test envelope detector
        let mut env = EnvelopeDetector::new(48000.0, 5.0, 100.0);
        for _ in 0..500 {
            env.step(0.0);
        }
        assert!(env.envelope_db > -1.0);
    }
}
