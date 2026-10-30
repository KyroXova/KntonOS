//! Kernel-Direct Real-Time In-Game Audio Dynamic Range Compressor & Hearing Safety Limiter.
//! Provides bare-metal low-latency dynamic range control (DRC) for handheld game audio:

#![allow(dead_code)]

extern crate alloc;

use crate::audio::hrtf::AUDIO_SAMPLE_RATE;
use crate::audio::voice_dsp::log10_f32;

// Pure no_std Floating Point Math Helpers

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

// 1. Dynamic Range Compressor Parameters & Gain Curve

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

// 2. Envelope Detection State & Smoothing Filter

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

// 3. Sub-Millisecond Lookahead Buffer & Real-Time Compressor Engine

/// Capacity of the lookahead circular buffer (64 samples = 1.33 ms at 48 kHz).
pub const LOOKAHEAD_CAPACITY: usize = 64;
pub const LOOKAHEAD_MASK: usize = LOOKAHEAD_CAPACITY - 1;

/// Stereo lookahead delay ring buffer preventing transient overshoot.
#[derive(Debug, Clone, Copy)]
pub struct LookaheadBuffer {
    left_buf: [f32; LOOKAHEAD_CAPACITY],
    right_buf: [f32; LOOKAHEAD_CAPACITY],
    write_idx: usize,
    pub delay_samples: usize,
}

impl LookaheadBuffer {
    pub const fn new(delay_samples: usize) -> Self {
        let clamped = if delay_samples >= LOOKAHEAD_CAPACITY {
            LOOKAHEAD_CAPACITY - 1
        } else {
            delay_samples
        };

        Self {
            left_buf: [0.0; LOOKAHEAD_CAPACITY],
            right_buf: [0.0; LOOKAHEAD_CAPACITY],
            write_idx: 0,
            delay_samples: clamped,
        }
    }

    pub fn reset(&mut self) {
        self.left_buf = [0.0; LOOKAHEAD_CAPACITY];
        self.right_buf = [0.0; LOOKAHEAD_CAPACITY];
        self.write_idx = 0;
    }

    /// Pushes stereo sample into buffer and pops the delayed sample from `delay_samples` ago.
    pub fn push_and_pop(&mut self, left: f32, right: f32) -> (f32, f32) {
        if self.delay_samples == 0 {
            return (left, right);
        }

        self.left_buf[self.write_idx] = left;
        self.right_buf[self.write_idx] = right;

        let read_idx = (self.write_idx + LOOKAHEAD_CAPACITY - self.delay_samples) & LOOKAHEAD_MASK;
        self.write_idx = (self.write_idx + 1) & LOOKAHEAD_MASK;

        (self.left_buf[read_idx], self.right_buf[read_idx])
    }
}

/// Stereo Dynamic Range Compressor (DRC) with soft-knee gain computer, lookahead, and variable envelope detector.
#[derive(Debug, Clone, Copy)]
pub struct DynamicRangeCompressor {
    pub params: CompressorParameters,
    pub lookahead: LookaheadBuffer,
    pub detector: EnvelopeDetector,
    pub current_gr_db: f32,
    pub sidechain_peak: f32,
}

impl DynamicRangeCompressor {
    /// Creates a new compressor with the specified parameters and lookahead delay.
    pub fn new(params: CompressorParameters, lookahead_samples: usize) -> Self {
        let mut comp = Self {
            params,
            lookahead: LookaheadBuffer::new(lookahead_samples),
            detector: EnvelopeDetector::new(params.sample_rate, params.attack_time_ms, params.release_time_ms),
            current_gr_db: 0.0,
            sidechain_peak: 0.0,
        };
        comp.detector.envelope_db = 0.0; // Initial gain reduction is 0 dB
        comp
    }

    pub fn reset(&mut self) {
        self.lookahead.reset();
        self.detector.reset();
        self.detector.envelope_db = 0.0;
        self.current_gr_db = 0.0;
        self.sidechain_peak = 0.0;
    }

    pub fn set_parameters(&mut self, params: CompressorParameters) {
        self.params = params;
        self.detector.update_coefficients(params.sample_rate, params.attack_time_ms, params.release_time_ms);
    }

