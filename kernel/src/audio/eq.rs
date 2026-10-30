//! Parametric 10-Band Audio Equalizer & Biquad IIR Filter Engine.
//! Implements Robert Bristow-Johnson (RBJ) Audio EQ Cookbook biquad filters:

use crate::graphics::types::{cos_f32, sin_f32, sqrt_f32, PI};
use super::hrtf::AUDIO_SAMPLE_RATE;

/// Standard ISO 10-band center frequencies in Hz.
pub const EQ_BAND_FREQUENCIES: [f32; 10] = [
    31.25,   // Sub-bass
    62.5,    // Bass
    125.0,   // Upper bass
    250.0,   // Low midrange
    500.0,   // Midrange
    1000.0,  // Upper midrange
    2000.0,  // Presence
    4000.0,  // Detail
    8000.0,  // High treble
    16000.0, // Air / brilliance
];

/// Number of parametric EQ frequency bands.
pub const NUM_EQ_BANDS: usize = 10;

/// Biquad digital filter response topology.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterType {
    /// Peaking (parametric bell) filter with center frequency, Q, and gain.
    Peaking,
    /// Low-shelf filter boosting or cutting all frequencies below cutoff.
    LowShelf,
    /// High-shelf filter boosting or cutting all frequencies above cutoff.
    HighShelf,
    /// Bandpass filter passing frequencies around center and attenuating outside.
    Bandpass,
}

/// Calculates 10^(db / 40) using 6th-order Taylor polynomial for e^x.
/// In RBJ formulas, A = sqrt(10^(dB/20)) = 10^(dB/40).
pub fn db_to_amplitude_ratio(db: f32) -> f32 {
    let clamped_db = db.clamp(-12.0, 12.0);
    // x = (db / 40) * ln(10) where ln(10) ~= 2.3025851
    let x = (clamped_db / 40.0) * 2.3025851;
    let x2 = x * x;
    let x3 = x2 * x;
    let x4 = x2 * x2;
    let x5 = x4 * x;
    let x6 = x3 * x3;

    1.0 + x + (x2 * 0.5) + (x3 * (1.0 / 6.0)) + (x4 * (1.0 / 24.0)) + (x5 * (1.0 / 120.0)) + (x6 * (1.0 / 720.0))
}

/// Biquad filter normalized coefficients:
/// H(z) = (b0 + b1*z^-1 + b2*z^-2) / (1 + a1*z^-1 + a2*z^-2)
#[derive(Debug, Clone, Copy)]
pub struct BiquadCoefficients {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
}

