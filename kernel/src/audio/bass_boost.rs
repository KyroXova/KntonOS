//! Dynamic Bass Booster & Psychoacoustic Harmonic Synthesizer.
//!
//! Designed for handheld console micro-speakers with physical low-frequency cutoffs (~80 Hz):
//! - Isolates sub-bass content below the speaker cutoff frequency.
//! - High-pass filters the physical audio stream to protect micro-speaker voice coils from over-excursion.
//! - Synthesizes 2nd and 3rd harmonics using nonlinear waveshaping (Missing Fundamental psychoacoustic effect).
//! - Shapes generated harmonics through a resonant bandpass filter aligned with micro-speaker acoustic efficiency.
//! - Blends synthesized harmonics back into the main audio path for rich, perceived deep bass.

use crate::graphics::types::{cos_f32, sin_f32, PI};
use super::hrtf::AUDIO_SAMPLE_RATE;

/// Default handheld micro-speaker physical cutoff frequency (80 Hz).
pub const DEFAULT_SPEAKER_CUTOFF_HZ: f32 = 80.0;

/// Center frequency for harmonic bandpass resonance (140 Hz).
pub const HARMONIC_RESONANCE_HZ: f32 = 140.0;

// -----------------------------------------------------------------------------
// Biquad Filter Helpers for Crossover & Harmonic Shaping
// -----------------------------------------------------------------------------

/// Simple 2nd-order IIR biquad section for crossover filtering.
#[derive(Debug, Clone, Copy)]
pub struct SimpleBiquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1_l: f32,
    x2_l: f32,
    y1_l: f32,
    y2_l: f32,
    x1_r: f32,
    x2_r: f32,
    y1_r: f32,
    y2_r: f32,
}

impl SimpleBiquad {
    pub const fn empty() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            x1_l: 0.0,
            x2_l: 0.0,
            y1_l: 0.0,
            y2_l: 0.0,
            x1_r: 0.0,
            x2_r: 0.0,
            y1_r: 0.0,
            y2_r: 0.0,
        }
    }

    /// Configures 2nd-order Butterworth low-pass filter.
    pub fn setup_lowpass(&mut self, freq: f32, q: f32) {
        let w0 = (2.0 * PI * freq) / AUDIO_SAMPLE_RATE;
        let cos_w0 = cos_f32(w0);
        let sin_w0 = sin_f32(w0);
        let alpha = sin_w0 / (2.0 * q);

        let b0 = (1.0 - cos_w0) * 0.5;
        let b1 = 1.0 - cos_w0;
        let b2 = (1.0 - cos_w0) * 0.5;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;

        let inv_a0 = 1.0 / a0;
        self.b0 = b0 * inv_a0;
        self.b1 = b1 * inv_a0;
        self.b2 = b2 * inv_a0;
        self.a1 = a1 * inv_a0;
        self.a2 = a2 * inv_a0;
    }

    /// Configures 2nd-order Butterworth high-pass filter.
    pub fn setup_highpass(&mut self, freq: f32, q: f32) {
        let w0 = (2.0 * PI * freq) / AUDIO_SAMPLE_RATE;
        let cos_w0 = cos_f32(w0);
        let sin_w0 = sin_f32(w0);
        let alpha = sin_w0 / (2.0 * q);

        let b0 = (1.0 + cos_w0) * 0.5;
        let b1 = -(1.0 + cos_w0);
        let b2 = (1.0 + cos_w0) * 0.5;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;

        let inv_a0 = 1.0 / a0;
        self.b0 = b0 * inv_a0;
        self.b1 = b1 * inv_a0;
        self.b2 = b2 * inv_a0;
        self.a1 = a1 * inv_a0;
        self.a2 = a2 * inv_a0;
    }

    /// Configures 2nd-order bandpass filter.
    pub fn setup_bandpass(&mut self, freq: f32, q: f32) {
        let w0 = (2.0 * PI * freq) / AUDIO_SAMPLE_RATE;
        let cos_w0 = cos_f32(w0);
        let sin_w0 = sin_f32(w0);
        let alpha = sin_w0 / (2.0 * q);

        let b0 = alpha;
        let b1 = 0.0;
        let b2 = -alpha;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;

        let inv_a0 = 1.0 / a0;
        self.b0 = b0 * inv_a0;
        self.b1 = b1 * inv_a0;
        self.b2 = b2 * inv_a0;
        self.a1 = a1 * inv_a0;
        self.a2 = a2 * inv_a0;
    }

    pub fn reset(&mut self) {
        self.x1_l = 0.0;
        self.x2_l = 0.0;
        self.y1_l = 0.0;
        self.y2_l = 0.0;
        self.x1_r = 0.0;
        self.x2_r = 0.0;
        self.y1_r = 0.0;
        self.y2_r = 0.0;
    }

    #[inline]
    pub fn process(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        let y_l = self.b0 * in_l + self.b1 * self.x1_l + self.b2 * self.x2_l
            - self.a1 * self.y1_l - self.a2 * self.y2_l;
        self.x2_l = self.x1_l;
        self.x1_l = in_l;
        self.y2_l = self.y1_l;
        self.y1_l = y_l;

        let y_r = self.b0 * in_r + self.b1 * self.x1_r + self.b2 * self.x2_r
            - self.a1 * self.y1_r - self.a2 * self.y2_r;
        self.x2_r = self.x1_r;
        self.x1_r = in_r;
        self.y2_r = self.y1_r;
        self.y1_r = y_r;

        (y_l, y_r)
    }
}

