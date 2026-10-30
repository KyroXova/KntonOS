//! Kernel-Direct Low-Latency Spatial Audio Virtualizer & Ear-Canal Impedance Compensator Engine.
//!
//! Provides bare-metal 3D binaural audio spatialization, sub-millisecond fractional delay modeling,
//! spherical head-shadow diffraction, cross-talk cancellation (XTC) for handheld stereo speakers,
//! and ear-canal acoustic impedance resonance compensation for headphone and IEM gaming.

#![allow(dead_code)]

extern crate alloc;

use crate::graphics::types::{cos_f32, sin_f32, sqrt_f32, HALF_PI, PI, TWO_PI};
use crate::audio::spatializer::{atan_f32, atan2_f32};
use crate::audio::eq::db_to_amplitude_ratio;

// -----------------------------------------------------------------------------
// Constants
// -----------------------------------------------------------------------------

use crate::audio::hrtf::{AUDIO_SAMPLE_RATE, SPEED_OF_SOUND};

/// Standard human cranial radius in meters (8.75 cm).
pub const DEFAULT_HEAD_RADIUS: f32 = 0.0875;

/// Average human ear-canal length in meters (25 mm).
pub const EAR_CANAL_LENGTH: f32 = 0.025;

/// Average human ear-canal diameter in meters (7.5 mm).
pub const EAR_CANAL_DIAMETER: f32 = 0.0075;

/// Nominal human ear-canal quarter-wavelength resonance frequency in Hz (~3.15 kHz).
pub const EAR_CANAL_RESONANCE_FREQ: f32 = 3146.0;

/// Nominal ear-canal resonance Q factor.
pub const EAR_CANAL_RESONANCE_Q: f32 = 3.0;

/// Nominal natural acoustic peak boost at tympanic membrane (eardrum) in dB.
pub const EAR_CANAL_NATURAL_BOOST_DB: f32 = 12.0;

/// Maximum allowable Interaural Time Difference in seconds (~0.8 ms).
pub const MAX_ITD_SECONDS: f32 = 0.0008;

/// Maximum allowable ITD in samples at 48 kHz (~38.4 samples).
pub const SPATIAL_MAX_ITD_SAMPLES: f32 = MAX_ITD_SECONDS * AUDIO_SAMPLE_RATE;

// -----------------------------------------------------------------------------
// 1. 3D Spatial Vector & Coordinate Geometry
// -----------------------------------------------------------------------------

/// 3D Spatial Vector representing positions, velocities, and directional vectors in meters.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct SpatialVector3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl SpatialVector3 {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub const fn zero() -> Self {
        Self { x: 0.0, y: 0.0, z: 0.0 }
    }

    pub const fn forward() -> Self {
        Self { x: 0.0, y: 1.0, z: 0.0 }
    }

    pub const fn up() -> Self {
        Self { x: 0.0, y: 0.0, z: 1.0 }
    }

    pub const fn right() -> Self {
        Self { x: 1.0, y: 0.0, z: 0.0 }
    }

    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    pub fn cross(self, other: Self) -> Self {
        Self {
            x: self.y * other.z - self.z * other.y,
            y: self.z * other.x - self.x * other.z,
            z: self.x * other.y - self.y * other.x,
        }
    }

    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    pub fn length(self) -> f32 {
        sqrt_f32(self.length_squared())
    }

    pub fn normalize(self) -> Self {
        let len = self.length();
        if len > 1e-6 {
            let inv = 1.0 / len;
            Self {
                x: self.x * inv,
                y: self.y * inv,
                z: self.z * inv,
            }
        } else {
            Self::zero()
        }
    }

    pub fn distance_to(self, other: Self) -> f32 {
        (self - other).length()
    }
}

impl core::ops::Add for SpatialVector3 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
            z: self.z + rhs.z,
        }
    }
}

impl core::ops::Sub for SpatialVector3 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
            z: self.z - rhs.z,
        }
    }
}

