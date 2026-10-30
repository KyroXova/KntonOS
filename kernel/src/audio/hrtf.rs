//! Head-Related Transfer Function (HRTF) Dataset, ITD Delay Line & ILD Filtering.
//!
//! Implements binaural acoustic modeling for headphone gaming and immersive 3D sound:
//! - Interaural Time Difference (ITD) fractional delay line using circular buffer linear interpolation.
//! - Interaural Level Difference (ILD) head-shadow acoustic attenuation filter.
//! - Synthetic 16-tap HRIR (Head-Related Impulse Response) dataset indexed by azimuth and elevation.
//! - Direct FIR convolution engine for spatialized binaural audio rendering.

use crate::graphics::types::{sin_f32, PI, HALF_PI};

/// Audio sample rate standard in KntonOS audio pipeline (48 kHz).
pub const AUDIO_SAMPLE_RATE: f32 = 48_000.0;

/// Speed of sound in dry air at 20°C (m/s).
pub const SPEED_OF_SOUND: f32 = 343.0;

/// Average adult human head radius (m) used for Woodworth ITD calculation (8.75 cm).
pub const HEAD_RADIUS: f32 = 0.0875;

/// Number of taps in the FIR filter for each ear's impulse response.
pub const HRIR_TAPS: usize = 16;

/// Circular buffer capacity for the ITD fractional delay line (power of 2).
pub const DELAY_LINE_SIZE: usize = 128;
pub const DELAY_LINE_MASK: usize = DELAY_LINE_SIZE - 1;

/// Maximum expected ITD delay in audio samples at 48 kHz (~0.66 ms -> ~32 samples).
pub const MAX_ITD_SAMPLES: f32 = 48.0;

/// Number of azimuth bins in the discrete HRTF database (every 15° around 360°).
pub const AZIMUTH_BINS: usize = 24;

// -----------------------------------------------------------------------------
// Interaural Time Difference (ITD) Delay Line
// -----------------------------------------------------------------------------

/// Circular delay line supporting fractional-sample delay with linear interpolation.
#[derive(Clone, Copy)]
pub struct EarDelayLine {
    buffer: [i16; DELAY_LINE_SIZE],
    write_pos: usize,
}

impl EarDelayLine {
    /// Creates a zeroed delay line.
    pub const fn new() -> Self {
        Self {
            buffer: [0; DELAY_LINE_SIZE],
            write_pos: 0,
        }
    }

    /// Pushes a new PCM audio sample into the delay line.
    #[inline]
    pub fn push(&mut self, sample: i16) {
        self.buffer[self.write_pos] = sample;
        self.write_pos = (self.write_pos + 1) & DELAY_LINE_MASK;
    }

    /// Reads a sample delayed by `delay_samples` (fractional).
    /// Uses linear interpolation between adjacent samples.
    #[inline]
    pub fn read_delayed(&self, delay_samples: f32) -> i16 {
        let clamped_delay = if delay_samples < 0.0 {
            0.0
        } else if delay_samples > (DELAY_LINE_SIZE - 2) as f32 {
            (DELAY_LINE_SIZE - 2) as f32
        } else {
            delay_samples
        };

        let d_int = clamped_delay as usize;
        let frac = clamped_delay - (d_int as f32);

        let idx0 = (self.write_pos + DELAY_LINE_SIZE - 1 - d_int) & DELAY_LINE_MASK;
        let idx1 = (self.write_pos + DELAY_LINE_SIZE - 2 - d_int) & DELAY_LINE_MASK;

        let s0 = self.buffer[idx0] as f32;
        let s1 = self.buffer[idx1] as f32;

        let interp = s0 * (1.0 - frac) + s1 * frac;
        interp.clamp(i16::MIN as f32, i16::MAX as f32) as i16
    }

    /// Resets all delay line samples to silence.
    pub fn clear(&mut self) {
        self.buffer = [0; DELAY_LINE_SIZE];
        self.write_pos = 0;
    }
}

// -----------------------------------------------------------------------------
// Interaural Level Difference (ILD) Head-Shadow Filter
// -----------------------------------------------------------------------------

/// 1-pole dynamic IIR low-pass and attenuation filter modeling head acoustic shadow.
#[derive(Clone, Copy, Default)]
pub struct IldFilter {
    prev_out: f32,
}

