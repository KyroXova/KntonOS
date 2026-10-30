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


// -----------------------------------------------------------------------------
// VAD Feature Extraction & Spectral Analysis
// -----------------------------------------------------------------------------

/// Extract spectral features, energy metrics, and zero-crossing counts from a 5ms PCM frame.
pub fn extract_spectral_features(samples: &[i16; VAD_FRAME_SAMPLES]) -> VadSpectralFeatures {
    let mut sum_energy_q16: i64 = 0;
    let mut sum_low_energy_q16: i64 = 0;
    let mut zero_crossings: u16 = 0;

    // One-pole low-pass filter at ~1500 Hz for vocal formant extraction (alpha ~= 0.18 in Q16 = 11796)
    let alpha_low_q16 = 11796i32;
    let mut filter_state_q16 = 0i32;

    let mut prev_sign = samples[0] >= 0;

    for &raw_sample in samples.iter() {
        // Normalize 16-bit PCM sample to Q16.16 [-65536, 65535]
        let s_q16 = (raw_sample as i32) * 2;
        let s_sq = ((s_q16 as i64) * (s_q16 as i64)) >> 16;
        sum_energy_q16 += s_sq;

        // Apply 1500 Hz low-pass filter for vocal fundamental energy
        let diff = s_q16 - filter_state_q16;
        let delta = fp_mul_q16(diff, alpha_low_q16);
        filter_state_q16 += delta;

        let low_sq = ((filter_state_q16 as i64) * (filter_state_q16 as i64)) >> 16;
        sum_low_energy_q16 += low_sq;

        // Zero-crossing detection
        let current_sign = raw_sample >= 0;
        if current_sign != prev_sign {
            zero_crossings = zero_crossings.saturating_add(1);
            prev_sign = current_sign;
        }
    }

    let frame_energy_q16 = (sum_energy_q16 / (VAD_FRAME_SAMPLES as i64)) as i32;
    let low_band_energy_q16 = (sum_low_energy_q16 / (VAD_FRAME_SAMPLES as i64)) as i32;
    let high_band_energy_q16 = (frame_energy_q16 - low_band_energy_q16).max(0);

    // Spectral flatness estimation: ratio of high/low distribution vs peak formants
    let spectral_flatness_q16 = if frame_energy_q16 <= 10 {
        FP_Q16_ONE // Digital silence is treated as flat noise
    } else {
        // High vocal formant concentration yields low flatness (< 0.40)
        let low_ratio = fp_div_q16(low_band_energy_q16, frame_energy_q16);
        if low_ratio >= 45875 { // >= 70% low band
            16384 // ~0.25 (pronounced harmonic formants)
        } else if low_ratio >= 32768 { // >= 50% low band
            29491 // ~0.45 (mixed speech)
        } else {
            // Flat noise / fan rumble / high hiss
            52428 // ~0.80
        }
    };

    VadSpectralFeatures {
        frame_energy_q16,
        zero_crossing_count: zero_crossings,
        spectral_flatness_q16,
        low_band_energy_q16,
        high_band_energy_q16,
        energy_variance_q16: 0,
    }
}

// -----------------------------------------------------------------------------
// Dynamic Background Noise Floor Estimator & Chassis Rejection
// -----------------------------------------------------------------------------

impl NoiseFloorEstimator {
    /// Update background noise floor estimate based on current frame features and speech state.
    pub fn update(&mut self, features: &VadSpectralFeatures, is_speech: bool) {
        let frame_e = features.frame_energy_q16;

        if !is_speech {
            // Track noise floor when human speech is inactive
            if frame_e < self.noise_energy_q16 {
                // Noise dropped: track downwards quickly
                let diff = self.noise_energy_q16 - frame_e;
                let step = fp_mul_q16(diff, self.alpha_fall_q16);
                self.noise_energy_q16 = (self.noise_energy_q16 - step).max(10);
            } else {
                // Noise rose (fan spinning up): track upwards slowly
                let diff = frame_e - self.noise_energy_q16;
                let step = fp_mul_q16(diff, self.alpha_rise_q16);
                self.noise_energy_q16 = self.noise_energy_q16 + step;
            }
        }

        // Calculate instantaneous SNR: SNR_dB ~= 10 * log10(E_frame / E_noise)
        let effective_noise = fp_mul_q16(self.noise_energy_q16, self.fan_coupling_factor_q16).max(10);
        let ratio = fp_div_q16(frame_e.max(10), effective_noise);
        // Fixed-point linear SNR indicator
        self.snr_q16 = ratio;
    }