impl core::ops::Mul<f32> for SpatialVector3 {
    type Output = Self;
    fn mul(self, rhs: f32) -> Self {
        Self {
            x: self.x * rhs,
            y: self.y * rhs,
            z: self.z * rhs,
        }
    }
}

/// Orthonormal cranial coordinate orientation for the 3D listener.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct SpatialOrientation {
    pub forward: SpatialVector3,
    pub up: SpatialVector3,
    pub right: SpatialVector3,
}

impl SpatialOrientation {
    /// Creates an identity orientation (looking along +Y forward, +Z up, +X right).
    pub const fn identity() -> Self {
        Self {
            forward: SpatialVector3::forward(),
            up: SpatialVector3::up(),
            right: SpatialVector3::right(),
        }
    }

    /// Creates an orthonormal orientation frame from given forward and up vectors.
    pub fn from_forward_up(forward: SpatialVector3, up: SpatialVector3) -> Self {
        let f = forward.normalize();
        let r = f.cross(up).normalize();
        let u = r.cross(f).normalize();
        Self {
            forward: f,
            up: u,
            right: r,
        }
    }
}

/// 3D Spatial Listener with cranial dimensions and ear positioning.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct SpatialListener {
    pub position: SpatialVector3,
    pub orientation: SpatialOrientation,
    pub head_radius: f32,
    pub ear_spacing: f32,
}

impl SpatialListener {
    pub const fn new(position: SpatialVector3) -> Self {
        Self {
            position,
            orientation: SpatialOrientation::identity(),
            head_radius: DEFAULT_HEAD_RADIUS,
            ear_spacing: DEFAULT_HEAD_RADIUS * 2.0,
        }
    }

    pub fn with_orientation(mut self, orientation: SpatialOrientation) -> Self {
        self.orientation = orientation;
        self
    }

    pub fn get_left_ear_position(&self) -> SpatialVector3 {
        self.position - self.orientation.right * (self.ear_spacing * 0.5)
    }

    pub fn get_right_ear_position(&self) -> SpatialVector3 {
        self.position + self.orientation.right * (self.ear_spacing * 0.5)
    }
}

/// 3D Spatial Emitter (Sound Source) with distance attenuation and directional radiation cone.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct SpatialEmitter {
    pub position: SpatialVector3,
    pub velocity: SpatialVector3,
    pub cone_direction: SpatialVector3,
    pub reference_distance: f32,
    pub max_distance: f32,
    pub rolloff_factor: f32,
    pub inner_cone_angle_rad: f32,
    pub outer_cone_angle_rad: f32,
    pub outer_cone_gain: f32,
    pub gain: f32,
}

impl SpatialEmitter {
    pub const fn new(position: SpatialVector3) -> Self {
        Self {
            position,
            velocity: SpatialVector3::zero(),
            cone_direction: SpatialVector3::forward(),
            reference_distance: 1.0,
            max_distance: 50.0,
            rolloff_factor: 1.0,
            inner_cone_angle_rad: TWO_PI,
            outer_cone_angle_rad: TWO_PI,
            outer_cone_gain: 1.0,
            gain: 1.0,
        }
    }

    /// Computes inverse-distance rolloff attenuation factor.
    pub fn calculate_distance_attenuation(&self, distance: f32) -> f32 {
        let clamped_dist = if distance < self.reference_distance {
            self.reference_distance
        } else if distance > self.max_distance {
            self.max_distance
        } else {
            distance
        };

        if clamped_dist <= 0.0 {
            return 1.0;
        }

        self.reference_distance / (self.reference_distance + self.rolloff_factor * (clamped_dist - self.reference_distance))
    }