impl IldFilter {
    pub const fn new() -> Self {
        Self { prev_out: 0.0 }
    }

    /// Processes one sample through head-shadow attenuation and low-pass filtering.
    /// `alpha`: Low-pass filter coefficient in range [0.05, 1.0]. (1.0 = full bandwidth / transparent).
    /// `gain`: Amplitude gain factor in range [0.0, 1.0].
    #[inline]
    pub fn process(&mut self, input: i16, alpha: f32, gain: f32) -> i16 {
        let clamped_alpha = if alpha < 0.05 {
            0.05
        } else if alpha > 1.0 {
            1.0
        } else {
            alpha
        };
        let clamped_gain = if gain < 0.0 {
            0.0
        } else if gain > 1.0 {
            1.0
        } else {
            gain
        };

        let in_f = input as f32;
        // 1-pole low pass: y[n] = y[n-1] + alpha * (x[n] - y[n-1])
        let filtered = self.prev_out + clamped_alpha * (in_f - self.prev_out);
        self.prev_out = filtered;

        let output = filtered * clamped_gain;
        output.clamp(i16::MIN as f32, i16::MAX as f32) as i16
    }

    /// Resets internal filter history.
    pub fn reset(&mut self) {
        self.prev_out = 0.0;
    }
}

// -----------------------------------------------------------------------------
// HRIR Impulse Response & Dataset
// -----------------------------------------------------------------------------

/// Pair of 16-tap FIR impulse response filters (Left & Right ears).
#[derive(Clone, Copy)]
pub struct HrtfImpulseResponse {
    pub left_taps: [f32; HRIR_TAPS],
    pub right_taps: [f32; HRIR_TAPS],
}

impl HrtfImpulseResponse {
    pub const fn new(left: [f32; HRIR_TAPS], right: [f32; HRIR_TAPS]) -> Self {
        Self {
            left_taps: left,
            right_taps: right,
        }
    }
}

/// Circular FIR convolver applying a 16-tap impulse response to an audio stream.
#[derive(Clone, Copy)]
pub struct FirConvolver {
    history: [i16; HRIR_TAPS],
    pos: usize,
}

impl FirConvolver {
    pub const fn new() -> Self {
        Self {
            history: [0; HRIR_TAPS],
            pos: 0,
        }
    }

    /// Pushes sample and evaluates the 16-tap FIR convolution sum: y[n] = sum(x[n-k] * h[k]).
    #[inline]
    pub fn process(&mut self, sample: i16, taps: &[f32; HRIR_TAPS]) -> i16 {
        self.history[self.pos] = sample;
        let mut sum = 0.0f32;

        for k in 0..HRIR_TAPS {
            let idx = (self.pos + HRIR_TAPS - k) % HRIR_TAPS;
            sum += self.history[idx] as f32 * taps[k];
        }

        self.pos = (self.pos + 1) % HRIR_TAPS;
        sum.clamp(i16::MIN as f32, i16::MAX as f32) as i16
    }

    pub fn reset(&mut self) {
        self.history = [0; HRIR_TAPS];
        self.pos = 0;
    }
}

// -----------------------------------------------------------------------------
// Woodworth ITD & ILD Mathematical Modeling
// -----------------------------------------------------------------------------

/// Computes the Woodworth ITD (Interaural Time Difference) in samples at 48 kHz.
/// `azimuth_rad`: Angle in radians from listener's forward heading (-PI to +PI).
/// Positive azimuth is to the RIGHT, negative azimuth is to the LEFT.
/// Returns `(left_ear_delay, right_ear_delay)` in samples.
pub fn calculate_itd_samples(azimuth_rad: f32) -> (f32, f32) {
    let abs_angle = if azimuth_rad < 0.0 { -azimuth_rad } else { azimuth_rad };
    let clamped_angle = if abs_angle > PI { PI } else { abs_angle };

    // Woodworth formula: delta_t = (r / c) * (sin(theta) + theta) for theta in [0, PI/2]
    // For theta in [PI/2, PI]: delta_t = (r / c) * (sin(PI - theta) + (PI - theta))
    let effective_angle = if clamped_angle > HALF_PI {
        PI - clamped_angle
    } else {
        clamped_angle
    };

    let sin_val = sin_f32(effective_angle);
    let delta_t_sec = (HEAD_RADIUS / SPEED_OF_SOUND) * (sin_val + effective_angle);
    let delay_samples = delta_t_sec * AUDIO_SAMPLE_RATE;

    if azimuth_rad > 0.0 {
        // Sound is on the RIGHT: reaches right ear first, left ear is delayed
        (delay_samples, 0.0)
    } else if azimuth_rad < 0.0 {
        // Sound is on the LEFT: reaches left ear first, right ear is delayed
        (0.0, delay_samples)
    } else {
        (0.0, 0.0)
    }
}