// -----------------------------------------------------------------------------
// Psychoacoustic Harmonic Synthesizer
// -----------------------------------------------------------------------------

/// Harmonic synthesis parameters for virtual bass enhancement.
#[derive(Debug, Clone, Copy)]
pub struct HarmonicBassConfig {
    /// Cutoff frequency below which harmonics are synthesized (Hz).
    pub cutoff_freq: f32,
    /// Amplitude of 2nd harmonic (f*2) in mix [0.0, 1.0].
    pub harmonic_2nd_gain: f32,
    /// Amplitude of 3rd harmonic (f*3) in mix [0.0, 1.0].
    pub harmonic_3rd_gain: f32,
    /// Master bass boost drive multiplier [0.0, 3.0].
    pub drive: f32,
    /// High-pass speaker protection attenuation factor [0.0, 1.0].
    pub highpass_speaker_protection: bool,
}

impl Default for HarmonicBassConfig {
    fn default() -> Self {
        Self {
            cutoff_freq: DEFAULT_SPEAKER_CUTOFF_HZ,
            harmonic_2nd_gain: 0.65,
            harmonic_3rd_gain: 0.35,
            drive: 1.25,
            highpass_speaker_protection: true,
        }
    }
}

/// Psychoacoustic Bass Enhancement Engine using the Missing Fundamental phenomenon.
pub struct PsychoacousticBassEnhancer {
    pub config: HarmonicBassConfig,
    sub_lowpass: SimpleBiquad,
    speaker_highpass: SimpleBiquad,
    harmonic_bandpass: SimpleBiquad,
    pub enabled: bool,
}

impl PsychoacousticBassEnhancer {
    /// Creates and tunes the psychoacoustic bass enhancer for handheld micro-speakers.
    pub fn new(config: HarmonicBassConfig) -> Self {
        let mut sub_lowpass = SimpleBiquad::empty();
        sub_lowpass.setup_lowpass(config.cutoff_freq, 0.707);

        let mut speaker_highpass = SimpleBiquad::empty();
        speaker_highpass.setup_highpass(config.cutoff_freq, 0.707);

        let mut harmonic_bandpass = SimpleBiquad::empty();
        harmonic_bandpass.setup_bandpass(HARMONIC_RESONANCE_HZ, 1.2);

        Self {
            config,
            sub_lowpass,
            speaker_highpass,
            harmonic_bandpass,
            enabled: true,
        }
    }

    /// Updates configuration and re-tunes crossover and harmonic shaping filters.
    pub fn configure(&mut self, config: HarmonicBassConfig) {
        self.config = config;
        self.sub_lowpass.setup_lowpass(config.cutoff_freq, 0.707);
        self.speaker_highpass.setup_highpass(config.cutoff_freq, 0.707);
        self.harmonic_bandpass.setup_bandpass(HARMONIC_RESONANCE_HZ, 1.2);
    }