    /// Computes directional cone attenuation factor.
    pub fn calculate_cone_attenuation(&self, to_listener: SpatialVector3) -> f32 {
        if self.inner_cone_angle_rad >= TWO_PI {
            return 1.0;
        }

        let dir = to_listener.normalize();
        let cos_angle = self.cone_direction.dot(dir);
        let angle = if cos_angle >= 1.0 {
            0.0
        } else if cos_angle <= -1.0 {
            PI
        } else {
            // High-precision acos approximation: acos(x) = HALF_PI - asin(x)
            HALF_PI - atan_f32(cos_angle / sqrt_f32(1.0 - cos_angle * cos_angle))
        };

        let half_inner = self.inner_cone_angle_rad * 0.5;
        let half_outer = self.outer_cone_angle_rad * 0.5;

        if angle <= half_inner {
            1.0
        } else if angle >= half_outer {
            self.outer_cone_gain
        } else {
            let t = (angle - half_inner) / (half_outer - half_inner);
            1.0 - t * (1.0 - self.outer_cone_gain)
        }
    }
}

// -----------------------------------------------------------------------------
// 2. Interaural Time (ITD) & Level (ILD) Difference Modeling
// -----------------------------------------------------------------------------

/// Calculated Binaural Spatial Parameters for a listener-emitter pair.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct BinauralParameters {
    pub azimuth_rad: f32,
    pub elevation_rad: f32,
    pub distance: f32,
    pub left_delay_seconds: f32,
    pub right_delay_seconds: f32,
    pub left_delay_samples: f32,
    pub right_delay_samples: f32,
    pub itd_samples: f32,
    pub left_gain: f32,
    pub right_gain: f32,
    pub ild_db: f32,
}

impl BinauralParameters {
    /// Computes exact binaural parameters from a listener and emitter using Woodworth spherical head model.
    pub fn calculate(listener: &SpatialListener, emitter: &SpatialEmitter) -> Self {
        let rel_pos = emitter.position - listener.position;
        let distance = rel_pos.length();

        // Transform into listener's local coordinate frame
        let local_x = rel_pos.dot(listener.orientation.right);
        let local_y = rel_pos.dot(listener.orientation.forward);
        let local_z = rel_pos.dot(listener.orientation.up);

        let horiz_dist = sqrt_f32(local_x * local_x + local_y * local_y);
        let azimuth_rad = atan2_f32(local_x, local_y);
        let elevation_rad = if distance > 1e-6 {
            atan2_f32(local_z, horiz_dist)
        } else {
            0.0
        };

        // Base attenuation
        let dist_gain = emitter.calculate_distance_attenuation(distance);
        let cone_gain = emitter.calculate_cone_attenuation(listener.position - emitter.position);
        let base_gain = emitter.gain * dist_gain * cone_gain;

        // Woodworth ITD Calculation:
        // Delta_t = (r / c) * (|azimuth| + sin|azimuth|) * cos(elevation)
        let abs_azimuth = if azimuth_rad < 0.0 { -azimuth_rad } else { azimuth_rad };
        let cos_elev = cos_f32(elevation_rad);
        let abs_cos_elev = if cos_elev < 0.0 { 0.0 } else { cos_elev };

        let time_delay = (listener.head_radius / SPEED_OF_SOUND)
            * (abs_azimuth + sin_f32(abs_azimuth))
            * abs_cos_elev;

        let (left_delay_seconds, right_delay_seconds) = if azimuth_rad > 0.0 {
            // Sound is to the right: right ear is ipsilateral, left ear is delayed (contralateral)
            (time_delay, 0.0)
        } else {
            // Sound is to the left: left ear is ipsilateral, right ear is delayed (contralateral)
            (0.0, time_delay)
        };

        let left_delay_samples = left_delay_seconds * AUDIO_SAMPLE_RATE;
        let right_delay_samples = right_delay_seconds * AUDIO_SAMPLE_RATE;
        let itd_samples = (left_delay_samples - right_delay_samples).abs();

        // Interaural Level Difference (ILD):
        // Up to 18 dB attenuation on contralateral ear for high frequencies
        let sin_az = sin_f32(azimuth_rad);
        let ild_db = 18.0 * (sin_az * sin_az) * abs_cos_elev;
        let shadow_ratio = db_to_amplitude_ratio(-ild_db);

        let (left_gain, right_gain) = if azimuth_rad > 0.0 {
            // Emitter is on the right
            (base_gain * shadow_ratio, base_gain)
        } else {
            // Emitter is on the left
            (base_gain, base_gain * shadow_ratio)
        };

        Self {
            azimuth_rad,
            elevation_rad,
            distance,
            left_delay_seconds,
            right_delay_seconds,
            left_delay_samples,
            right_delay_samples,
            itd_samples,
            left_gain,
            right_gain,
            ild_db,
        }
    }
}

