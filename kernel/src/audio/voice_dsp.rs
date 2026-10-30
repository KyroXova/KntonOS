//! Real-Time Low-Latency Game Voice Chat & Acoustic Noise Cancellation (DSP / RNNoise-Style) Filter Engine.
//!
//! Provides bare-metal voice audio DSP for handheld gaming communications:
//! - Circular microphone capture ring buffer (48 kHz 16-bit PCM).
//! - 10ms low-latency voice frames (480 samples).
//! - 16-band Bark-scale bandpass analysis filterbank for spectral decomposition.
//! - Voice Activity Detection (VAD) with energy thresholds, zero-crossing rates, and speech probability scoring.
//! - Spectral noise estimation and continuous background noise tracking with speech hangover hysteresis.
//! - Real-time spectral subtraction (Wiener-style multi-band gain attenuation) suppressing acoustic noise.
//! - Automatic Gain Control (AGC) with smooth envelope follower, vocal level leveling, and soft-saturation limiter.

#![allow(dead_code)]

extern crate alloc;

use crate::graphics::types::{cos_f32, sin_f32, sqrt_f32, TWO_PI};

// -----------------------------------------------------------------------------
// Voice Audio Constants
// -----------------------------------------------------------------------------

/// Voice chat sampling frequency in Hertz.
pub const VOICE_SAMPLE_RATE: u32 = 48000;

/// Number of audio samples in a 10-millisecond voice processing frame (480 samples).
pub const VOICE_FRAME_SAMPLES: usize = 480;

/// Circular voice capture ring buffer capacity (100ms buffer = 4800 samples).
pub const VOICE_CAPTURE_CAPACITY: usize = 4800;

/// Number of spectral Bark-scale analysis bands.
pub const NUM_SPECTRAL_BANDS: usize = 16;

/// Center frequencies for 16 Bark-scale critical voice analysis bands in Hz.
pub const BARK_BAND_FREQUENCIES: [f32; NUM_SPECTRAL_BANDS] = [
    150.0,   // Band 0: Sub-voice rumble / plosives / wind
    250.0,   // Band 1: Male fundamental voice range
    400.0,   // Band 2: Female fundamental voice range
    600.0,   // Band 3: First voice formant region
    850.0,   // Band 4: First voice formant region
    1150.0,  // Band 5: Second voice formant
    1500.0,  // Band 6: Second voice formant / intelligibility
    2000.0,  // Band 7: Vowel transitions
    2600.0,  // Band 8: Consonant articulation
    3400.0,  // Band 9: Telephone / communication presence
    4400.0,  // Band 10: Sibilance / fricatives (s, z)
    5600.0,  // Band 11: High sibilance
    7200.0,  // Band 12: High treble breath noise
    9500.0,  // Band 13: Air / hiss
    13000.0, // Band 14: Ultra-high frequencies
    18000.0, // Band 15: Nyquist edge
];

// -----------------------------------------------------------------------------
// Floating Point Math Helpers (no_std natural log and exponential)
// -----------------------------------------------------------------------------

/// Computes the natural logarithm ln(x) for x > 0 using IEEE 754 decomposition and Taylor expansion.
pub fn ln_f32(x: f32) -> f32 {
    if x <= 1e-12 {
        return -27.63; // ~ln(1e-12)
    }

    let bits = x.to_bits();
    let exp = ((bits >> 23) & 0xFF) as i32 - 127;
    let mantissa = f32::from_bits((bits & 0x007F_FFFF) | 0x3F80_0000); // Range [1.0, 2.0)

    let z = (mantissa - 1.0) / (mantissa + 1.0);
    let z2 = z * z;
    let z3 = z2 * z;
    let z5 = z3 * z2;
    let z7 = z5 * z2;

    let ln_m = 2.0 * (z + z3 / 3.0 + z5 / 5.0 + z7 / 7.0);
    (exp as f32) * 0.69314718 + ln_m
}

/// Computes log10(x) for x > 0.
pub fn log10_f32(x: f32) -> f32 {
    ln_f32(x) * 0.43429448
}

// -----------------------------------------------------------------------------
// Voice Channel Layout & PCM Frame
// -----------------------------------------------------------------------------

/// Voice capture channel layout configuration.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceChannels {
    Mono = 1,
    Stereo = 2,
}

/// 10ms Voice Audio PCM Frame holding 480 16-bit signed integer samples at 48 kHz.
#[derive(Debug, Clone, Copy)]
pub struct VoicePcmFrame {
    pub samples: [i16; VOICE_FRAME_SAMPLES],
    pub channels: VoiceChannels,
    pub timestamp_us: u64,
    pub sequence_number: u32,
}

impl VoicePcmFrame {
    pub const fn empty() -> Self {
        Self {
            samples: [0; VOICE_FRAME_SAMPLES],
            channels: VoiceChannels::Mono,
            timestamp_us: 0,
            sequence_number: 0,
        }
    }

    /// Convert 16-bit integer PCM samples to normalized float values in [-1.0, 1.0].
    pub fn to_float(&self, out_f: &mut [f32; VOICE_FRAME_SAMPLES]) {
        for (i, &s) in self.samples.iter().enumerate() {
            out_f[i] = (s as f32) / 32768.0;
        }
    }