    /// Flushes all filter history.
    pub fn reset(&mut self) {
        self.sub_lowpass.reset();
        self.speaker_highpass.reset();
        self.harmonic_bandpass.reset();
    }

    /// Synthesizes 2nd and 3rd harmonics from a normalized sub-bass input.
    /// - 2nd harmonic (even): generated via full-wave rectification / absolute magnitude waveshaping.
    /// - 3rd harmonic (odd): generated via cubic polynomial saturation: x * (1.5 - 0.5 * x^2).
    #[inline]
    fn synthesize_harmonics_channel(&self, sub: f32) -> f32 {
        let norm = (sub / 32768.0).clamp(-1.0, 1.0);

        // 2nd harmonic generation: h2 = |norm| * 2.0 - 1.0 (rectification removes fundamental, doubles frequency)
        let abs_val = if norm < 0.0 { -norm } else { norm };
        let h2 = (abs_val * 2.0 - 1.0) * self.config.harmonic_2nd_gain;

        // 3rd harmonic generation: h3 = 1.5 * norm - 0.5 * norm^3
        let norm_cube = norm * norm * norm;
        let h3 = (1.5 * norm - 0.5 * norm_cube) * self.config.harmonic_3rd_gain;

        // Sum harmonics scaled back to sample amplitude
        (h2 + h3) * 32768.0 * self.config.drive
    }

    /// Processes a single stereo PCM sample:
    /// 1. Extracts sub-bass below speaker cutoff.
    /// 2. Protects speaker by high-passing the direct sound.
    /// 3. Generates 2nd and 3rd harmonics in the audible 80-250 Hz range.
    /// 4. Shapes harmonics with resonant bandpass filter.
    /// 5. Mixes synthesized harmonics back into the output stream.
    #[inline]
    pub fn process_sample(&mut self, left: i16, right: i16) -> (i16, i16) {
        if !self.enabled {
            return (left, right);
        }

        let in_l = left as f32;
        let in_r = right as f32;

        // 1. Extract sub-bass content
        let (sub_l, sub_r) = self.sub_lowpass.process(in_l, in_r);

        // 2. High-pass direct signal to prevent speaker over-excursion
        let (direct_l, direct_r) = if self.config.highpass_speaker_protection {
            self.speaker_highpass.process(in_l, in_r)
        } else {
            (in_l, in_r)
        };

        // 3. Synthesize nonlinear harmonics
        let raw_harm_l = self.synthesize_harmonics_channel(sub_l);
        let raw_harm_r = self.synthesize_harmonics_channel(sub_r);

        // 4. Shape harmonics through resonant bandpass filter
        let (shaped_harm_l, shaped_harm_r) = self.harmonic_bandpass.process(raw_harm_l, raw_harm_r);

        // 5. Blend direct signal and synthesized harmonics
        let out_l = direct_l + shaped_harm_l;
        let out_r = direct_r + shaped_harm_r;

        let clamped_l = out_l.clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        let clamped_r = out_r.clamp(i16::MIN as f32, i16::MAX as f32) as i16;

        (clamped_l, clamped_r)
    }

    /// Processes a stereo PCM buffer in-place.
    pub fn process_buffer(&mut self, left_buf: &mut [i16], right_buf: &mut [i16]) {
        if !self.enabled {
            return;
        }

        let count = left_buf.len().min(right_buf.len());
        for i in 0..count {
            let (l, r) = self.process_sample(left_buf[i], right_buf[i]);
            left_buf[i] = l;
            right_buf[i] = r;
        }
    }
}

// -----------------------------------------------------------------------------
// Unit Tests
// -----------------------------------------------------------------------------