/// Computes Interaural Level Difference (ILD) filter parameters (alpha, gain) for Left and Right ears.
/// Returns `((left_alpha, left_gain), (right_alpha, right_gain))`.
pub fn calculate_ild_parameters(azimuth_rad: f32) -> ((f32, f32), (f32, f32)) {
    let sin_val = sin_f32(azimuth_rad);

    if azimuth_rad > 0.0 {
        // Sound on RIGHT:
        // Right ear (ipsilateral): direct path, high-frequency transparency, slight concha boost
        let r_gain = 1.0;
        let r_alpha = 1.0;

        // Left ear (contralateral): skull shadow attenuates high frequencies and overall volume
        let shadow = sin_val.clamp(0.0, 1.0);
        let l_gain = 1.0 - (0.45 * shadow);
        let l_alpha = 1.0 - (0.75 * shadow);

        ((l_alpha, l_gain), (r_alpha, r_gain))
    } else if azimuth_rad < 0.0 {
        // Sound on LEFT:
        let shadow = (-sin_val).clamp(0.0, 1.0);
        let l_gain = 1.0;
        let l_alpha = 1.0;

        let r_gain = 1.0 - (0.45 * shadow);
        let r_alpha = 1.0 - (0.75 * shadow);

        ((l_alpha, l_gain), (r_alpha, r_gain))
    } else {
        // Straight ahead: symmetric
        ((1.0, 1.0), (1.0, 1.0))
    }
}

// -----------------------------------------------------------------------------
// HRTF Dataset Lookup
// -----------------------------------------------------------------------------

/// Generates a synthesized 16-tap impulse response for a given ear based on incidence angle.
/// Models the direct wave arrival, pinna reflection notch (pinna notch cue), and head diffraction.
pub fn synthesize_ear_ir(azimuth_rad: f32, is_ipsilateral: bool) -> [f32; HRIR_TAPS] {
    let mut taps = [0.0f32; HRIR_TAPS];

    if is_ipsilateral {
        // Ipsilateral ear: sharp initial impulse, distinct pinna reflection at tap 3..5
        taps[0] = 0.85;
        taps[1] = 0.20;
        taps[2] = -0.10;
        // Pinna reflection cue varies with angle
        let reflection = 0.15 * cos_f32(azimuth_rad);
        taps[3] = reflection;
        taps[4] = -reflection * 0.5;
        taps[5] = 0.05;
    } else {
        // Contralateral ear: head diffraction disperses and smooths the impulse across taps 2..8
        taps[0] = 0.0;
        taps[1] = 0.10;
        taps[2] = 0.35;
        taps[3] = 0.25;
        taps[4] = 0.12;
        taps[5] = 0.06;
        taps[6] = 0.02;
    }

    taps
}

fn cos_f32(rad: f32) -> f32 {
    sin_f32(rad + HALF_PI)
}

/// HRTF Dataset holding impulse responses for discrete azimuths around the horizontal plane.
pub struct HrtfDataset {
    responses: [HrtfImpulseResponse; AZIMUTH_BINS],
}

impl HrtfDataset {
    /// Constructs and precalculates the synthetic HRTF dataset across 24 azimuth angles.
    pub fn new() -> Self {
        let mut responses = [HrtfImpulseResponse::new([0.0; HRIR_TAPS], [0.0; HRIR_TAPS]); AZIMUTH_BINS];

        for i in 0..AZIMUTH_BINS {
            let angle_rad = ((i as f32) / (AZIMUTH_BINS as f32)) * 2.0 * PI - PI;
            let left_ipsi = angle_rad <= 0.0;
            let right_ipsi = angle_rad >= 0.0;

            let left_ir = synthesize_ear_ir(angle_rad, left_ipsi);
            let right_ir = synthesize_ear_ir(angle_rad, right_ipsi);
            responses[i] = HrtfImpulseResponse::new(left_ir, right_ir);
        }

        Self { responses }
    }