    /// Modulate noise floor based on handheld cooling fan RPM percentage (0% to 100%).
    pub fn set_fan_rpm_factor(&mut self, fan_percent: u32) {
        let pct = fan_percent.min(100);
        // At 0% fan: coupling = 1.0 (65536)
        // At 100% fan: coupling = 2.5 (163840) to prevent fan whine false speech triggers
        self.fan_coupling_factor_q16 = FP_Q16_ONE + ((pct as i32) * 98304) / 100;
    }

    /// Detect transient mechanical chassis noise (tactile button click or bumper release).
    /// Mechanical clicks exhibit high zero-crossing rates, high high-band energy, and low vocal formant ratio.
    pub fn is_chassis_noise(&self, features: &VadSpectralFeatures) -> bool {
        let high_energy_bias = features.high_band_energy_q16 > (features.low_band_energy_q16 * 2);
        let high_zcr = features.zero_crossing_count > 65;
        let transient_spike = features.frame_energy_q16 > (self.noise_energy_q16 * 3);

        high_energy_bias && high_zcr && transient_spike
    }
}

// -----------------------------------------------------------------------------
// Voice Activity Detector Engine with Hangover Timer
// -----------------------------------------------------------------------------

/// Real-time low-latency Voice Activity Detector (VAD) engine.
pub struct VoiceActivityDetector {
    /// Internal decision and hangover state.
    pub state: VadState,
    /// Background noise floor and fan tracking estimator.
    pub noise_estimator: NoiseFloorEstimator,
    /// Speech probability detection threshold in 16.16 fixed point (~0.50).
    pub speech_threshold_q16: i32,
}

impl VoiceActivityDetector {
    /// Construct a new VAD engine with configured hangover duration.
    pub fn new(hangover_duration_frames: u16) -> Self {
        Self {
            state: VadState::new(hangover_duration_frames),
            noise_estimator: NoiseFloorEstimator::new(),
            speech_threshold_q16: 32768, // 0.50 * 65536
        }
    }

    /// Analyze a 5ms voice frame and return the classified VAD decision and extracted features.
    pub fn process_frame(&mut self, samples: &[i16; VAD_FRAME_SAMPLES]) -> (VadDecision, VadSpectralFeatures) {
        let features = extract_spectral_features(samples);

        // 1. Check for mechanical button click / chassis noise rejection
        if self.noise_estimator.is_chassis_noise(&features) {
            // Chassis click rejected
            self.state.decision = VadDecision::ChassisNoise;
            self.state.speech_probability_q16 = 0;
            self.noise_estimator.update(&features, false);
            return (VadDecision::ChassisNoise, features);
        }

        // 2. Evaluate Voice Activity score:
        // Speech criteria:
        // - SNR > threshold (frame energy significantly exceeds fan noise floor)
        // - Formant concentration: low band energy >= 40% of total
        // - Moderate zero-crossing rate: typical speech is 5..65 crossings per 240 samples
        // - Spectral flatness < 0.70
        let mut score_q16 = 0i32;

        let effective_noise = fp_mul_q16(
            self.noise_estimator.noise_energy_q16,
            self.noise_estimator.fan_coupling_factor_q16,
        );

        // Energy criterion (up to +0.40 score)
        if features.frame_energy_q16 > (effective_noise * 3) {
            score_q16 += 26214; // 0.40
        } else if features.frame_energy_q16 > (effective_noise * 2) {
            score_q16 += 16384; // 0.25
        } else if features.frame_energy_q16 > effective_noise {
            score_q16 += 6554;  // 0.10
        }

        // Vocal formant concentration criterion (up to +0.30 score)
        if features.frame_energy_q16 > 10 {
            let low_ratio = fp_div_q16(features.low_band_energy_q16, features.frame_energy_q16);
            if low_ratio >= 32768 { // >= 50% in vocal formant region
                score_q16 += 19661; // 0.30
            } else if low_ratio >= 22938 { // >= 35%
                score_q16 += 9830;  // 0.15
            }
        }

        // Zero-crossing speech range criterion (5 to 65 per 240 samples = ~500 Hz to 6500 Hz) (+0.20 score)
        if features.zero_crossing_count >= 5 && features.zero_crossing_count <= 65 {
            score_q16 += 13107; // 0.20
        }

        // Spectral non-flatness criterion (+0.10 score)
        if features.spectral_flatness_q16 < 39322 { // < 0.60
            score_q16 += 6554; // 0.10
        }

        self.state.speech_probability_q16 = score_q16.min(FP_Q16_ONE);

        // 3. Make speech classification decision with hangover timer hysteresis
        let raw_speech = self.state.speech_probability_q16 >= self.speech_threshold_q16;

        if raw_speech {
            self.state.decision = VadDecision::Speech;
            self.state.hangover_counter = self.state.hangover_duration_frames;
            self.state.consecutive_speech_frames = self.state.consecutive_speech_frames.saturating_add(1);
        } else {
            // Raw speech below threshold: check if hangover timer is still active
            if self.state.hangover_counter > 0 {
                self.state.hangover_counter -= 1;
                // Hold speech decision to prevent chopped phonemes at word endings
                self.state.decision = VadDecision::Speech;
            } else {
                self.state.decision = VadDecision::Silence;
                self.state.consecutive_speech_frames = 0;
            }
        }

        // 4. Update noise floor tracker
        let speech_active = self.state.decision == VadDecision::Speech;
        self.noise_estimator.update(&features, speech_active);

        (self.state.decision, features)
    }