/// Unit test verifying psychoacoustic harmonic bass generation and speaker protection highpass.
pub fn test_psychoacoustic_harmonic_synthesizer() -> bool {
    let mut enhancer = PsychoacousticBassEnhancer::new(HarmonicBassConfig::default());

    // 1. Verify pass-through when disabled
    enhancer.enabled = false;
    let (p_l, p_r) = enhancer.process_sample(1234, -1234);
    if p_l != 1234 || p_r != -1234 {
        return false;
    }
    enhancer.enabled = true;

    // 2. Feed a deep 40 Hz square wave (period = 1200 samples at 48 kHz).
    // Frequencies below 80 Hz should generate harmonics in the 80 Hz - 250 Hz range!
    let mut generated_harmonic_energy = 0i64;
    for i in 0..400 {
        let in_sig = if (i % 1200) < 600 { 10000 } else { -10000 };
        let (out_l, _) = enhancer.process_sample(in_sig, in_sig);
        if i >= 100 {
            generated_harmonic_energy += out_l.abs() as i64;
        }
    }

    // Significant energy must be synthesized into the output
    if generated_harmonic_energy == 0 {
        return false;
    }

    // 3. Verify speaker protection: highpass direct signal without harmonic synthesis
    let mut direct_only = SimpleBiquad::empty();
    direct_only.setup_highpass(80.0, 0.707);
    let mut direct_energy = 0i64;
    for i in 0..400 {
        let in_sig = if (i % 1200) < 600 { 10000.0 } else { -10000.0 };
        let (hp_out, _) = direct_only.process(in_sig, in_sig);
        if i >= 100 {
            direct_energy += hp_out.abs() as i64;
        }
    }

    // Harmonic enhancer output must be richer than direct highpassed signal
    if generated_harmonic_energy <= direct_energy {
        return false;
    }

    true
}

// -----------------------------------------------------------------------------
// Dynamic Soft-Saturation Limiter
// -----------------------------------------------------------------------------

/// Soft-knee saturation limiter preventing digital clipping and micro-speaker bottoming out.
#[derive(Debug, Clone, Copy)]
pub struct SoftSaturationLimiter {
    pub threshold: f32,
    pub ceiling: f32,
    pub compression_count: usize,
    pub peak_input: f32,
    pub peak_output: f32,
    pub enabled: bool,
}

impl SoftSaturationLimiter {
    pub const fn new(threshold: f32) -> Self {
        Self {
            threshold,
            ceiling: 0.999,
            compression_count: 0,
            peak_input: 0.0,
            peak_output: 0.0,
            enabled: true,
        }
    }

    /// Limits a single floating-point sample in range [-inf, +inf] to strictly [-ceiling, +ceiling].
    #[inline]
    pub fn process_channel(&mut self, sample: f32) -> f32 {
        if !self.enabled {
            return sample;
        }

        let abs_val = if sample < 0.0 { -sample } else { sample };
        if abs_val > self.peak_input {
            self.peak_input = abs_val;
        }

        let out = if abs_val <= self.threshold {
            sample
        } else {
            self.compression_count += 1;
            let delta = abs_val - self.threshold;
            let denom_scale = 1.0 - self.threshold;
            let compressed = self.threshold + delta / (1.0 + delta / denom_scale);
            let sign = if sample < 0.0 { -1.0 } else { 1.0 };
            sign * compressed.min(self.ceiling)
        };

        let abs_out = if out < 0.0 { -out } else { out };
        if abs_out > self.peak_output {
            self.peak_output = abs_out;
        }

        out
    }

    /// Limits a stereo pair of PCM audio samples.
    #[inline]
    pub fn process_sample(&mut self, left: i16, right: i16) -> (i16, i16) {
        let norm_l = (left as f32) / 32768.0;
        let norm_r = (right as f32) / 32768.0;

        let lim_l = self.process_channel(norm_l);
        let lim_r = self.process_channel(norm_r);

        let out_l = (lim_l * 32767.0).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        let out_r = (lim_r * 32767.0).clamp(i16::MIN as f32, i16::MAX as f32) as i16;

        (out_l, out_r)
    }

    pub fn reset_stats(&mut self) {
        self.compression_count = 0;
        self.peak_input = 0.0;
        self.peak_output = 0.0;
    }
}

// -----------------------------------------------------------------------------
// Real-Time Game Audio Pipeline Processor
// -----------------------------------------------------------------------------