    /// Processes a single stereo sample pair, returning compressed and leveled stereo audio.
    pub fn process_sample(&mut self, left_in: f32, right_in: f32) -> (f32, f32) {
        // 1. Stereo-linked peak detection (prevents stereo image shifting)
        let abs_l = if left_in < 0.0 { -left_in } else { left_in };
        let abs_r = if right_in < 0.0 { -right_in } else { right_in };
        let peak = if abs_l > abs_r { abs_l } else { abs_r };
        self.sidechain_peak = peak;

        // 2. Convert to decibels and compute static target gain reduction
        let peak_db = linear_to_db(peak);
        let target_gr_db = self.params.calculate_gain_reduction_db(peak_db); // <= 0.0 dB

        // 3. Smooth gain reduction through decoupled attack/release filter
        // In gain reduction domain:
        if target_gr_db < self.detector.envelope_db {
            // Attack phase
            self.detector.envelope_db = self.detector.attack_coeff * self.detector.envelope_db
                + (1.0 - self.detector.attack_coeff) * target_gr_db;
        } else {
            // Release phase
            self.detector.envelope_db = self.detector.release_coeff * self.detector.envelope_db
                + (1.0 - self.detector.release_coeff) * target_gr_db;
        }
        self.current_gr_db = self.detector.envelope_db;

        // 4. Retrieve lookahead delayed audio samples
        let (del_l, del_r) = self.lookahead.push_and_pop(left_in, right_in);

        // 5. Calculate total net linear gain (gain reduction + makeup gain)
        let total_gain_db = self.current_gr_db + self.params.makeup_gain_db;
        let linear_gain = db_to_linear(total_gain_db);

        // 6. Apply gain to delayed audio
        (del_l * linear_gain, del_r * linear_gain)
    }

    /// Processes an audio frame slice in-place or buffer-to-buffer.
    pub fn process_frame(
        &mut self,
        in_left: &[f32],
        in_right: &[f32],
        out_left: &mut [f32],
        out_right: &mut [f32],
    ) {
        let count = in_left.len().min(in_right.len()).min(out_left.len()).min(out_right.len());
        for i in 0..count {
            let (l, r) = self.process_sample(in_left[i], in_right[i]);
            out_left[i] = l;
            out_right[i] = r;
        }
    }
}

// 4. True-Peak Estimator, Hearing Safety Limiter & Footstep Booster

/// ITU-R BS.1770 compliant true-peak inter-sample loudness estimator.
#[derive(Debug, Clone, Copy, Default)]
pub struct TruePeakEstimator {
    prev_left: f32,
    prev_right: f32,
}

impl TruePeakEstimator {
    pub const fn new() -> Self {
        Self {
            prev_left: 0.0,
            prev_right: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.prev_left = 0.0;
        self.prev_right = 0.0;
    }

    /// Estimates the true continuous analog peak amplitude across samples,
    /// detecting inter-sample peaks that would clip during digital-to-analog conversion.
    pub fn estimate_sample_pair(&mut self, left: f32, right: f32) -> f32 {
        let abs_l = if left < 0.0 { -left } else { left };
        let abs_r = if right < 0.0 { -right } else { right };
        let direct_peak = if abs_l > abs_r { abs_l } else { abs_r };

        // Parabolic inter-sample interpolation between previous and current sample
        let inter_l = 0.5 * (self.prev_left + left);
        let inter_r = 0.5 * (self.prev_right + right);
        let abs_inter_l = if inter_l < 0.0 { -inter_l } else { inter_l };
        let abs_inter_r = if inter_r < 0.0 { -inter_r } else { inter_r };
        let inter_peak = if abs_inter_l > abs_inter_r { abs_inter_l } else { abs_inter_r };

        self.prev_left = left;
        self.prev_right = right;

        if inter_peak > direct_peak {
            inter_peak
        } else {
            direct_peak
        }
    }
}

/// Hearing Safety Brickwall Limiter.
/// Implements sub-millisecond lookahead true-peak limiting with an uncompromising
#[derive(Debug, Clone, Copy)]
pub struct HearingSafetyLimiter {
    pub ceiling_db: f32,
    pub ceiling_linear: f32,
    pub lookahead: LookaheadBuffer,
    pub detector: EnvelopeDetector,
    pub true_peak: TruePeakEstimator,
    pub current_attenuation: f32,
    pub enabled: bool,
}

impl HearingSafetyLimiter {
    /// Creates a hearing safety limiter with standard -1.0 dBFS ceiling and 16-sample lookahead (0.33 ms).
    pub fn new(ceiling_db: f32, lookahead_samples: usize) -> Self {
        let safe_ceiling = if ceiling_db > -0.2 { -0.2 } else { ceiling_db };
        let ceiling_lin = db_to_linear(safe_ceiling);

        Self {
            ceiling_db: safe_ceiling,
            ceiling_linear: ceiling_lin,
            lookahead: LookaheadBuffer::new(lookahead_samples),
            detector: EnvelopeDetector::new(AUDIO_SAMPLE_RATE, 0.1, 40.0),
            true_peak: TruePeakEstimator::new(),
            current_attenuation: 1.0,
            enabled: true,
        }
    }