impl BiquadCoefficients {
    /// Identity coefficients (pass-through / flat).
    pub const fn flat() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
        }
    }

    /// Computes Robert Bristow-Johnson Audio EQ Cookbook coefficients.
    pub fn calculate(filter_type: FilterType, freq: f32, q: f32, gain_db: f32) -> Self {
        let clamped_q = q.clamp(0.1, 10.0);
        let clamped_gain = gain_db.clamp(-12.0, 12.0);

        // If gain is virtually zero for shelf/peaking, return flat pass-through
        if clamped_gain.abs() < 1e-3 && filter_type != FilterType::Bandpass {
            return Self::flat();
        }

        let nyquist = AUDIO_SAMPLE_RATE * 0.5;
        let clamped_freq = freq.clamp(10.0, nyquist - 100.0);

        let w0 = (2.0 * PI * clamped_freq) / AUDIO_SAMPLE_RATE;
        let cos_w0 = cos_f32(w0);
        let sin_w0 = sin_f32(w0);
        let alpha = sin_w0 / (2.0 * clamped_q);
        let a = db_to_amplitude_ratio(clamped_gain);
        let sqrt_a = sqrt_f32(a);

        let (b0_raw, b1_raw, b2_raw, a0_raw, a1_raw, a2_raw) = match filter_type {
            FilterType::Peaking => {
                let b0 = 1.0 + alpha * a;
                let b1 = -2.0 * cos_w0;
                let b2 = 1.0 - alpha * a;
                let a0 = 1.0 + alpha / a;
                let a1 = -2.0 * cos_w0;
                let a2 = 1.0 - alpha / a;
                (b0, b1, b2, a0, a1, a2)
            }
            FilterType::LowShelf => {
                let two_sqrt_a_alpha = 2.0 * sqrt_a * alpha;
                let b0 = a * ((a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
                let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0);
                let b2 = a * ((a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha);
                let a0 = (a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
                let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0);
                let a2 = (a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha;
                (b0, b1, b2, a0, a1, a2)
            }
            FilterType::HighShelf => {
                let two_sqrt_a_alpha = 2.0 * sqrt_a * alpha;
                let b0 = a * ((a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
                let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0);
                let b2 = a * ((a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha);
                let a0 = (a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
                let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos_w0);
                let a2 = (a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha;
                (b0, b1, b2, a0, a1, a2)
            }
            FilterType::Bandpass => {
                let b0 = alpha;
                let b1 = 0.0;
                let b2 = -alpha;
                let a0 = 1.0 + alpha;
                let a1 = -2.0 * cos_w0;
                let a2 = 1.0 - alpha;
                (b0, b1, b2, a0, a1, a2)
            }
        };

        if a0_raw.abs() < 1e-7 {
            return Self::flat();
        }

        let inv_a0 = 1.0 / a0_raw;
        Self {
            b0: b0_raw * inv_a0,
            b1: b1_raw * inv_a0,
            b2: b2_raw * inv_a0,
            a1: a1_raw * inv_a0,
            a2: a2_raw * inv_a0,
        }
    }
}

/// Stereo 2nd-order biquad IIR digital filter.
#[derive(Debug, Clone, Copy)]
pub struct BiquadFilter {
    pub coeffs: BiquadCoefficients,
    pub filter_type: FilterType,
    pub frequency: f32,
    pub q: f32,
    pub gain_db: f32,
    // Left channel state history
    x1_l: f32,
    x2_l: f32,
    y1_l: f32,
    y2_l: f32,
    // Right channel state history
    x1_r: f32,
    x2_r: f32,
    y1_r: f32,
    y2_r: f32,
}

impl BiquadFilter {
    pub fn new(filter_type: FilterType, freq: f32, q: f32, gain_db: f32) -> Self {
        let coeffs = BiquadCoefficients::calculate(filter_type, freq, q, gain_db);
        Self {
            coeffs,
            filter_type,
            frequency: freq,
            q,
            gain_db,
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

    /// Reconfigures filter parameters and recalculates coefficients.
    pub fn configure(&mut self, filter_type: FilterType, freq: f32, q: f32, gain_db: f32) {
        self.filter_type = filter_type;
        self.frequency = freq;
        self.q = q;
        self.gain_db = gain_db;
        self.coeffs = BiquadCoefficients::calculate(filter_type, freq, q, gain_db);
    }

    /// Resets filter history (flushes delay registers).
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

    /// Processes a single stereo PCM sample through Direct Form I difference equation.
    #[inline]
    pub fn process_sample(&mut self, left: i16, right: i16) -> (i16, i16) {
        let x_l = left as f32;
        let y_l = self.coeffs.b0 * x_l
            + self.coeffs.b1 * self.x1_l
            + self.coeffs.b2 * self.x2_l
            - self.coeffs.a1 * self.y1_l
            - self.coeffs.a2 * self.y2_l;

        self.x2_l = self.x1_l;
        self.x1_l = x_l;
        self.y2_l = self.y1_l;
        self.y1_l = y_l;

        let x_r = right as f32;
        let y_r = self.coeffs.b0 * x_r
            + self.coeffs.b1 * self.x1_r
            + self.coeffs.b2 * self.x2_r
            - self.coeffs.a1 * self.y1_r
            - self.coeffs.a2 * self.y2_r;

        self.x2_r = self.x1_r;
        self.x1_r = x_r;
        self.y2_r = self.y1_r;
        self.y1_r = y_r;

        let out_l = y_l.clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        let out_r = y_r.clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        (out_l, out_r)
    }
}

/// Equalizer presets designed for handheld gaming and vocal clarity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EqPreset {
    Flat,
    BassBoost,
    TrebleBoost,
    VocalClear,
    ActionGame,
}

/// 10-Band Parametric Equalizer chaining 10 stereo biquad filters in series.
pub struct ParametricEqualizer10 {
    pub bands: [BiquadFilter; NUM_EQ_BANDS],
    pub enabled: bool,
}

impl ParametricEqualizer10 {
    /// Creates a 10-band equalizer initialized with default flat response (0 dB on all bands).
    pub fn new() -> Self {
        let bands = [
            BiquadFilter::new(FilterType::LowShelf, EQ_BAND_FREQUENCIES[0], 0.707, 0.0),
            BiquadFilter::new(FilterType::Peaking, EQ_BAND_FREQUENCIES[1], 1.414, 0.0),
            BiquadFilter::new(FilterType::Peaking, EQ_BAND_FREQUENCIES[2], 1.414, 0.0),
            BiquadFilter::new(FilterType::Peaking, EQ_BAND_FREQUENCIES[3], 1.414, 0.0),
            BiquadFilter::new(FilterType::Peaking, EQ_BAND_FREQUENCIES[4], 1.414, 0.0),
            BiquadFilter::new(FilterType::Peaking, EQ_BAND_FREQUENCIES[5], 1.414, 0.0),
            BiquadFilter::new(FilterType::Peaking, EQ_BAND_FREQUENCIES[6], 1.414, 0.0),
            BiquadFilter::new(FilterType::Peaking, EQ_BAND_FREQUENCIES[7], 1.414, 0.0),
            BiquadFilter::new(FilterType::Peaking, EQ_BAND_FREQUENCIES[8], 1.414, 0.0),
            BiquadFilter::new(FilterType::HighShelf, EQ_BAND_FREQUENCIES[9], 0.707, 0.0),
        ];

        Self {
            bands,
            enabled: true,
        }
    }

    /// Sets the gain of a specific frequency band in dB (-12.0 to +12.0).
    pub fn set_band_gain(&mut self, band_idx: usize, gain_db: f32) {
        if band_idx < NUM_EQ_BANDS {
            let filter_type = self.bands[band_idx].filter_type;
            let freq = self.bands[band_idx].frequency;
            let q = self.bands[band_idx].q;
            self.bands[band_idx].configure(filter_type, freq, q, gain_db);
        }
    }

    /// Applies a curated audio equalization preset.
    pub fn apply_preset(&mut self, preset: EqPreset) {
        match preset {
            EqPreset::Flat => {
                for i in 0..NUM_EQ_BANDS {
                    self.set_band_gain(i, 0.0);
                }
            }
            EqPreset::BassBoost => {
                // Boost 31Hz, 62Hz, 125Hz by +6dB, gentle taper
                self.set_band_gain(0, 6.0);
                self.set_band_gain(1, 6.0);
                self.set_band_gain(2, 4.0);
                self.set_band_gain(3, 2.0);
                for i in 4..NUM_EQ_BANDS {
                    self.set_band_gain(i, 0.0);
                }
            }
            EqPreset::TrebleBoost => {
                for i in 0..6 {
                    self.set_band_gain(i, 0.0);
                }
                self.set_band_gain(6, 2.0);
                self.set_band_gain(7, 4.0);
                self.set_band_gain(8, 6.0);
                self.set_band_gain(9, 6.0);
            }
            EqPreset::VocalClear => {
                // Dip sub-bass to cut rumble, boost 1kHz - 4kHz presence
                self.set_band_gain(0, -4.0);
                self.set_band_gain(1, -2.0);
                self.set_band_gain(2, 0.0);
                self.set_band_gain(3, 1.0);
                self.set_band_gain(4, 3.0);
                self.set_band_gain(5, 5.0);
                self.set_band_gain(6, 4.0);
                self.set_band_gain(7, 2.0);
                self.set_band_gain(8, 0.0);
                self.set_band_gain(9, -1.0);
            }
            EqPreset::ActionGame => {
                // V-shaped curve: punchy explosions & crisp footsteps, scooped mids
                self.set_band_gain(0, 5.0);
                self.set_band_gain(1, 6.0);
                self.set_band_gain(2, 3.0);
                self.set_band_gain(3, -1.0);
                self.set_band_gain(4, -2.0);
                self.set_band_gain(5, -1.0);
                self.set_band_gain(6, 2.0);
                self.set_band_gain(7, 4.0);
                self.set_band_gain(8, 5.0);
                self.set_band_gain(9, 6.0);
            }
        }
    }

    /// Flushes all filter delay states.
    pub fn reset(&mut self) {
        for band in self.bands.iter_mut() {
            band.reset();
        }
    }

    /// Processes a single stereo sample through all 10 cascade stages.
    #[inline]
    pub fn process_sample(&mut self, mut left: i16, mut right: i16) -> (i16, i16) {
        if !self.enabled {
            return (left, right);
        }

        for band in self.bands.iter_mut() {
            let (l, r) = band.process_sample(left, right);
            left = l;
            right = r;
        }

        (left, right)
    }

    /// Processes stereo buffer in-place.
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

// Unit Tests

/// Unit test verifying biquad coefficient calculations and 10-band equalizer processing.
pub fn test_parametric_eq_core() -> bool {
    // 1. Verify amplitude ratio calculation
    let r_flat = db_to_amplitude_ratio(0.0);
    if (r_flat - 1.0).abs() > 1e-4 {
        return false;
    }
    let r_plus6 = db_to_amplitude_ratio(6.0);
    // 10^(6/40) ~= 1.4125
    if (r_plus6 - 1.4125).abs() > 1e-3 {
        return false;
    }
    let r_minus6 = db_to_amplitude_ratio(-6.0);
    // 10^(-6/40) ~= 0.7079
    if (r_minus6 - 0.7079).abs() > 1e-3 {
        return false;
    }

    // 2. Verify flat pass-through preserves input signal exactly
    let mut eq = ParametricEqualizer10::new();
    let sample_l = 1000i16;
    let sample_r = -1000i16;
    let (out_l, out_r) = eq.process_sample(sample_l, sample_r);
    if out_l != sample_l || out_r != sample_r {
        return false;
    }

    // 3. Verify Peaking Boost filter amplifies a resonant test frequency
    let mut peak_filter = BiquadFilter::new(FilterType::Peaking, 1000.0, 2.0, 6.0);
    let mut boosted_energy = 0i64;
    let mut flat_energy = 0i64;

    // Feed a 1 kHz square/pulse signal: period = 48 samples (48000 / 1000)
    for i in 0..96 {
        let in_sig = if (i % 48) < 24 { 2000 } else { -2000 };
        let (p_out, _) = peak_filter.process_sample(in_sig, in_sig);
        if i >= 48 {
            boosted_energy += p_out.abs() as i64;
            flat_energy += in_sig.abs() as i64;
        }
    }

    // Boosted signal must have higher energy than the flat input
    if boosted_energy <= flat_energy {
        return false;
    }

    // 4. Verify Low-Shelf filter boost
    let mut low_shelf = BiquadFilter::new(FilterType::LowShelf, 100.0, 0.707, 6.0);
    let mut low_boost_energy = 0i64;
    let mut low_flat_energy = 0i64;
    // Feed low-frequency 50 Hz signal: period = 960 samples
    for i in 0..200 {
        let in_sig = if i < 100 { 1000 } else { -1000 };
        let (ls_out, _) = low_shelf.process_sample(in_sig, in_sig);
        if i >= 50 {
            low_boost_energy += ls_out.abs() as i64;
            low_flat_energy += in_sig.abs() as i64;
        }
    }
    if low_boost_energy <= low_flat_energy {
        return false;
    }

    // 5. Verify EQ Presets
    eq.apply_preset(EqPreset::ActionGame);
    if eq.bands[0].gain_db != 5.0 || eq.bands[4].gain_db != -2.0 || eq.bands[9].gain_db != 6.0 {
        return false;
    }

    true
}