// -----------------------------------------------------------------------------
// 2b. Sub-Millisecond Fractional Delay Line & Spherical Head-Shadow Filter
// -----------------------------------------------------------------------------

/// Ring buffer capacity for the sub-millisecond fractional delay line (128 samples = 2.67 ms at 48 kHz).
pub const FRACTIONAL_DELAY_CAPACITY: usize = 128;
pub const FRACTIONAL_DELAY_MASK: usize = FRACTIONAL_DELAY_CAPACITY - 1;

/// Sub-millisecond fractional delay line supporting 4-point Hermite and Lagrange interpolation.
#[derive(Debug, Clone, Copy)]
pub struct SubMillisecondDelayLine {
    buffer: [f32; FRACTIONAL_DELAY_CAPACITY],
    write_pos: usize,
    pub current_delay_samples: f32,
}

impl SubMillisecondDelayLine {
    /// Creates a zeroed delay line.
    pub const fn new() -> Self {
        Self {
            buffer: [0.0; FRACTIONAL_DELAY_CAPACITY],
            write_pos: 0,
            current_delay_samples: 0.0,
        }
    }

    /// Resets all delay line state and buffer memory to zero.
    pub fn reset(&mut self) {
        self.buffer = [0.0; FRACTIONAL_DELAY_CAPACITY];
        self.write_pos = 0;
        self.current_delay_samples = 0.0;
    }

    /// Pushes a single audio sample into the circular delay line.
    pub fn write_sample(&mut self, sample: f32) {
        self.buffer[self.write_pos] = sample;
        self.write_pos = (self.write_pos + 1) & FRACTIONAL_DELAY_MASK;
    }

    /// Reads delayed sample using 4-point, 3rd-order Hermite cubic interpolation with C1 continuity.
    pub fn read_hermite(&self, delay_samples: f32) -> f32 {
        let clamped_delay = if delay_samples < 0.0 {
            0.0
        } else if delay_samples > (FRACTIONAL_DELAY_CAPACITY - 4) as f32 {
            (FRACTIONAL_DELAY_CAPACITY - 4) as f32
        } else {
            delay_samples
        };

        let int_delay = clamped_delay as usize;
        let frac = clamped_delay - int_delay as f32;

        let base = (self.write_pos + FRACTIONAL_DELAY_CAPACITY - 1 - int_delay) & FRACTIONAL_DELAY_MASK;
        let idx_prev = (base + 1) & FRACTIONAL_DELAY_MASK;
        let idx_0 = base;
        let idx_1 = (base + FRACTIONAL_DELAY_CAPACITY - 1) & FRACTIONAL_DELAY_MASK;
        let idx_2 = (base + FRACTIONAL_DELAY_CAPACITY - 2) & FRACTIONAL_DELAY_MASK;

        let y_m1 = self.buffer[idx_prev];
        let y0 = self.buffer[idx_0];
        let y1 = self.buffer[idx_1];
        let y2 = self.buffer[idx_2];

        let c0 = y0;
        let c1 = 0.5 * (y1 - y_m1);
        let c2 = y_m1 - 2.5 * y0 + 2.0 * y1 - 0.5 * y2;
        let c3 = 0.5 * (y2 - y_m1) + 1.5 * (y0 - y1);

        ((c3 * frac + c2) * frac + c1) * frac + c0
    }