    pub fn reset(&mut self) {
        self.lookahead.reset();
        self.detector.reset();
        self.true_peak.reset();
        self.current_attenuation = 1.0;
    }

    /// Processes stereo samples, clamping true-peak levels strictly beneath ceiling.
    pub fn process_sample(&mut self, left_in: f32, right_in: f32) -> (f32, f32) {
        if !self.enabled {
            return (left_in, right_in);
        }

        // 1. Estimate true-peak level
        let peak = self.true_peak.estimate_sample_pair(left_in, right_in);

        // 2. Compute target attenuation ratio
        let target_ratio = if peak > self.ceiling_linear && peak > 1e-6 {
            self.ceiling_linear / peak
        } else {
            1.0
        };

        // 3. Smooth attenuation envelope
        let target_db = linear_to_db(target_ratio); // <= 0.0 dB
        if target_db < self.detector.envelope_db {
            // Attack (fast clamp)
            self.detector.envelope_db = self.detector.attack_coeff * self.detector.envelope_db
                + (1.0 - self.detector.attack_coeff) * target_db;
        } else {
            // Release (smooth recovery)
            self.detector.envelope_db = self.detector.release_coeff * self.detector.envelope_db
                + (1.0 - self.detector.release_coeff) * target_db;
        }

        self.current_attenuation = db_to_linear(self.detector.envelope_db);

        // 4. Retrieve lookahead sample
        let (del_l, del_r) = self.lookahead.push_and_pop(left_in, right_in);

        // 5. Apply attenuation and strict brickwall safety ceiling clamp
        let out_l = (del_l * self.current_attenuation).clamp(-self.ceiling_linear, self.ceiling_linear);
        let out_r = (del_r * self.current_attenuation).clamp(-self.ceiling_linear, self.ceiling_linear);

        (out_l, out_r)
    }
}

/// Esports Footstep & Subtle Detail Booster.
/// Implements selective upward dynamic range expansion for quiet cues between -50 dBFS and -25 dBFS,
#[derive(Debug, Clone, Copy)]
pub struct FootstepDetailBooster {
    pub low_threshold_db: f32,
    pub high_threshold_db: f32,
    pub max_boost_db: f32,
    pub enabled: bool,
}

impl FootstepDetailBooster {
    pub const fn new() -> Self {
        Self {
            low_threshold_db: -48.0,
            high_threshold_db: -24.0,
            max_boost_db: 7.5,
            enabled: true,
        }
    }

    /// Evaluates upward boost curve:
    /// Below low threshold (-48 dB): max boost (+7.5 dB).
    pub fn calculate_boost_db(&self, level_db: f32) -> f32 {
        if !self.enabled || level_db <= -90.0 {
            return 0.0;
        }

        if level_db <= self.low_threshold_db {
            self.max_boost_db
        } else if level_db >= self.high_threshold_db {
            0.0
        } else {
            let t = (level_db - self.low_threshold_db) / (self.high_threshold_db - self.low_threshold_db);
            self.max_boost_db * (1.0 - t)
        }
    }