    /// Adjust hangover duration in frames.
    pub fn set_hangover_duration(&mut self, frames: u16) {
        self.state.hangover_duration_frames = frames;
    }
}


// -----------------------------------------------------------------------------
// Normalized Least Mean Squares (NLMS) Acoustic Echo Canceller Implementation
// -----------------------------------------------------------------------------

/// Real-time Normalized Least Mean Squares (NLMS) Acoustic Echo Canceller (AEC).
pub struct AcousticEchoCanceller {
    pub state: AcousticEchoCancellerState,
    /// Dual-talk threshold factor in 16.16 fixed point (e.g. 0.60 * 65536 = 39322).
    pub dual_talk_threshold_q16: i32,
}

impl AcousticEchoCanceller {
    /// Construct a new acoustic echo canceler with specified adaptation step size.
    pub fn new(step_size_mu_q16: i32) -> Self {
        let mut state = AcousticEchoCancellerState::new();
        if step_size_mu_q16 > 0 {
            state.step_size_mu_q16 = step_size_mu_q16;
        }
        Self {
            state,
            dual_talk_threshold_q16: 39322, // 0.60 * 65536
        }
    }

    /// Process a single audio sample: subtract estimated echo and adapt filter weights.
    /// Both `mic_sample_q16` and `speaker_sample_q16` are in 16.16 fixed-point format.
    /// Returns the echo-cancelled output error sample `e[n]`.
    #[inline]
    pub fn process_sample(&mut self, mic_sample_q16: i32, speaker_sample_q16: i32) -> i32 {
        // 1. Insert current speaker sample into ring buffer history
        self.state.speaker_history_q16[self.state.history_cursor] = speaker_sample_q16;

        // 2. Compute estimated acoustic echo coupling: d_hat = sum(w_k * x[n-k])
        let mut echo_estimate_acc: i64 = 0;
        let mut speaker_norm_acc: i64 = 0;
        let mut max_speaker_abs: i32 = 0;

        let cursor = self.state.history_cursor;
        for k in 0..NLMS_FILTER_TAPS {
            // Read past speaker sample in reverse chronological order
            let hist_idx = (cursor + NLMS_FILTER_TAPS - k) % NLMS_FILTER_TAPS;
            let x_k = self.state.speaker_history_q16[hist_idx];
            let w_k = self.state.filter_weights_q16[k];

            echo_estimate_acc += ((w_k as i64) * (x_k as i64)) >> 16;
            speaker_norm_acc += ((x_k as i64) * (x_k as i64)) >> 16;

            let abs_x = x_k.abs();
            if abs_x > max_speaker_abs {
                max_speaker_abs = abs_x;
            }
        }

        let echo_estimate_q16 = echo_estimate_acc as i32;

        // 3. Compute error signal: e[n] = y[n] - d_hat[n]
        let error_q16 = mic_sample_q16 - echo_estimate_q16;

        // Advance ring cursor
        self.state.history_cursor = (self.state.history_cursor + 1) % NLMS_FILTER_TAPS;

        // 4. Geigel Dual-Talk Detection (DTD)
        // If microphone amplitude exceeds threshold * max_speaker_amplitude: near-end user is speaking!
        let mic_abs = mic_sample_q16.abs();
        let geigel_bound = fp_mul_q16(max_speaker_abs, self.dual_talk_threshold_q16);
        let is_dual_talk = mic_abs > geigel_bound && mic_abs > 1310; // > ~0.02 amplitude

        self.state.dual_talk_detected = is_dual_talk;

        // 5. Adapt filter weights only when dual-talk is NOT active
        // Weight update: w_k[n+1] = w_k[n] + mu * e[n] * x[n-k] / (||x||^2 + eps)
        if !is_dual_talk && speaker_norm_acc > 100 {
            let norm_q16 = (speaker_norm_acc.min(i32::MAX as i64) as i32) + self.state.regularization_eps_q16;
            let mu_e_q16 = fp_mul_q16(self.state.step_size_mu_q16, error_q16);
            let step_scale_q16 = fp_div_q16(mu_e_q16, norm_q16);

            for k in 0..NLMS_FILTER_TAPS {
                let hist_idx = (cursor + NLMS_FILTER_TAPS - k) % NLMS_FILTER_TAPS;
                let x_k = self.state.speaker_history_q16[hist_idx];
                let weight_delta = fp_mul_q16(step_scale_q16, x_k);
                self.state.filter_weights_q16[k] = self.state.filter_weights_q16[k].saturating_add(weight_delta);
            }
        }

        // Update power statistics for ERLE measurement
        let mic_sq = ((mic_sample_q16 as i64) * (mic_sample_q16 as i64)) >> 16;
        let err_sq = ((error_q16 as i64) * (error_q16 as i64)) >> 16;
        self.state.mic_power_q16 = (self.state.mic_power_q16 * 15 + (mic_sq as i32)) >> 4;
        self.state.error_power_q16 = (self.state.error_power_q16 * 15 + (err_sq as i32)) >> 4;

        if self.state.error_power_q16 > 10 && self.state.mic_power_q16 > self.state.error_power_q16 {
            let erle_ratio = fp_div_q16(self.state.mic_power_q16, self.state.error_power_q16);
            // Rough approximation of 10 * log10(ratio) in dB
            self.state.erle_db = if erle_ratio >= 655360 { // >= 10x
                20
            } else if erle_ratio >= 262144 { // >= 4x
                12
            } else if erle_ratio >= 131072 { // >= 2x
                6
            } else {
                3
            };
        }

        error_q16
    }