    /// Reads delayed sample using 4-point Lagrange polynomial interpolation.
    pub fn read_lagrange(&self, delay_samples: f32) -> f32 {
        let clamped_delay = if delay_samples < 0.0 {
            0.0
        } else if delay_samples > (FRACTIONAL_DELAY_CAPACITY - 4) as f32 {
            (FRACTIONAL_DELAY_CAPACITY - 4) as f32
        } else {
            delay_samples
        };

        let int_delay = clamped_delay as usize;
        let frac = clamped_delay - int_delay as f32;

        let base = (self.write_pos + FRACTIONAL_DELAY_CAPACITY - 1 - int_delay) & FRACTIONAL_DELAY_MASK;
        let idx_m1 = (base + 1) & FRACTIONAL_DELAY_MASK;
        let idx_0 = base;
        let idx_1 = (base + FRACTIONAL_DELAY_CAPACITY - 1) & FRACTIONAL_DELAY_MASK;
        let idx_2 = (base + FRACTIONAL_DELAY_CAPACITY - 2) & FRACTIONAL_DELAY_MASK;

        let ym1 = self.buffer[idx_m1];
        let y0 = self.buffer[idx_0];
        let y1 = self.buffer[idx_1];
        let y2 = self.buffer[idx_2];

        let l_m1 = -frac * (frac - 1.0) * (frac - 2.0) / 6.0;
        let l0 = (frac + 1.0) * (frac - 1.0) * (frac - 2.0) * 0.5;
        let l1 = -(frac + 1.0) * frac * (frac - 2.0) * 0.5;
        let l2 = (frac + 1.0) * frac * (frac - 1.0) / 6.0;

        l_m1 * ym1 + l0 * y0 + l1 * y1 + l2 * y2
    }

    /// Pushes a sample and retrieves delayed output with smooth delay parameter slewing.
    pub fn process_sample_slewed(&mut self, input: f32, target_delay: f32, slew_rate: f32) -> f32 {
        self.write_sample(input);
        let diff = target_delay - self.current_delay_samples;
        if diff.abs() <= slew_rate {
            self.current_delay_samples = target_delay;
        } else if diff > 0.0 {
            self.current_delay_samples += slew_rate;
        } else {
            self.current_delay_samples -= slew_rate;
        }
        self.read_hermite(self.current_delay_samples)
    }
}

/// First-order continuous spherical head diffraction filter (Brown & Duda Rayleigh sphere model).
/// Models acoustic diffraction around a rigid sphere with dynamic azimuthal angle tracking.
#[derive(Debug, Clone, Copy)]
pub struct HeadShadowSphericalFilter {
    pub b0: f32,
    pub b1: f32,
    pub a1: f32,
    prev_x: f32,
    prev_y: f32,
    pub current_alpha: f32,
}

impl HeadShadowSphericalFilter {
    pub const fn new() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            a1: 0.0,
            prev_x: 0.0,
            prev_y: 0.0,
            current_alpha: 1.0,
        }
    }

    pub fn reset(&mut self) {
        self.prev_x = 0.0;
        self.prev_y = 0.0;
    }

    /// Updates filter coefficients based on the incident azimuth angle relative to the ear in radians.
    pub fn update_angle(&mut self, theta_ear: f32) {
        let sin_val = sin_f32(theta_ear);
        let raw_alpha = 1.0 + 0.9 * sin_val;
        let alpha = if raw_alpha < 0.18 {
            0.18
        } else if raw_alpha > 2.0 {
            2.0
        } else {
            raw_alpha
        };
        self.current_alpha = alpha;

        let w0 = 2.0 * SPEED_OF_SOUND / DEFAULT_HEAD_RADIUS;
        let k = (2.0 * AUDIO_SAMPLE_RATE) / w0;

        let denom = k + 1.0;
        self.b0 = (k * alpha + 1.0) / denom;
        self.b1 = (1.0 - k * alpha) / denom;
        self.a1 = (1.0 - k) / denom;
    }

    /// Processes a single audio sample through the spherical head-shadow IIR filter.
    pub fn process_sample(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.prev_x - self.a1 * self.prev_y;
        self.prev_x = x;
        self.prev_y = y;
        y
    }

    /// Evaluates the frequency magnitude response |H(e^{j*omega})| in linear scale.
    pub fn magnitude_response(&self, freq_hz: f32) -> f32 {
        let w = (TWO_PI * freq_hz) / AUDIO_SAMPLE_RATE;
        let cos_w = cos_f32(w);
        let sin_w = sin_f32(w);

        let num_re = self.b0 + self.b1 * cos_w;
        let num_im = -self.b1 * sin_w;
        let num_mag2 = num_re * num_re + num_im * num_im;

        let den_re = 1.0 + self.a1 * cos_w;
        let den_im = -self.a1 * sin_w;
        let den_mag2 = den_re * den_re + den_im * den_im;

        if den_mag2 > 1e-12 {
            sqrt_f32(num_mag2 / den_mag2)
        } else {
            1.0
        }
    }
}