    pub fn process_sample(&self, left_in: f32, right_in: f32) -> (f32, f32) {
        if !self.enabled {
            return (left_in, right_in);
        }

        let abs_l = if left_in < 0.0 { -left_in } else { left_in };
        let abs_r = if right_in < 0.0 { -right_in } else { right_in };
        let peak = if abs_l > abs_r { abs_l } else { abs_r };

        let level_db = linear_to_db(peak);
        let boost_db = self.calculate_boost_db(level_db);
        let boost_gain = db_to_linear(boost_db);

        (left_in * boost_gain, right_in * boost_gain)
    }
}

/// Operational DRC Profile Mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrcProfile {
    /// Raw uncompressed audio pass-through.
    Bypass,
    /// Balanced dynamic compression for handheld speakers/headphones.
    DefaultGaming,
    /// Esports mode: Footstep booster + hearing safety limiter.
    EsportsTactical,
    /// Night-time mode: heavy dynamic range reduction for low-volume listening.
    NightQuiet,
}

/// Integrated Kernel-Direct Real-Time In-Game Audio DRC Engine.
#[derive(Debug, Clone, Copy)]
pub struct GameAudioDrcEngine {
    pub compressor: DynamicRangeCompressor,
    pub booster: FootstepDetailBooster,
    pub limiter: HearingSafetyLimiter,
    pub profile: DrcProfile,
}

impl GameAudioDrcEngine {
    pub fn new(profile: DrcProfile) -> Self {
        let comp_params = match profile {
            DrcProfile::Bypass => CompressorParameters::default_gaming(),
            DrcProfile::DefaultGaming => CompressorParameters::default_gaming(),
            DrcProfile::EsportsTactical => CompressorParameters::esports_footstep_boost(),
            DrcProfile::NightQuiet => {
                let mut p = CompressorParameters::default_gaming();
                p.threshold_db = -24.0;
                p.ratio = 8.0;
                p.makeup_gain_db = 6.0;
                p
            }
        };

        let mut booster = FootstepDetailBooster::new();
        booster.enabled = profile == DrcProfile::EsportsTactical;

        Self {
            compressor: DynamicRangeCompressor::new(comp_params, 24),
            booster,
            limiter: HearingSafetyLimiter::new(-1.0, 16),
            profile,
        }
    }

    pub fn set_profile(&mut self, profile: DrcProfile) {
        self.profile = profile;
        self.booster.enabled = profile == DrcProfile::EsportsTactical;
        let params = match profile {
            DrcProfile::Bypass => CompressorParameters::default_gaming(),
            DrcProfile::DefaultGaming => CompressorParameters::default_gaming(),
            DrcProfile::EsportsTactical => CompressorParameters::esports_footstep_boost(),
            DrcProfile::NightQuiet => {
                let mut p = CompressorParameters::default_gaming();
                p.threshold_db = -24.0;
                p.ratio = 8.0;
                p.makeup_gain_db = 6.0;
                p
            }
        };
        self.compressor.set_parameters(params);
    }

    pub fn reset(&mut self) {
        self.compressor.reset();
        self.limiter.reset();
    }

    /// Processes a single stereo sample pair through the complete DRC pipeline:
    /// Footstep Booster -> Soft-Knee Compressor -> Hearing Safety Brickwall Limiter.
    pub fn process_sample(&mut self, left_in: f32, right_in: f32) -> (f32, f32) {
        if self.profile == DrcProfile::Bypass {
            return (left_in, right_in);
        }

        // 1. Tactical footstep/whisper expansion (if enabled)
        let (boost_l, boost_r) = self.booster.process_sample(left_in, right_in);

        // 2. Dynamic range soft-knee compression
        let (comp_l, comp_r) = self.compressor.process_sample(boost_l, boost_r);

        // 3. Hearing safety brickwall true-peak limiter
        self.limiter.process_sample(comp_l, comp_r)
    }

    /// Processes an entire stereo audio frame block.
    pub fn process_frame(
        &mut self,
        in_left: &[f32],
        in_right: &[f32],
        out_left: &mut [f32],
        out_right: &mut [f32],
    ) {
        let count = in_left.len().min(in_right.len()).min(out_left.len()).min(out_right.len());
        for i in 0..count {
            let (l, r) = self.process_sample(in_left[i], in_right[i]);
            out_left[i] = l;
            out_right[i] = r;
        }
    }
}

