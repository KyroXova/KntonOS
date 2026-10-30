//! Real-Time Low-Latency Game Voice Chat & Acoustic Noise Cancellation (DSP / RNNoise-Style) Filter Engine.
//!
//! Provides bare-metal voice audio DSP for handheld gaming communications:
//! - Circular microphone capture ring buffer (48 kHz 16-bit PCM).
//! - 10ms low-latency voice frames (480 samples).
//! - 16-band Bark-scale bandpass analysis filterbank for spectral decomposition.
//! - Voice Activity Detection (VAD) with energy thresholds and zero-crossing rates.
//! - Spectral noise estimation, continuous background noise tracking, and Wiener-style subtraction.
//! - Real-time Automatic Gain Control (AGC) with smooth envelope follower and soft-saturation limiter.

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
// Bandpass Analysis Filterbank
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
// Unit Self-Tests for Commit 1
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