/// Dual-Ear Binaural Channel Processor uniting fractional delay and spherical head shadow.
#[derive(Debug, Clone, Copy)]
pub struct DualEarSpatialChannel {
    pub delay_line: SubMillisecondDelayLine,
    pub shadow_filter: HeadShadowSphericalFilter,
}

impl DualEarSpatialChannel {
    pub const fn new() -> Self {
        Self {
            delay_line: SubMillisecondDelayLine::new(),
            shadow_filter: HeadShadowSphericalFilter::new(),
        }
    }

    pub fn reset(&mut self) {
        self.delay_line.reset();
        self.shadow_filter.reset();
    }

    pub fn process(&mut self, input: f32, delay_samples: f32, ear_azimuth: f32, gain: f32) -> f32 {
        self.shadow_filter.update_angle(ear_azimuth);
        let filtered = self.shadow_filter.process_sample(input * gain);
        self.delay_line.process_sample_slewed(filtered, delay_samples, 0.05)
    }
}

// -----------------------------------------------------------------------------
// 3. Ear-Canal Acoustic Impedance & Transfer Function (ECTF)
// -----------------------------------------------------------------------------

/// Acoustic Impedance and Quarter-Wave Resonance Model for the Human Ear Canal.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct EarCanalModel {
    pub length_m: f32,
    pub diameter_m: f32,
    pub resonance_freq_hz: f32,
    pub q_factor: f32,
    pub natural_boost_db: f32,
}

impl EarCanalModel {
    /// Creates the standard human ear canal acoustic model.
    pub const fn default_human() -> Self {
        Self {
            length_m: EAR_CANAL_LENGTH,
            diameter_m: EAR_CANAL_DIAMETER,
            resonance_freq_hz: EAR_CANAL_RESONANCE_FREQ,
            q_factor: EAR_CANAL_RESONANCE_Q,
            natural_boost_db: EAR_CANAL_NATURAL_BOOST_DB,
        }
    }

    /// Computes acoustic quarter-wave resonance frequency with end correction:
    /// f_res = c / (4 * (L + 0.3 * D))
    pub fn calculate_resonance_frequency(&self) -> f32 {
        let effective_length = self.length_m + 0.3 * self.diameter_m;
        if effective_length > 1e-4 {
            SPEED_OF_SOUND / (4.0 * effective_length)
        } else {
            EAR_CANAL_RESONANCE_FREQ
        }
    }
}

/// Biquad second-order IIR filter coefficients for ear-canal acoustic resonance equalization:
/// H(z) = (b0 + b1*z^-1 + b2*z^-2) / (1 + a1*z^-1 + a2*z^-2)
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct EarCanalTransferFunction {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
    pub center_freq: f32,
    pub gain_db: f32,
    pub q: f32,
}