    /// Looks up the nearest HRIR pair for an azimuth in radians [-PI, +PI].
    pub fn lookup(&self, azimuth_rad: f32) -> HrtfImpulseResponse {
        // Normalize angle to [0, 2*PI)
        let mut norm = azimuth_rad + PI;
        while norm < 0.0 {
            norm += 2.0 * PI;
        }
        while norm >= 2.0 * PI {
            norm -= 2.0 * PI;
        }

        let bin = ((norm / (2.0 * PI)) * (AZIMUTH_BINS as f32)) as usize;
        let clamped_bin = if bin >= AZIMUTH_BINS { AZIMUTH_BINS - 1 } else { bin };
        self.responses[clamped_bin]
    }
}

// -----------------------------------------------------------------------------
// Unit Tests
// -----------------------------------------------------------------------------

/// Unit test verifying HRTF delay line, ILD filter, and dataset lookup.
pub fn test_hrtf_dsp_core() -> bool {
    // 1. Verify EarDelayLine fractional delay
    let mut delay_line = EarDelayLine::new();
    for i in 0..30 {
        delay_line.push((i * 100) as i16);
    }

    // Zero delay must return most recent sample (29 * 100 = 2900)
    let s0 = delay_line.read_delayed(0.0);
    if s0 != 2900 {
        return false;
    }

    // Delay of 5 samples: sample 24 * 100 = 2400
    let s5 = delay_line.read_delayed(5.0);
    if s5 != 2400 {
        return false;
    }

    // Delay of 2.5 samples: midpoint between 2600 and 2700 = 2650
    let s_mid = delay_line.read_delayed(2.5);
    if (s_mid - 2650).abs() > 1 {
        return false;
    }

    // 2. Verify Woodworth ITD calculation
    let (l_zero, r_zero) = calculate_itd_samples(0.0);
    if l_zero != 0.0 || r_zero != 0.0 {
        return false;
    }

    // At +90 degrees (HALF_PI), sound is on right: left ear must be delayed by ~30..33 samples, right 0
    let (l_delay_r90, r_delay_r90) = calculate_itd_samples(HALF_PI);
    if r_delay_r90 != 0.0 || l_delay_r90 < 28.0 || l_delay_r90 > 34.0 {
        return false;
    }

    // At -90 degrees (-HALF_PI), sound is on left: right ear must be delayed by ~30..33 samples, left 0
    let (l_delay_l90, r_delay_l90) = calculate_itd_samples(-HALF_PI);
    if l_delay_l90 != 0.0 || r_delay_l90 < 28.0 || r_delay_l90 > 34.0 {
        return false;
    }

    // 3. Verify ILD head-shadow parameters
    let ((l_a_r90, l_g_r90), (r_a_r90, r_g_r90)) = calculate_ild_parameters(HALF_PI);
    if r_g_r90 != 1.0 || r_a_r90 != 1.0 {
        return false;
    }
    // Left ear (contralateral) must be attenuated and low-passed
    if l_g_r90 >= 1.0 || l_a_r90 >= 1.0 || l_g_r90 <= 0.0 || l_a_r90 <= 0.0 {
        return false;
    }

    // 4. Verify IldFilter dynamic low-pass attenuation
    let mut filter = IldFilter::new();
    let mut high_freq_response = 0;
    // Feed alternating +/- 1000 square wave (Nyquist / high frequency)
    for i in 0..16 {
        let sig = if i % 2 == 0 { 1000 } else { -1000 };
        high_freq_response = filter.process(sig, 0.15, 0.7);
    }
    // High frequency output must be heavily attenuated (< 300)
    if high_freq_response.abs() > 300 {
        return false;
    }

    // 5. Verify FirConvolver
    let mut convolver = FirConvolver::new();
    let mut impulse_taps = [0.0f32; HRIR_TAPS];
    impulse_taps[0] = 1.0; // Identity impulse
    let out = convolver.process(1234, &impulse_taps);
    if out != 1234 {
        return false;
    }

    // 6. Verify HrtfDataset
    let dataset = HrtfDataset::new();
    let ir_front = dataset.lookup(0.0);
    // Frontal IR should have non-zero direct tap
    if ir_front.left_taps[0] <= 0.0 || ir_front.right_taps[0] <= 0.0 {
        return false;
    }

    true
}
