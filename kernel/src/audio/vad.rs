//! Kernel-Direct Ultra-Low-Latency In-Game Voice Activity Detector (VAD) & Spatial Echo Canceller.
//!
//! Provides bare-metal low-latency voice communications DSP for gaming handhelds:
//! - Sub-5ms low-latency frame analysis (240 samples at 48 kHz).
//! - Multi-feature Voice Activity Detection (VAD) combining energy, zero-crossing rate (ZCR),
//!   spectral flatness, and sub-band vocal formant concentration.
//! - Dynamic background noise estimator tracking chassis cooling fan rumble and button click rejection.
//! - Speech hangover hysteresis timer preventing chopped word endings.
//! - Normalized Least Mean Squares (NLMS) Adaptive Acoustic Echo Canceller (AEC) subtracting
//!   speaker audio from microphone capture.
//! - Geigel-style cross-correlation Dual-Talk Detector (DTD) preventing filter divergence during simultaneous speech.
//! - Zero-allocation 16.16 fixed-point arithmetic execution.

#![allow(dead_code)]

extern crate alloc;

// -----------------------------------------------------------------------------
// Voice Activity Detection & Echo Cancellation Constants
// -----------------------------------------------------------------------------

/// Voice chat sampling frequency in Hertz (48 kHz standard).
pub const VAD_SAMPLE_RATE_HZ: u32 = 48000;

/// Number of 16-bit PCM audio samples per 5-millisecond analysis frame (240 samples).
pub const VAD_FRAME_SAMPLES: usize = 240;

/// Number of filter taps for the NLMS acoustic echo canceler (128 taps = 2.67 ms direct path coupling).
pub const NLMS_FILTER_TAPS: usize = 128;

/// Fixed-point scale factor for 16.16 arithmetic ($1.0 = 65536$).
pub const FP_Q16_ONE: i32 = 65536;

/// Default speech hangover timer duration in frames (10 frames = 50 ms at 5 ms/frame).
pub const DEFAULT_HANGOVER_FRAMES: u16 = 10;

// -----------------------------------------------------------------------------
// 16.16 Fixed-Point Math Helpers
// -----------------------------------------------------------------------------

/// Fixed-point 16.16 multiplication with rounding.
#[inline(always)]
pub fn fp_mul_q16(a: i32, b: i32) -> i32 {
    let prod = (a as i64) * (b as i64) + 32768;
    (prod >> 16) as i32
}

/// Fixed-point 16.16 division with overflow protection.
#[inline(always)]
pub fn fp_div_q16(numerator: i32, denominator: i32) -> i32 {
    if denominator == 0 {
        return if numerator >= 0 { i32::MAX } else { i32::MIN };
    }
    let num_scaled = (numerator as i64) << 16;
    (num_scaled / (denominator as i64)) as i32
}

/// Fixed-point 16.16 integer square root approximation.
pub fn fp_sqrt_q16(val: i32) -> i32 {
    if val <= 0 {
        return 0;
    }
    let mut x = val;
    let mut y = (x + FP_Q16_ONE) >> 1;
    while y < x {
        x = y;
        y = (x + fp_div_q16(val, x)) >> 1;
    }
    x
}

// -----------------------------------------------------------------------------
// Spectral Features & VAD State Structures
// -----------------------------------------------------------------------------

/// Primary acoustic feature set extracted from each 5ms voice frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VadSpectralFeatures {
    /// Total root-mean-square (RMS) energy in 16.16 fixed-point format.
    pub frame_energy_q16: i32,
    /// Number of zero-crossing sign inversions across the frame.
    pub zero_crossing_count: u16,
    /// Spectral flatness measure in 16.16 fixed point [0.0 = voiced tonal peaks, 1.0 = flat noise].
    pub spectral_flatness_q16: i32,
    /// Low-band vocal formant energy (100 Hz - 1000 Hz) in 16.16 fixed point.
    pub low_band_energy_q16: i32,
    /// High-band energy (2000 Hz - 8000 Hz) capturing sibilance, fan noise, or clicks.
    pub high_band_energy_q16: i32,
    /// Energy variance / periodicity indicator distinguishing voice modulation from stationary noise.
    pub energy_variance_q16: i32,
}

impl VadSpectralFeatures {
    pub const fn empty() -> Self {
        Self {
            frame_energy_q16: 0,
            zero_crossing_count: 0,
            spectral_flatness_q16: 0,
            low_band_energy_q16: 0,
            high_band_energy_q16: 0,
            energy_variance_q16: 0,
        }
    }
}