impl EarCanalTransferFunction {
    /// Pass-through flat identity filter.
    pub const fn identity() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            center_freq: 3000.0,
            gain_db: 0.0,
            q: 1.0,
        }
    }

    /// Computes Robert Bristow-Johnson (RBJ) parametric peaking EQ coefficients for ear-canal boost or notch.
    pub fn parametric_eq(center_freq: f32, q: f32, gain_db: f32) -> Self {
        let clamped_q = q.clamp(0.1, 10.0);
        let clamped_gain = gain_db.clamp(-24.0, 24.0);

        if clamped_gain.abs() < 1e-4 {
            return Self::identity();
        }

        let nyquist = AUDIO_SAMPLE_RATE * 0.5;
        let clamped_freq = center_freq.clamp(50.0, nyquist - 200.0);

        let w0 = (TWO_PI * clamped_freq) / AUDIO_SAMPLE_RATE;
        let cos_w0 = cos_f32(w0);
        let sin_w0 = sin_f32(w0);
        let alpha = sin_w0 / (2.0 * clamped_q);

        let a = db_to_amplitude_ratio(clamped_gain);

        let b0_raw = 1.0 + alpha * a;
        let b1_raw = -2.0 * cos_w0;
        let b2_raw = 1.0 - alpha * a;
        let a0_raw = 1.0 + alpha / a;
        let a1_raw = -2.0 * cos_w0;
        let a2_raw = 1.0 - alpha / a;

        let inv_a0 = 1.0 / a0_raw;
        Self {
            b0: b0_raw * inv_a0,
            b1: b1_raw * inv_a0,
            b2: b2_raw * inv_a0,
            a1: a1_raw * inv_a0,
            a2: a2_raw * inv_a0,
            center_freq: clamped_freq,
            gain_db: clamped_gain,
            q: clamped_q,
        }
    }

    /// Evaluates the frequency magnitude response |H(e^{j*omega})| in linear scale.
    pub fn magnitude_response(&self, freq_hz: f32) -> f32 {
        let w = (TWO_PI * freq_hz) / AUDIO_SAMPLE_RATE;
        let cos_w = cos_f32(w);
        let cos_2w = cos_f32(2.0 * w);
        let sin_w = sin_f32(w);
        let sin_2w = sin_f32(2.0 * w);

        // Numerator N(e^jw) = b0 + b1*cos(w) + b2*cos(2w) + j*(-b1*sin(w) - b2*sin(2w))
        let num_re = self.b0 + self.b1 * cos_w + self.b2 * cos_2w;
        let num_im = -self.b1 * sin_w - self.b2 * sin_2w;
        let num_mag2 = num_re * num_re + num_im * num_im;

        // Denominator D(e^jw) = 1 + a1*cos(w) + a2*cos(2w) + j*(-a1*sin(w) - a2*sin(2w))
        let den_re = 1.0 + self.a1 * cos_w + self.a2 * cos_2w;
        let den_im = -self.a1 * sin_w - self.a2 * sin_2w;
        let den_mag2 = den_re * den_re + den_im * den_im;

        if den_mag2 > 1e-12 {
            sqrt_f32(num_mag2 / den_mag2)
        } else {
            1.0
        }
    }
}

/// Compensation Profile for diverse gaming audio transducer environments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompensationProfile {
    /// Raw uncompensated pass-through.
    Bypass,
    /// Over-ear gaming headphones: slight resonance compensation (-3.0 dB at 3.15 kHz).
    HeadphonesOverEar,
    /// In-Ear Monitors (IEMs): deep occlusion peak compensation (-6.5 dB at 3.15 kHz) to eliminate listening fatigue.
    InEarMonitorIEM,
    /// Handheld stereo speakers: cross-talk cancellation and presence enhancement.
    HandheldStereoSpeakersXTC,
}

impl CompensationProfile {
    pub fn get_transfer_function(self) -> EarCanalTransferFunction {
        match self {
            Self::Bypass => EarCanalTransferFunction::identity(),
            Self::HeadphonesOverEar => EarCanalTransferFunction::parametric_eq(
                EAR_CANAL_RESONANCE_FREQ,
                EAR_CANAL_RESONANCE_Q,
                -3.0,
            ),
            Self::InEarMonitorIEM => EarCanalTransferFunction::parametric_eq(
                EAR_CANAL_RESONANCE_FREQ,
                EAR_CANAL_RESONANCE_Q,
                -6.5,
            ),
            Self::HandheldStereoSpeakersXTC => EarCanalTransferFunction::parametric_eq(
                3000.0,
                2.0,
                2.5,
            ),
        }
    }
}