    /// Construct a VoicePcmFrame from normalized float samples in [-1.0, 1.0].
    pub fn from_float(
        in_f: &[f32; VOICE_FRAME_SAMPLES],
        seq: u32,
        timestamp_us: u64,
    ) -> Self {
        let mut samples = [0i16; VOICE_FRAME_SAMPLES];
        for (i, &val) in in_f.iter().enumerate() {
            let clamped = val.clamp(-1.0, 1.0);
            samples[i] = (clamped * 32767.0) as i16;
        }

        Self {
            samples,
            channels: VoiceChannels::Mono,
            timestamp_us,
            sequence_number: seq,
        }
    }

    /// Calculate Root-Mean-Square (RMS) signal energy of the frame.
    pub fn rms_energy(&self) -> f32 {
        let mut sum_sq = 0.0f32;
        for &s in &self.samples {
            let norm = (s as f32) / 32768.0;
            sum_sq += norm * norm;
        }
        sqrt_f32(sum_sq / (VOICE_FRAME_SAMPLES as f32))
    }

    /// Calculate peak absolute amplitude of the frame.
    pub fn peak_amplitude(&self) -> i16 {
        let mut peak = 0i16;
        for &s in &self.samples {
            let abs_val = if s == i16::MIN { i16::MAX } else { s.abs() };
            if abs_val > peak {
                peak = abs_val;
            }
        }
        peak
    }
}

// -----------------------------------------------------------------------------
// Circular Microphone Capture Ring Buffer
// -----------------------------------------------------------------------------

/// Circular SPSC microphone capture ring buffer handling continuous PCM streaming.
pub struct VoiceCaptureRing {
    buffer: [i16; VOICE_CAPTURE_CAPACITY],
    write_ptr: usize,
    read_ptr: usize,
    count: usize,
    overflow_drops: u64,
    next_sequence: u32,
}

impl VoiceCaptureRing {
    pub const fn new() -> Self {
        Self {
            buffer: [0; VOICE_CAPTURE_CAPACITY],
            write_ptr: 0,
            read_ptr: 0,
            count: 0,
            overflow_drops: 0,
            next_sequence: 1,
        }
    }