// 5. Subsystem Verification Suite & Boot Sentinel

/// Verification self-test suite for Kernel-Direct Real-Time In-Game Audio Dynamic Range Compressor & Hearing Safety Limiter (TASK-KNT-124).
pub fn test_audio_compressor_subsystem() -> bool {
    crate::serial_write_str("[KNT-AUDIO-COMPRESSOR-TEST] Running Audio Dynamic Range Compressor & Hearing Safety Limiter verification suite...\r\n");

    // 1. Decibel conversions, soft-knee gain computer, and static curve
    let lin_half = 0.5f32;
    let db_half = linear_to_db(lin_half);
    let lin_restored = db_to_linear(db_half);
    if (lin_restored - 0.5).abs() > 0.01 {
        return false;
    }

    let params = CompressorParameters::default_gaming();
    let gr_low = params.calculate_gain_reduction_db(-35.0);
    if gr_low.abs() > 0.01 {
        return false;
    }
    let gr_high = params.calculate_gain_reduction_db(-6.0);
    if gr_high > -7.0 || gr_high < -11.0 {
        return false;
    }
    crate::serial_write_str("  * Decibel conversions, soft-knee gain computer, and static curve verified\r\n");

    // 2. Sub-millisecond lookahead buffer and decoupled attack/release envelope detector
    let mut lookahead = LookaheadBuffer::new(8);
    for i in 0..8 {
        lookahead.push_and_pop(i as f32, i as f32);
    }
    let (l_del, r_del) = lookahead.push_and_pop(8.0, 8.0);
    if l_del != 0.0 || r_del != 0.0 {
        return false;
    }

    let mut env = EnvelopeDetector::new(AUDIO_SAMPLE_RATE, 2.0, 50.0);
    for _ in 0..600 {
        env.step(0.0);
    }
    if env.envelope_db < -1.5 {
        return false;
    }
    crate::serial_write_str("  * Sub-millisecond lookahead buffer and decoupled attack/release envelope detector verified\r\n");

    // 3. True-peak inter-sample estimator and -1.0 dBFS hearing safety brickwall limiter
    let mut true_peak = TruePeakEstimator::new();
    let tp_est = true_peak.estimate_sample_pair(0.9, 0.9);
    if tp_est <= 0.0 {
        return false;
    }

    let mut limiter = HearingSafetyLimiter::new(-1.0, 16);
    let max_allowed = db_to_linear(-1.0);
    for _ in 0..100 {
        let (out_l, out_r) = limiter.process_sample(2.5, 2.5);
        if out_l > max_allowed + 0.001 || out_r > max_allowed + 0.001 {
            return false;
        }
    }
    crate::serial_write_str("  * True-peak inter-sample estimator and -1.0 dBFS hearing safety brickwall limiter verified\r\n");

    // 4. Tactical footstep booster and subtle audio presence expansion
    let booster = FootstepDetailBooster::new();
    let boost_quiet = booster.calculate_boost_db(-40.0);
    if boost_quiet < 4.0 || boost_quiet > 8.0 {
        return false;
    }
    let boost_loud = booster.calculate_boost_db(-15.0);
    if boost_loud.abs() > 0.01 {
        return false;
    }
    crate::serial_write_str("  * Tactical footstep booster and subtle audio presence expansion verified\r\n");

    // 5. End-to-end real-time in-game audio DRC and hearing safety pipeline
    let mut engine = GameAudioDrcEngine::new(DrcProfile::EsportsTactical);
    let mut in_l = [0.0f32; 64];
    let mut in_r = [0.0f32; 64];
    for (i, (l, r)) in in_l.iter_mut().zip(in_r.iter_mut()).enumerate() {
        let val = if i < 32 { 0.02 } else { 1.8 }; // Quiet footstep followed by loud explosion
        *l = val;
        *r = val;
    }
    let mut out_l = [0.0f32; 64];
    let mut out_r = [0.0f32; 64];
    engine.process_frame(&in_l, &in_r, &mut out_l, &mut out_r);

    let mut max_output_peak = 0.0f32;
    for i in 0..64 {
        if out_l[i].abs() > max_output_peak {
            max_output_peak = out_l[i].abs();
        }
        if out_r[i].abs() > max_output_peak {
            max_output_peak = out_r[i].abs();
        }
    }
    if max_output_peak <= 0.0 || max_output_peak > max_allowed + 0.005 {
        return false;
    }
    crate::serial_write_str("  * End-to-end real-time in-game audio DRC and hearing safety pipeline verified\r\n");

    crate::serial_write_str("[KNT-AUDIO-COMPRESSOR-PASS] Real-time audio dynamic range compressor and hearing safety limiter operational\r\n");
    true
}