// -----------------------------------------------------------------------------
// Unit Tests for Commit 1
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn test_spatial_vector_and_listener_emitter() {
        let v1 = SpatialVector3::new(1.0, 2.0, 3.0);
        let v2 = SpatialVector3::new(4.0, 5.0, 6.0);
        let sum = v1 + v2;
        assert_eq!(sum, SpatialVector3::new(5.0, 7.0, 9.0));

        let listener = SpatialListener::new(SpatialVector3::zero());
        let emitter = SpatialEmitter::new(SpatialVector3::new(3.0, 0.0, 0.0)); // 3m to the right

        let left_ear = listener.get_left_ear_position();
        let right_ear = listener.get_right_ear_position();
        assert!(left_ear.x < 0.0);
        assert!(right_ear.x > 0.0);

        let binaural = BinauralParameters::calculate(&listener, &emitter);
        // Emitter is exactly on +X (azimuth ~ +PI/2)
        assert!(binaural.azimuth_rad > 1.4 && binaural.azimuth_rad < 1.7);
        assert!(binaural.itd_samples > 15.0 && binaural.itd_samples < 35.0);
        assert_eq!(binaural.right_delay_samples, 0.0); // right ear is ipsilateral
        assert!(binaural.left_delay_samples > 0.0);   // left ear is contralateral
        assert!(binaural.right_gain > binaural.left_gain); // right ear receives direct sound
    }

    #[test]
    pub fn test_fractional_delay_and_head_shadow_filter() {
        let mut delay = SubMillisecondDelayLine::new();
        for i in 0..64 {
            delay.write_sample(i as f32);
        }

        // Test integer delay (e.g. 5.0 samples)
        let val_5 = delay.read_hermite(5.0);
        // Delay 5 from 63 should be 58.0
        assert!((val_5 - 58.0).abs() < 1e-3);

        // Test fractional delay (e.g. 5.5 samples)
        let val_5_5_hermite = delay.read_hermite(5.5);
        let val_5_5_lagrange = delay.read_lagrange(5.5);
        assert!((val_5_5_hermite - 57.5).abs() < 0.1);
        assert!((val_5_5_lagrange - 57.5).abs() < 0.1);

        // Test Head Shadow Spherical Filter
        let mut filter = HeadShadowSphericalFilter::new();
        // Frontal incidence (theta = 0): response should be flat ~1.0
        filter.update_angle(0.0);
        let mag_front = filter.magnitude_response(5000.0);
        assert!((mag_front - 1.0).abs() < 0.05);

        // Ipsilateral ear (+PI/2): high-frequency presence boost
        filter.update_angle(HALF_PI);
        let mag_ipsi = filter.magnitude_response(5000.0);
        assert!(mag_ipsi > 1.2);

        // Contralateral ear (-PI/2): deep head shadow attenuation
        filter.update_angle(-HALF_PI);
        let mag_contra = filter.magnitude_response(5000.0);
        assert!(mag_contra < 0.4);
    }

    #[test]
    pub fn test_ear_canal_model_and_transfer_function() {
        let model = EarCanalModel::default_human();
        let calc_res = model.calculate_resonance_frequency();
        // Quarter-wave resonance should be around 3.0 to 3.4 kHz
        assert!(calc_res >= 3000.0 && calc_res <= 3300.0);

        let iem_tf = CompensationProfile::InEarMonitorIEM.get_transfer_function();
        assert_eq!(iem_tf.gain_db, -6.5);
        let mag_at_res = iem_tf.magnitude_response(calc_res);
        // At resonance frequency, magnitude should be significantly attenuated (< 0.7)
        assert!(mag_at_res < 0.7);

        let mag_at_low = iem_tf.magnitude_response(100.0);
        // At low frequencies, magnitude should remain close to 1.0 (pass-through)
        assert!((mag_at_low - 1.0).abs() < 0.05);
    }
}