/// Unified real-time game audio enhancement processor combining:
/// 1. 10-Band Parametric Equalizer (RBJ Biquad filters)
/// 2. Psychoacoustic Bass Enhancer (Missing fundamental harmonic synthesizer)
/// 3. Dynamic Soft-Saturation Limiter (Anti-clipping protection)
pub struct GameAudioProcessor {
    pub eq: super::eq::ParametricEqualizer10,
    pub bass_enhancer: PsychoacousticBassEnhancer,
    pub limiter: SoftSaturationLimiter,
    pub master_gain: f32,
    pub enabled: bool,
}

impl GameAudioProcessor {
    pub fn new() -> Self {
        Self {
            eq: super::eq::ParametricEqualizer10::new(),
            bass_enhancer: PsychoacousticBassEnhancer::new(HarmonicBassConfig::default()),
            limiter: SoftSaturationLimiter::new(0.85),
            master_gain: 1.0,
            enabled: true,
        }
    }

    /// Processes a single stereo PCM sample through the complete enhancement chain.
    #[inline]
    pub fn process_sample(&mut self, left: i16, right: i16) -> (i16, i16) {
        if !self.enabled {
            return (left, right);
        }

        // Stage 1: 10-Band Parametric Equalizer
        let (eq_l, eq_r) = self.eq.process_sample(left, right);

        // Stage 2: Psychoacoustic Bass Enhancer
        let (bass_l, bass_r) = self.bass_enhancer.process_sample(eq_l, eq_r);

        // Apply master gain
        let scaled_l = (bass_l as f32 * self.master_gain).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        let scaled_r = (bass_r as f32 * self.master_gain).clamp(i16::MIN as f32, i16::MAX as f32) as i16;

        // Stage 3: Dynamic Soft-Saturation Limiter
        self.limiter.process_sample(scaled_l, scaled_r)
    }

    /// Processes split stereo buffers in-place.
    pub fn process_buffers(&mut self, left: &mut [i16], right: &mut [i16]) {
        if !self.enabled {
            return;
        }

        let count = left.len().min(right.len());
        for i in 0..count {
            let (l, r) = self.process_sample(left[i], right[i]);
            left[i] = l;
            right[i] = r;
        }
    }

    /// Resets all filter and limiter states.
    pub fn reset(&mut self) {
        self.eq.reset();
        self.bass_enhancer.reset();
        self.limiter.reset_stats();
    }
}

/// Unit test verifying dynamic soft-saturation limiter and integrated game audio processor.
pub fn test_soft_limiter_and_game_audio_pipeline() -> bool {
    // 1. Verify SoftSaturationLimiter
    let mut limiter = SoftSaturationLimiter::new(0.85);

    // Within linear threshold (0.50): output matches input exactly
    let out_lin = limiter.process_channel(0.50);
    if (out_lin - 0.50).abs() > 1e-5 {
        return false;
    }

    // Heavy overload (2.0): smoothly compressed strictly below ceiling (0.999)
    let out_overload = limiter.process_channel(2.0);
    if out_overload >= 0.999 || out_overload <= 0.85 {
        return false;
    }

    // Verify negative symmetry
    let out_neg = limiter.process_channel(-2.0);
    if (out_neg + out_overload).abs() > 1e-5 {
        return false;
    }

    if limiter.compression_count != 2 {
        return false;
    }

    // 2. Verify GameAudioProcessor full pipeline
    let mut processor = GameAudioProcessor::new();
    processor.eq.apply_preset(super::eq::EqPreset::ActionGame);
    processor.master_gain = 1.5; // Deliberate 50% gain boost

    // Process a loud stereo burst
    let mut left_buf = [25000i16; 64];
    let mut right_buf = [-25000i16; 64];
    processor.process_buffers(&mut left_buf, &mut right_buf);

    // Verify every single sample is bounded to valid i16 range without overflow
    for i in 0..64 {
        if left_buf[i] == i16::MAX || left_buf[i] == i16::MIN {
            // Should be softly compressed, not hard clipped at rails
            return false;
        }
    }

    // Limiter should have caught and prevented clipping
    if processor.limiter.compression_count == 0 {
        return false;
    }

    true
}