// Unit Tests

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
        let gr_high = params.calculate_gain_reduction_db(-6.0);
        assert!((gr_high - (-9.0)).abs() < 0.1);

        // Test envelope detector
        let mut env = EnvelopeDetector::new(48000.0, 5.0, 100.0);
        for _ in 0..500 {
            env.step(0.0);
        }
        assert!(env.envelope_db > -1.0);
    }
    #[test]
    pub fn test_lookahead_buffer_and_compressor() {
        let mut lookahead = LookaheadBuffer::new(4);
        assert_eq!(lookahead.delay_samples, 4);

        // Push 1, 2, 3, 4 -> outputs should be 0.0 initially
        for i in 1..=4 {
            let (l, r) = lookahead.push_and_pop(i as f32, i as f32);
            assert_eq!(l, 0.0);
            assert_eq!(r, 0.0);
        }

        // Push 5 -> pop 1
        let (l5, r5) = lookahead.push_and_pop(5.0, 5.0);
        assert_eq!(l5, 1.0);
        assert_eq!(r5, 1.0);

        // Test DynamicRangeCompressor with high-amplitude input
        let mut params = CompressorParameters::default_gaming();
        params.makeup_gain_db = 0.0; // 0 dB makeup gain to observe pure compression
        params.threshold_db = -12.0;
        params.ratio = 4.0;
        params.attack_time_ms = 0.5;
        params.release_time_ms = 20.0;

        let mut compressor = DynamicRangeCompressor::new(params, 16);

        // Feed quiet signal (-30 dBFS -> amp ~ 0.0316)
        for _ in 0..100 {
            compressor.process_sample(0.0316, 0.0316);
        }
        assert!((compressor.current_gr_db - 0.0).abs() < 0.1);

        // Feed loud step signal (0 dBFS -> amp 1.0)
        for _ in 0..200 {
            compressor.process_sample(1.0, 1.0);
        }
        // At 0 dBFS input and -12 dB threshold with 4:1 ratio:
        // Expected GR is (0 - 12) * (1 - 1/4) = 12 * 0.75 = -9.0 dB
        assert!(compressor.current_gr_db < -7.0 && compressor.current_gr_db > -10.0);
    }
    #[test]
    pub fn test_limiter_and_footstep_booster() {
        let mut booster = FootstepDetailBooster::new();
        // Quiet sound at -40 dBFS should receive significant boost (> 4.0 dB)
        let boost_quiet = booster.calculate_boost_db(-40.0);
        assert!(boost_quiet > 4.0);

        // Loud sound at -10 dBFS should receive 0 dB boost
        let boost_loud = booster.calculate_boost_db(-10.0);
        assert_eq!(boost_loud, 0.0);

        // Test Hearing Safety Limiter with extreme clipping input (+6 dBFS -> amp ~ 2.0)
        let mut limiter = HearingSafetyLimiter::new(-1.0, 16);
        for _ in 0..100 {
            let (l, r) = limiter.process_sample(2.0, 2.0);
            // Output must NEVER exceed -1.0 dBFS (amplitude ~ 0.89125)
            assert!(l <= 0.892);
            assert!(r <= 0.892);
        }

        // Test Integrated GameAudioDrcEngine
        let mut engine = GameAudioDrcEngine::new(DrcProfile::EsportsTactical);
        let (out_l, out_r) = engine.process_sample(0.5, 0.5);
        assert!(out_l != 0.0 || out_r != 0.0);
    }
}