/// Voice Activity classification decision.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadDecision {
    /// Silence or sub-threshold background.
    Silence = 0,
    /// Active human voice speech detected.
    Speech = 1,
    /// Transient noise rejected (fan whir spike or mechanical button click).
    ChassisNoise = 2,
}

/// Internal state tracker for voice activity detection and hangover hysteresis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VadState {
    /// Current classification decision.
    pub decision: VadDecision,
    /// Voice activity confidence score in 16.16 fixed-point [0.0 = 0, 1.0 = 65536].
    pub speech_probability_q16: i32,
    /// Number of consecutive speech frames observed.
    pub consecutive_speech_frames: u16,
    /// Remaining hangover countdown in frames preventing speech chopping.
    pub hangover_counter: u16,
    /// Configured hangover duration in frames.
    pub hangover_duration_frames: u16,
}

impl VadState {
    pub const fn new(hangover_duration_frames: u16) -> Self {
        Self {
            decision: VadDecision::Silence,
            speech_probability_q16: 0,
            consecutive_speech_frames: 0,
            hangover_counter: 0,
            hangover_duration_frames,
        }
    }
}

/// Dynamic noise floor estimator tracking fan noise and ambient room acoustics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoiseFloorEstimator {
    /// Slowly tracked noise floor energy in 16.16 fixed point.
    pub noise_energy_q16: i32,
    /// Short-term minimum energy tracking baseline.
    pub min_energy_q16: i32,
    /// Estimated Signal-to-Noise Ratio (SNR) in 16.16 fixed-point dB.
    pub snr_q16: i32,
    /// Noise floor upward adaptation coefficient in Q16 (~0.02).
    pub alpha_rise_q16: i32,
    /// Noise floor downward adaptation coefficient in Q16 (~0.10).
    pub alpha_fall_q16: i32,
    /// Chassis fan RPM coupling factor in Q16 (modulates threshold under high fan speeds).
    pub fan_coupling_factor_q16: i32,
}

impl NoiseFloorEstimator {
    pub const fn new() -> Self {
        Self {
            noise_energy_q16: 655,        // Initial nominal floor (~0.01)
            min_energy_q16: 655,
            snr_q16: 0,
            alpha_rise_q16: 1310,        // 0.02 * 65536
            alpha_fall_q16: 6554,        // 0.10 * 65536
            fan_coupling_factor_q16: FP_Q16_ONE,
        }
    }
}

// -----------------------------------------------------------------------------
// Normalized Least Mean Squares (NLMS) Acoustic Echo Canceller Structures
// -----------------------------------------------------------------------------

/// State structures for the Normalized Least Mean Squares (NLMS) acoustic echo canceler.
pub struct AcousticEchoCancellerState {
    /// Adaptive FIR filter weight coefficients in 16.16 fixed point.
    pub filter_weights_q16: [i32; NLMS_FILTER_TAPS],
    /// Ring buffer storing past far-end (speaker output) samples in 16.16 fixed point.
    pub speaker_history_q16: [i32; NLMS_FILTER_TAPS],
    /// Current write index in the speaker history ring.
    pub history_cursor: usize,
    /// NLMS normalized adaptation step size ($\mu$) in 16.16 fixed point (~0.25).
    pub step_size_mu_q16: i32,
    /// Power regularization constant ($\epsilon$) preventing division by zero.
    pub regularization_eps_q16: i32,
    /// Dual-talk state: true when near-end user is speaking during speaker playback.
    pub dual_talk_detected: bool,
    /// Echo Return Loss Enhancement (ERLE) metric in decibels (dB).
    pub erle_db: i32,
    /// Speaker reference signal power estimate in 16.16 fixed point.
    pub speaker_power_q16: i32,
    /// Microphone input signal power estimate in 16.16 fixed point.
    pub mic_power_q16: i32,
    /// Error signal power estimate after echo subtraction.
    pub error_power_q16: i32,
}

impl AcousticEchoCancellerState {
    /// Allocate an empty acoustic echo canceler state.
    pub fn new() -> Self {
        Self {
            filter_weights_q16: [0; NLMS_FILTER_TAPS],
            speaker_history_q16: [0; NLMS_FILTER_TAPS],
            history_cursor: 0,
            step_size_mu_q16: 16384, // 0.25 * 65536
            regularization_eps_q16: 655, // 0.01 * 65536
            dual_talk_detected: false,
            erle_db: 0,
            speaker_power_q16: 0,
            mic_power_q16: 0,
            error_power_q16: 0,
        }
    }
}