    /// Process a full 5ms frame of 16-bit PCM microphone samples against speaker output reference.
    pub fn process_frame(
        &mut self,
        mic_in: &[i16; VAD_FRAME_SAMPLES],
        speaker_in: &[i16; VAD_FRAME_SAMPLES],
        out_clean: &mut [i16; VAD_FRAME_SAMPLES],
    ) {
        for i in 0..VAD_FRAME_SAMPLES {
            let mic_q16 = (mic_in[i] as i32) * 2;
            let spk_q16 = (speaker_in[i] as i32) * 2;

            let clean_q16 = self.process_sample(mic_q16, spk_q16);

            // Convert back to 16-bit PCM with saturation
            let sample_i16 = (clean_q16 / 2).clamp(-32768, 32767) as i16;
            out_clean[i] = sample_i16;
        }
    }
}

// -----------------------------------------------------------------------------
// Unified Voice Chat Audio Processor Pipeline
// -----------------------------------------------------------------------------

/// Unified real-time voice communications processing pipeline.
/// Integrates NLMS Acoustic Echo Cancellation with Multi-Feature Voice Activity Detection.
pub struct VoiceChatProcessor {
    /// Adaptive acoustic echo canceler removing speaker bleed.
    pub echo_canceller: AcousticEchoCanceller,
    /// Low-latency multi-feature VAD engine.
    pub vad: VoiceActivityDetector,
    /// Temporary frame buffer holding echo-cancelled audio.
    clean_frame_scratch: [i16; VAD_FRAME_SAMPLES],
}

impl VoiceChatProcessor {
    /// Allocate a unified voice chat processor.
    pub fn new() -> Self {
        Self {
            echo_canceller: AcousticEchoCanceller::new(16384), // step size = 0.25
            vad: VoiceActivityDetector::new(DEFAULT_HANGOVER_FRAMES),
            clean_frame_scratch: [0i16; VAD_FRAME_SAMPLES],
        }
    }

    /// Process 5ms microphone capture frame and speaker playback reference:
    /// 1. Subtract acoustic speaker echo using NLMS adaptive filter.
    /// 2. Perform VAD analysis on the echo-cancelled clean microphone signal.
    /// 3. Return `(VadDecision, clean_samples)`.
    pub fn process_voice_frame(
        &mut self,
        mic_in: &[i16; VAD_FRAME_SAMPLES],
        speaker_in: &[i16; VAD_FRAME_SAMPLES],
        out_clean: &mut [i16; VAD_FRAME_SAMPLES],
    ) -> VadDecision {
        // Step 1: Acoustic Echo Cancellation
        self.echo_canceller.process_frame(mic_in, speaker_in, &mut self.clean_frame_scratch);

        // Copy cleaned audio to output
        out_clean.copy_from_slice(&self.clean_frame_scratch);

        // Step 2: Voice Activity Detection on cleaned speech
        let (decision, _) = self.vad.process_frame(&self.clean_frame_scratch);

        decision
    }
}