    #[inline]
    pub fn count(&self) -> usize {
        self.count
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    #[inline]
    pub fn is_full(&self) -> bool {
        self.count == VOICE_CAPTURE_CAPACITY
    }

    #[inline]
    pub fn overflow_drops(&self) -> u64 {
        self.overflow_drops
    }

    /// Push raw 16-bit microphone capture samples into the ring buffer.
    pub fn push_samples(&mut self, samples: &[i16]) -> usize {
        let mut written = 0;
        for &sample in samples {
            if self.count >= VOICE_CAPTURE_CAPACITY {
                // Buffer overflow: drop incoming sample
                self.overflow_drops += 1;
            } else {
                self.buffer[self.write_ptr] = sample;
                self.write_ptr = (self.write_ptr + 1) % VOICE_CAPTURE_CAPACITY;
                self.count += 1;
                written += 1;
            }
        }
        written
    }

    /// Attempt to dequeue one complete 10ms frame (480 samples).
    pub fn pop_frame(&mut self, timestamp_us: u64) -> Option<VoicePcmFrame> {
        if self.count < VOICE_FRAME_SAMPLES {
            return None;
        }

        let mut frame_samples = [0i16; VOICE_FRAME_SAMPLES];
        for i in 0..VOICE_FRAME_SAMPLES {
            frame_samples[i] = self.buffer[self.read_ptr];
            self.read_ptr = (self.read_ptr + 1) % VOICE_CAPTURE_CAPACITY;
        }
        self.count -= VOICE_FRAME_SAMPLES;

        let seq = self.next_sequence;
        self.next_sequence += 1;

        Some(VoicePcmFrame {
            samples: frame_samples,
            channels: VoiceChannels::Mono,
            timestamp_us,
            sequence_number: seq,
        })
    }
}

// -----------------------------------------------------------------------------
// Bandpass Analysis & Synthesis Filterbank
// -----------------------------------------------------------------------------

/// 2nd-Order Biquad IIR Bandpass Filter (Direct Form II Transposed).
#[derive(Debug, Clone, Copy)]
pub struct BandpassFilter {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl BandpassFilter {
    /// Design a bandpass filter with specified center frequency, Q, and sample rate.
    pub fn new(center_hz: f32, q: f32, sample_rate: f32) -> Self {
        let w0 = TWO_PI * center_hz / sample_rate;
        let alpha = sin_f32(w0) / (2.0 * q);
        let cos_w0 = cos_f32(w0);

        let a0 = 1.0 + alpha;
        let b0 = alpha / a0;
        let b1 = 0.0;
        let b2 = -alpha / a0;
        let a1 = (-2.0 * cos_w0) / a0;
        let a2 = (1.0 - alpha) / a0;

        Self {
            b0,
            b1,
            b2,
            a1,
            a2,
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// Process a single audio sample through the bandpass filter.
    #[inline]
    pub fn process_sample(&mut self, x: f32) -> f32 {
        let out = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * out + self.z2;
        self.z2 = self.b2 * x - self.a2 * out;
        out
    }

    /// Reset filter state variables.
    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

/// 16-Band Spectral Analysis Filterbank decomposing 48 kHz voice audio into Bark frequency energies.
pub struct VoiceFilterbank {
    filters: [BandpassFilter; NUM_SPECTRAL_BANDS],
}

impl VoiceFilterbank {
    pub fn new() -> Self {
        let q = 2.0f32; // Moderate Q factor providing smooth overlapping critical bands
        let sample_rate = VOICE_SAMPLE_RATE as f32;

        let filters = [
            BandpassFilter::new(BARK_BAND_FREQUENCIES[0], q, sample_rate),
            BandpassFilter::new(BARK_BAND_FREQUENCIES[1], q, sample_rate),
            BandpassFilter::new(BARK_BAND_FREQUENCIES[2], q, sample_rate),
            BandpassFilter::new(BARK_BAND_FREQUENCIES[3], q, sample_rate),
            BandpassFilter::new(BARK_BAND_FREQUENCIES[4], q, sample_rate),
            BandpassFilter::new(BARK_BAND_FREQUENCIES[5], q, sample_rate),
            BandpassFilter::new(BARK_BAND_FREQUENCIES[6], q, sample_rate),
            BandpassFilter::new(BARK_BAND_FREQUENCIES[7], q, sample_rate),
            BandpassFilter::new(BARK_BAND_FREQUENCIES[8], q, sample_rate),
            BandpassFilter::new(BARK_BAND_FREQUENCIES[9], q, sample_rate),
            BandpassFilter::new(BARK_BAND_FREQUENCIES[10], q, sample_rate),
            BandpassFilter::new(BARK_BAND_FREQUENCIES[11], q, sample_rate),
            BandpassFilter::new(BARK_BAND_FREQUENCIES[12], q, sample_rate),
            BandpassFilter::new(BARK_BAND_FREQUENCIES[13], q, sample_rate),
            BandpassFilter::new(BARK_BAND_FREQUENCIES[14], q, sample_rate),
            BandpassFilter::new(BARK_BAND_FREQUENCIES[15], q, sample_rate),
        ];

        Self { filters }
    }

    /// Analyze a 10ms frame (480 float samples) and compute the energy in each of the 16 Bark bands.
    pub fn analyze_frame(
        &mut self,
        frame: &[f32; VOICE_FRAME_SAMPLES],
        energies_out: &mut [f32; NUM_SPECTRAL_BANDS],
    ) {
        let num_samples = VOICE_FRAME_SAMPLES as f32;

        for (band_idx, filter) in self.filters.iter_mut().enumerate() {
            let mut sum_sq = 0.0f32;
            for &sample in frame.iter() {
                let filtered = filter.process_sample(sample);
                sum_sq += filtered * filtered;
            }
            energies_out[band_idx] = sum_sq / num_samples;
        }
    }

    /// Reset all bandpass filter internal delay states.
    pub fn reset(&mut self) {
        for filter in &mut self.filters {
            filter.reset();
        }
    }
}

// -----------------------------------------------------------------------------
// Voice Activity Detection (VAD) & Zero-Crossing Rate
// -----------------------------------------------------------------------------

/// Computes normalized Zero-Crossing Rate (ZCR) of an audio frame in [0.0, 1.0].
pub fn compute_zero_crossing_rate(samples: &[f32; VOICE_FRAME_SAMPLES]) -> f32 {
    let mut zero_crossings = 0usize;
    for i in 1..VOICE_FRAME_SAMPLES {
        let prev = samples[i - 1];
        let curr = samples[i];
        if (prev >= 0.0 && curr < 0.0) || (prev < 0.0 && curr >= 0.0) {
            zero_crossings += 1;
        }
    }
    (zero_crossings as f32) / ((VOICE_FRAME_SAMPLES - 1) as f32)
}

/// Voice Activity Detection state machine status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadState {
    Silence,
    Speech,
    Hangover { remaining_frames: usize },
}

/// Result of a single VAD evaluation cycle.
#[derive(Debug, Clone, Copy)]
pub struct VadDecision {
    pub is_speech: bool,
    pub speech_probability: f32,
    pub frame_energy_db: f32,
    pub snr_db: f32,
    pub zero_crossing_rate: f32,
    pub state: VadState,
}

/// Voice Activity Detector evaluating energy, spectral distribution, and zero-crossing rates.
pub struct VoiceActivityDetector {
    state: VadState,
    noise_energy_floor: f32,
    energy_threshold_db: f32,
    hangover_duration_frames: usize,
    speech_probability_threshold: f32,
}

impl VoiceActivityDetector {
    pub fn new() -> Self {
        Self {
            state: VadState::Silence,
            noise_energy_floor: 1e-5, // Initial floor ~ -50 dBFS
            energy_threshold_db: 12.0, // Signal must exceed noise floor by >= 12 dB
            hangover_duration_frames: 6, // 6 frames = 60ms speech hangover
            speech_probability_threshold: 0.50,
        }
    }

    /// Analyze a frame and its spectral band energies, returning the VAD classification.
    pub fn analyze(
        &mut self,
        samples: &[f32; VOICE_FRAME_SAMPLES],
        band_energies: &[f32; NUM_SPECTRAL_BANDS],
    ) -> VadDecision {
        // 1. Frame total energy calculation
        let mut total_energy = 0.0f32;
        for &s in samples {
            total_energy += s * s;
        }
        let mean_energy = total_energy / (VOICE_FRAME_SAMPLES as f32);
        let safe_energy = mean_energy.max(1e-10);
        let frame_energy_db = 10.0 * log10_f32(safe_energy);

        // 2. SNR relative to tracked noise floor
        let snr = safe_energy / self.noise_energy_floor.max(1e-10);
        let snr_db = 10.0 * log10_f32(snr);

        // 3. Zero-crossing rate
        let zcr = compute_zero_crossing_rate(samples);

        // 4. Voice-band energy ratio (Bands 1 to 9: 250 Hz to 4 kHz contain vocal formants)
        let mut voice_band_sum = 0.0f32;
        let mut total_band_sum = 0.0f32;
        for (i, &e) in band_energies.iter().enumerate() {
            total_band_sum += e;
            if i >= 1 && i <= 9 {
                voice_band_sum += e;
            }
        }
        let voice_band_ratio = if total_band_sum > 1e-12 {
            voice_band_sum / total_band_sum
        } else {
            0.0
        };

        // 5. Speech Probability Score (heuristic combination of SNR, voice ratio, and ZCR)
        let mut prob = 0.0f32;
        if snr_db > 0.0 {
            prob += (snr_db / 30.0).clamp(0.0, 0.5); // Up to 0.5 from SNR
        }
        prob += (voice_band_ratio * 0.4).clamp(0.0, 0.4); // Up to 0.4 from voice formant concentration
        if zcr > 0.02 && zcr < 0.40 {
            prob += 0.1; // Typical voiced speech zero-crossing profile
        }
        let speech_probability = prob.clamp(0.0, 1.0);

        // 6. State Machine with Hangover Hysteresis
        let raw_speech = speech_probability >= self.speech_probability_threshold && snr_db >= self.energy_threshold_db;

        match self.state {
            VadState::Silence => {
                if raw_speech {
                    self.state = VadState::Speech;
                } else {
                    // Update noise floor during confirmed silence
                    self.noise_energy_floor = 0.95 * self.noise_energy_floor + 0.05 * safe_energy;
                }
            }
            VadState::Speech => {
                if !raw_speech {
                    self.state = VadState::Hangover {
                        remaining_frames: self.hangover_duration_frames,
                    };
                }
            }
            VadState::Hangover { remaining_frames } => {
                if raw_speech {
                    self.state = VadState::Speech;
                } else if remaining_frames > 1 {
                    self.state = VadState::Hangover {
                        remaining_frames: remaining_frames - 1,
                    };
                } else {
                    self.state = VadState::Silence;
                }
            }
        }

        let is_speech = match self.state {
            VadState::Speech | VadState::Hangover { .. } => true,
            VadState::Silence => false,
        };

        VadDecision {
            is_speech,
            speech_probability,
            frame_energy_db,
            snr_db,
            zero_crossing_rate: zcr,
            state: self.state,
        }
    }
}

// -----------------------------------------------------------------------------
// Spectral Background Noise Estimator
// -----------------------------------------------------------------------------

/// Continuous spectral noise estimator tracking acoustic noise floor per Bark band.
pub struct SpectralNoiseEstimator {
    pub noise_energies: [f32; NUM_SPECTRAL_BANDS],
    calibration_frames: usize,
    total_frames_processed: u64,
}

impl SpectralNoiseEstimator {
    pub fn new() -> Self {
        Self {
            noise_energies: [1e-5f32; NUM_SPECTRAL_BANDS],
            calibration_frames: 20, // First 20 frames (200ms) calibrate background noise
            total_frames_processed: 0,
        }
    }

    /// Update noise profile based on current frame band energies and VAD decision.
    pub fn update(&mut self, band_energies: &[f32; NUM_SPECTRAL_BANDS], vad: &VadDecision) {
        self.total_frames_processed += 1;

        if self.total_frames_processed <= (self.calibration_frames as u64) {
            // Rapid calibration phase: exponential moving average with alpha = 0.2
            for i in 0..NUM_SPECTRAL_BANDS {
                self.noise_energies[i] = 0.8 * self.noise_energies[i] + 0.2 * band_energies[i];
            }
        } else if !vad.is_speech {
            // Non-speech frames: adapt background noise with alpha = 0.05
            for i in 0..NUM_SPECTRAL_BANDS {
                self.noise_energies[i] = 0.95 * self.noise_energies[i] + 0.05 * band_energies[i];
            }
        } else {
            // Speech active: very slow background adaptation (alpha = 0.001) or minimum statistics
            for i in 0..NUM_SPECTRAL_BANDS {
                if band_energies[i] < self.noise_energies[i] {
                    // If current energy dropped below noise estimate, track downwards
                    self.noise_energies[i] = 0.95 * self.noise_energies[i] + 0.05 * band_energies[i];
                } else {
                    self.noise_energies[i] = 0.999 * self.noise_energies[i] + 0.001 * band_energies[i];
                }
            }
        }
    }

    /// Retrieve estimated noise energy for a specific Bark band.
    #[inline]
    pub fn band_noise_energy(&self, band: usize) -> f32 {
        if band < NUM_SPECTRAL_BANDS {
            self.noise_energies[band]
        } else {
            1e-5
        }
    }
}

// -----------------------------------------------------------------------------
// Real-Time Spectral Subtraction Noise Suppressor
// -----------------------------------------------------------------------------

/// Multi-band Wiener-style spectral subtraction acoustic noise suppressor.
pub struct SpectralNoiseSuppressor {
    over_subtraction_factor: f32, // Alpha factor (e.g. 1.3) to prevent musical noise
    min_gain_floor: f32,           // Minimum gain floor (e.g. 0.05 = -26 dB)
    smoothed_gains: [f32; NUM_SPECTRAL_BANDS],
}

impl SpectralNoiseSuppressor {
    pub fn new() -> Self {
        Self {
            over_subtraction_factor: 1.3,
            min_gain_floor: 0.05,
            smoothed_gains: [1.0f32; NUM_SPECTRAL_BANDS],
        }
    }

    /// Compute per-band Wiener gains from signal band energies and estimated noise energies.
    pub fn compute_band_gains(
        &mut self,
        signal_energies: &[f32; NUM_SPECTRAL_BANDS],
        noise_energies: &[f32; NUM_SPECTRAL_BANDS],
        vad: &VadDecision,
    ) -> [f32; NUM_SPECTRAL_BANDS] {
        let mut target_gains = [1.0f32; NUM_SPECTRAL_BANDS];

        for i in 0..NUM_SPECTRAL_BANDS {
            let sig_e = signal_energies[i].max(1e-10);
            let noise_e = noise_energies[i].max(1e-10);

            // Wiener gain with over-subtraction: G = max(G_min, 1 - alpha * N / S)
            let sub = 1.0 - (self.over_subtraction_factor * noise_e / sig_e);
            let mut gain = sub.clamp(self.min_gain_floor, 1.0);

            // During silence, apply aggressive attenuation towards min gain floor
            if !vad.is_speech {
                gain *= 0.1;
                gain = gain.max(self.min_gain_floor);
            }

            target_gains[i] = gain;

            // Attack/Release smoothing: fast attack (0.6), moderate release (0.2)
            let smooth_coeff = if target_gains[i] < self.smoothed_gains[i] { 0.4 } else { 0.2 };
            self.smoothed_gains[i] = (1.0 - smooth_coeff) * self.smoothed_gains[i] + smooth_coeff * target_gains[i];
        }

        self.smoothed_gains
    }

    /// Apply computed gains across the time-domain audio samples.
    pub fn apply_noise_suppression(
        &self,
        samples: &[f32; VOICE_FRAME_SAMPLES],
        band_gains: &[f32; NUM_SPECTRAL_BANDS],
        out_samples: &mut [f32; VOICE_FRAME_SAMPLES],
    ) {
        // Average spectral gain across the critical speech formant bands (Bands 1 to 9)
        let mut voice_gain_acc = 0.0f32;
        for i in 1..=9 {
            voice_gain_acc += band_gains[i];
        }
        let effective_gain = voice_gain_acc / 9.0;

        for (i, &s) in samples.iter().enumerate() {
            out_samples[i] = s * effective_gain;
        }
    }
}

// -----------------------------------------------------------------------------
// Automatic Gain Control (AGC) & Peak Limiter
// -----------------------------------------------------------------------------

/// Real-time Automatic Gain Control (AGC) maintaining consistent speech levels.
pub struct AutomaticGainControl {
    target_rms: f32,       // Target vocal RMS level (~ -16 dBFS = 0.158)
    envelope: f32,         // Signal envelope tracker
    current_gain: f32,     // Smoothed dynamic gain factor
    min_gain: f32,         // Min attenuation (0.25 = -12 dB)
    max_gain: f32,         // Max boost (4.0 = +12 dB)
}

impl AutomaticGainControl {
    pub fn new() -> Self {
        Self {
            target_rms: 0.16,
            envelope: 0.05,
            current_gain: 1.0,
            min_gain: 0.25,
            max_gain: 4.0,
        }
    }

    /// Process a frame through the AGC envelope follower and soft-saturation limiter.
    pub fn process(
        &mut self,
        samples: &[f32; VOICE_FRAME_SAMPLES],
        vad: &VadDecision,
        out_samples: &mut [f32; VOICE_FRAME_SAMPLES],
    ) -> f32 {
        // Calculate frame RMS
        let mut sum_sq = 0.0f32;
        for &s in samples {
            sum_sq += s * s;
        }
        let frame_rms = sqrt_f32(sum_sq / (VOICE_FRAME_SAMPLES as f32));

        // Update envelope follower only when speech is present (prevent noise pumping)
        if vad.is_speech {
            let attack = 0.1f32;
            let release = 0.02f32;
            let coeff = if frame_rms > self.envelope { attack } else { release };
            self.envelope = (1.0 - coeff) * self.envelope + coeff * frame_rms;

            // Target gain: target_rms / envelope
            let desired_gain = (self.target_rms / self.envelope.max(0.01)).clamp(self.min_gain, self.max_gain);
            self.current_gain = 0.90 * self.current_gain + 0.10 * desired_gain;
        }

        // Apply gain and soft-saturation peak limiter
        for (i, &s) in samples.iter().enumerate() {
            let amplified = s * self.current_gain;
            out_samples[i] = Self::soft_limit(amplified);
        }

        self.current_gain
    }

    /// Cubic soft-saturation limiter: keeps signal strictly within [-1.0, 1.0].
    #[inline]
    fn soft_limit(x: f32) -> f32 {
        if x > 1.0 {
            1.0
        } else if x < -1.0 {
            -1.0
        } else if x > 0.66 {
            // Soft knee compression: 0.66 + (x - 0.66) / (1.0 + (x - 0.66))
            let d = x - 0.66;
            0.66 + d / (1.0 + d * 0.5)
        } else if x < -0.66 {
            let d = -x - 0.66;
            -(0.66 + d / (1.0 + d * 0.5))
        } else {
            x
        }
    }
}

// -----------------------------------------------------------------------------
// Voice DSP Engine (Unified Processing Pipeline)
// -----------------------------------------------------------------------------

/// Comprehensive voice audio DSP pipeline integrating Capture, Filterbank, VAD,
/// Noise Cancellation, and Automatic Gain Control.
pub struct VoiceDspEngine {
    pub ring: VoiceCaptureRing,
    pub filterbank: VoiceFilterbank,
    pub vad: VoiceActivityDetector,
    pub noise_estimator: SpectralNoiseEstimator,
    pub noise_suppressor: SpectralNoiseSuppressor,
    pub agc: AutomaticGainControl,
    pub total_processed_frames: u64,
    pub last_speech_probability: f32,
    pub last_snr_db: f32,
    pub last_applied_gain: f32,
}

impl VoiceDspEngine {
    pub fn new() -> Self {
        Self {
            ring: VoiceCaptureRing::new(),
            filterbank: VoiceFilterbank::new(),
            vad: VoiceActivityDetector::new(),
            noise_estimator: SpectralNoiseEstimator::new(),
            noise_suppressor: SpectralNoiseSuppressor::new(),
            agc: AutomaticGainControl::new(),
            total_processed_frames: 0,
            last_speech_probability: 0.0,
            last_snr_db: 0.0,
            last_applied_gain: 1.0,
        }
    }

    /// Process a 10ms raw input PCM frame and return the noise-cancelled, level-controlled output frame.
    pub fn process_frame(&mut self, input: &VoicePcmFrame) -> VoicePcmFrame {
        let mut float_in = [0.0f32; VOICE_FRAME_SAMPLES];
        input.to_float(&mut float_in);

        // 1. Spectral filterbank analysis
        let mut band_energies = [0.0f32; NUM_SPECTRAL_BANDS];
        self.filterbank.analyze_frame(&float_in, &mut band_energies);

        // 2. Voice Activity Detection (VAD)
        let vad_decision = self.vad.analyze(&float_in, &band_energies);
        self.last_speech_probability = vad_decision.speech_probability;
        self.last_snr_db = vad_decision.snr_db;

        // 3. Spectral background noise estimation update
        self.noise_estimator.update(&band_energies, &vad_decision);

        // 4. Multi-band Wiener noise suppression gains
        let band_gains = self.noise_suppressor.compute_band_gains(
            &band_energies,
            &self.noise_estimator.noise_energies,
            &vad_decision,
        );

        let mut denoised = [0.0f32; VOICE_FRAME_SAMPLES];
        self.noise_suppressor.apply_noise_suppression(&float_in, &band_gains, &mut denoised);

        // 5. Automatic Gain Control (AGC) & Soft-Limiter
        let mut final_out = [0.0f32; VOICE_FRAME_SAMPLES];
        let agc_gain = self.agc.process(&denoised, &vad_decision, &mut final_out);
        self.last_applied_gain = agc_gain;

        self.total_processed_frames += 1;

        VoicePcmFrame::from_float(&final_out, input.sequence_number, input.timestamp_us)
    }
}

// -----------------------------------------------------------------------------
// Unit Self-Tests for Commits 1, 2, and 3
// -----------------------------------------------------------------------------

/// Verify VoicePcmFrame conversions, capture ring buffer operations, and filterbank energy decomposition.
pub fn test_voice_capture_ring_and_filterbank() -> bool {
    // 1. Test VoicePcmFrame conversion and RMS calculation
    let mut float_input = [0.0f32; VOICE_FRAME_SAMPLES];
    // Generate 1 kHz test sine wave: f = 1000 Hz, fs = 48000 Hz -> T = 48 samples
    for i in 0..VOICE_FRAME_SAMPLES {
        let phase = TWO_PI * 1000.0 * (i as f32) / (VOICE_SAMPLE_RATE as f32);
        float_input[i] = 0.5 * sin_f32(phase);
    }

    let frame = VoicePcmFrame::from_float(&float_input, 1, 10_000);
    if frame.sequence_number != 1 || frame.timestamp_us != 10_000 {
        return false;
    }

    // Sine wave of amplitude 0.5 has theoretical RMS = 0.5 / sqrt(2) ~= 0.3535
    let rms = frame.rms_energy();
    if rms < 0.30 || rms > 0.40 {
        return false;
    }

    // Peak amplitude should be ~16383
    let peak = frame.peak_amplitude();
    if peak < 15000 || peak > 17000 {
        return false;
    }

    // Convert back to float and check accuracy
    let mut float_output = [0.0f32; VOICE_FRAME_SAMPLES];
    frame.to_float(&mut float_output);
    if (float_output[12] - float_input[12]).abs() > 0.01 {
        return false;
    }

    // 2. Test VoiceCaptureRing buffer push and pop
    let mut ring = VoiceCaptureRing::new();
    if !ring.is_empty() || ring.count() != 0 {
        return false;
    }

    // Attempt to pop when empty should return None
    if ring.pop_frame(1000).is_some() {
        return false;
    }

    // Push 480 samples
    let written = ring.push_samples(&frame.samples);
    if written != VOICE_FRAME_SAMPLES || ring.count() != VOICE_FRAME_SAMPLES {
        return false;
    }

    // Now pop 1 frame
    let popped = match ring.pop_frame(20_000) {
        Some(f) => f,
        None => return false,
    };
    if popped.sequence_number != 1 || popped.timestamp_us != 20_000 {
        return false;
    }
    if !ring.is_empty() {
        return false;
    }

    // 3. Test VoiceFilterbank energy localization on 1 kHz sine wave
    let mut filterbank = VoiceFilterbank::new();
    let mut band_energies = [0.0f32; NUM_SPECTRAL_BANDS];

    // Prime the filterbank with 3 frames of 1000 Hz signal
    for _ in 0..3 {
        filterbank.analyze_frame(&float_input, &mut band_energies);
    }

    // 1000 Hz falls directly between Band 4 (850 Hz) and Band 5 (1150 Hz).
    // The energy in Band 4 and Band 5 should be significantly higher than Band 0 (150 Hz) or Band 15 (18000 Hz).
    let band_mid_energy = band_energies[4].max(band_energies[5]);
    let band_low_energy = band_energies[0];
    let band_high_energy = band_energies[15];

    if band_mid_energy <= band_low_energy * 5.0 {
        return false;
    }
    if band_mid_energy <= band_high_energy * 10.0 {
        return false;
    }

    true
}

/// Verify Zero-Crossing Rate, Voice Activity Detection (VAD) speech discrimination,
/// hangover hysteresis, and spectral noise estimation.
pub fn test_vad_and_noise_estimator() -> bool {
    let mut vad = VoiceActivityDetector::new();
    let mut noise_estimator = SpectralNoiseEstimator::new();
    let mut filterbank = VoiceFilterbank::new();

    // 1. Verify Zero-Crossing Rate
    // DC signal (constant 0.5) has ZCR = 0.0
    let dc_frame = [0.5f32; VOICE_FRAME_SAMPLES];
    if compute_zero_crossing_rate(&dc_frame) != 0.0 {
        return false;
    }

    // High-frequency alternating signal has ZCR = 1.0
    let mut alt_frame = [0.0f32; VOICE_FRAME_SAMPLES];
    for i in 0..VOICE_FRAME_SAMPLES {
        alt_frame[i] = if i % 2 == 0 { 0.5 } else { -0.5 };
    }
    if compute_zero_crossing_rate(&alt_frame) != 1.0 {
        return false;
    }

    // 2. Test Silence Discrimination
    // Generate low-amplitude background white noise ~ -60 dBFS (amplitude 0.001)
    let mut silence_frame = [0.0f32; VOICE_FRAME_SAMPLES];
    for i in 0..VOICE_FRAME_SAMPLES {
        silence_frame[i] = if i % 2 == 0 { 0.001 } else { -0.001 };
    }
    let mut silence_energies = [0.0f32; NUM_SPECTRAL_BANDS];
    filterbank.analyze_frame(&silence_frame, &mut silence_energies);

    // Initial frames in silence: VAD should report is_speech = false
    let decision = vad.analyze(&silence_frame, &silence_energies);
    noise_estimator.update(&silence_energies, &decision);

    if decision.is_speech || decision.speech_probability > 0.3 {
        return false;
    }

    // 3. Test Speech Discrimination on synthetic voice-like signal (400 Hz vowel + 850 Hz formant, amp = 0.4)
    let mut voice_frame = [0.0f32; VOICE_FRAME_SAMPLES];
    for i in 0..VOICE_FRAME_SAMPLES {
        let t = (i as f32) / (VOICE_SAMPLE_RATE as f32);
        let f1 = sin_f32(TWO_PI * 400.0 * t);
        let f2 = 0.5 * sin_f32(TWO_PI * 850.0 * t);
        voice_frame[i] = 0.3 * (f1 + f2);
    }
    let mut voice_energies = [0.0f32; NUM_SPECTRAL_BANDS];
    filterbank.analyze_frame(&voice_frame, &mut voice_energies);

    let voice_decision = vad.analyze(&voice_frame, &voice_energies);
    noise_estimator.update(&voice_energies, &voice_decision);

    if !voice_decision.is_speech || voice_decision.speech_probability < 0.5 {
        return false;
    }
    if voice_decision.state != VadState::Speech {
        return false;
    }

    // 4. Test Hangover Hysteresis
    // Feed silence frame immediately after speech frame: should enter Hangover state
    let hangover_decision = vad.analyze(&silence_frame, &silence_energies);
    if !hangover_decision.is_speech {
        return false; // Must remain speech during hangover
    }
    match hangover_decision.state {
        VadState::Hangover { remaining_frames } => {
            if remaining_frames != 5 {
                return false;
            }
        }
        _ => return false,
    }

    // Exhaust remaining 5 hangover frames
    for _ in 0..5 {
        let d = vad.analyze(&silence_frame, &silence_energies);
        noise_estimator.update(&silence_energies, &d);
    }

    // After 6 total silent frames, state must return to Silence
    let silent_final = vad.analyze(&silence_frame, &silence_energies);
    if silent_final.is_speech || silent_final.state != VadState::Silence {
        return false;
    }

    // 5. Test Spectral Noise Estimator tracking
    // Verify noise energies in bands 2 and 4 did not get blown out by the voice frame
    if noise_estimator.band_noise_energy(2) > 0.01 {
        return false;
    }

    true
}

/// Verify spectral subtraction noise suppression, Automatic Gain Control (AGC), and peak limiting.
pub fn test_noise_suppressor_and_agc() -> bool {
    let mut engine = VoiceDspEngine::new();

    // 1. Calibrate engine with 20 frames of stationary fan/hiss noise (amplitude 0.05)
    let mut noise_samples = [0.0f32; VOICE_FRAME_SAMPLES];
    for i in 0..VOICE_FRAME_SAMPLES {
        let t = (i as f32) / (VOICE_SAMPLE_RATE as f32);
        // Multitone background noise: 120 Hz hum + 3400 Hz fan hiss
        noise_samples[i] = 0.03 * sin_f32(TWO_PI * 120.0 * t) + 0.02 * sin_f32(TWO_PI * 3400.0 * t);
    }
    let noise_frame = VoicePcmFrame::from_float(&noise_samples, 0, 0);

    for i in 0..20 {
        let mut f = noise_frame;
        f.sequence_number = i as u32;
        let _ = engine.process_frame(&f);
    }

    // Process one more noise frame after calibration: noise suppression must attenuate noise significantly
    let post_cal_noise = engine.process_frame(&noise_frame);
    let post_rms = post_cal_noise.rms_energy();
    let initial_rms = noise_frame.rms_energy();

    // Post-suppression RMS should be attenuated by at least 6 dB (halved amplitude)
    if post_rms > initial_rms * 0.60 {
        return false;
    }

    // 2. Test Voice with Background Noise: Voice (400 Hz vowel, amp 0.35) + Noise (amp 0.05)
    let mut speech_plus_noise = [0.0f32; VOICE_FRAME_SAMPLES];
    for i in 0..VOICE_FRAME_SAMPLES {
        let t = (i as f32) / (VOICE_SAMPLE_RATE as f32);
        let voice = 0.30 * sin_f32(TWO_PI * 400.0 * t) + 0.15 * sin_f32(TWO_PI * 850.0 * t);
        let noise = 0.03 * sin_f32(TWO_PI * 120.0 * t) + 0.02 * sin_f32(TWO_PI * 3400.0 * t);
        speech_plus_noise[i] = voice + noise;
    }
    let speech_frame = VoicePcmFrame::from_float(&speech_plus_noise, 25, 250_000);

    let output_speech = engine.process_frame(&speech_frame);

    // VAD should recognize speech and probability should be high
    if engine.last_speech_probability < 0.5 {
        return false;
    }
    // Output should retain strong speech amplitude
    if output_speech.rms_energy() < 0.10 {
        return false;
    }

    // 3. Test AGC on soft/whispered speech (amplitude 0.02)
    let mut soft_speech = [0.0f32; VOICE_FRAME_SAMPLES];
    for i in 0..VOICE_FRAME_SAMPLES {
        let t = (i as f32) / (VOICE_SAMPLE_RATE as f32);
        soft_speech[i] = 0.02 * sin_f32(TWO_PI * 400.0 * t);
    }
    let mut agc = AutomaticGainControl::new();
    let vad_active = VadDecision {
        is_speech: true,
        speech_probability: 0.85,
        frame_energy_db: -30.0,
        snr_db: 20.0,
        zero_crossing_rate: 0.05,
        state: VadState::Speech,
    };

    let mut agc_out = [0.0f32; VOICE_FRAME_SAMPLES];
    let mut last_gain = 1.0;
    for _ in 0..10 {
        last_gain = agc.process(&soft_speech, &vad_active, &mut agc_out);
    }

    // AGC should have boosted gain above 1.5x for soft speech
    if last_gain < 1.5 {
        return false;
    }

    // 4. Test Soft-Saturation Limiter with loud input (amplitude 2.5)
    let loud_speech = [2.5f32; VOICE_FRAME_SAMPLES];
    let mut limited_out = [0.0f32; VOICE_FRAME_SAMPLES];
    agc.process(&loud_speech, &vad_active, &mut limited_out);

    for &s in &limited_out {
        if s > 1.0 || s < -1.0 {
            return false;
        }
    }

    true
}
