//! Real-Time In-Game 3D Audio Positional Occlusion & Acoustic Ray-Bouncing Reflection Engine.
//!
//! Provides spatial acoustic ray tracing between 3D audio emitters and listeners,
//! material sound transmission loss and low-pass filter (LPF) occlusion calculation,
//! first-order and second-order early reflection ray bouncers with fractional delay lines,
//! late reverb decay synthesis, and click-free audio parameter smoothing.

#![allow(dead_code)]

extern crate alloc;
use alloc::vec::Vec;

/// Speed of sound in dry air at 20 degrees Celsius in meters per second.
pub const SPEED_OF_SOUND_MPS: f32 = 343.0;

/// Standard audio sampling rate in Hz (48 kHz professional game audio).
pub const AUDIO_SAMPLE_RATE_HZ: f32 = 48000.0;

/// Number of samples corresponding to 1 meter of acoustic travel distance at 48 kHz.
pub const SAMPLES_PER_METER: f32 = AUDIO_SAMPLE_RATE_HZ / SPEED_OF_SOUND_MPS; // ~139.94

/// Maximum reflection delay line capacity in samples (~200 ms at 48 kHz = ~68 meters).
pub const MAX_DELAY_LINE_SAMPLES: usize = 9600;

/// Maximum number of active early reflection paths tracked per emitter.
pub const MAX_REFLECTION_PATHS: usize = 8;

// -----------------------------------------------------------------------------
// Pure no_std 3D Vector & Math Helpers
// -----------------------------------------------------------------------------

/// 3D Cartesian vector for acoustic spatial positioning and ray vectors.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AcousticVec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl AcousticVec3 {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub const fn zero() -> Self {
        Self { x: 0.0, y: 0.0, z: 0.0 }
    }

    #[inline]
    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    #[inline]
    pub fn cross(self, other: Self) -> Self {
        Self {
            x: self.y * other.z - self.z * other.y,
            y: self.z * other.x - self.x * other.z,
            z: self.x * other.y - self.y * other.x,
        }
    }

    #[inline]
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    #[inline]
    pub fn length(self) -> f32 {
        crate::graphics::types::sqrt_f32(self.length_squared())
    }

    #[inline]
    pub fn distance_to(self, other: Self) -> f32 {
        (other - self).length()
    }

    #[inline]
    pub fn normalized(self) -> Self {
        let len = self.length();
        if len > 1e-6 {
            Self {
                x: self.x / len,
                y: self.y / len,
                z: self.z / len,
            }
        } else {
            Self::zero()
        }
    }

    #[inline]
    pub fn reflect(self, normal: Self) -> Self {
        // R = V - 2 * (V . N) * N
        let d = self.dot(normal);
        Self {
            x: self.x - 2.0 * d * normal.x,
            y: self.y - 2.0 * d * normal.y,
            z: self.z - 2.0 * d * normal.z,
        }
    }
}

impl core::ops::Add for AcousticVec3 {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
            z: self.z + rhs.z,
        }
    }
}

impl core::ops::Sub for AcousticVec3 {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
            z: self.z - rhs.z,
        }
    }
}

impl core::ops::Mul<f32> for AcousticVec3 {
    type Output = Self;
    #[inline]
    fn mul(self, scalar: f32) -> Self {
        Self {
            x: self.x * scalar,
            y: self.y * scalar,
            z: self.z * scalar,
        }
    }
}

// -----------------------------------------------------------------------------
// Acoustic Materials & Transmission Coefficients
// -----------------------------------------------------------------------------

/// Physical material types defining acoustic wave interaction.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcousticMaterial {
    Air = 0,
    Concrete = 1,
    Glass = 2,
    Wood = 3,
    Metal = 4,
    Fabric = 5,
    Drywall = 6,
    Water = 7,
}

/// Acoustic transmission and frequency-dependent absorption coefficients for materials.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialAcousticProperties {
    /// Transmission loss in dB per nominal obstacle thickness (e.g. 15cm wall).
    pub transmission_loss_db: f32,
    /// Low-frequency absorption coefficient (125 Hz - 250 Hz) [0.0 = full reflection, 1.0 = full absorption].
    pub absorption_low: f32,
    /// Mid-frequency absorption coefficient (500 Hz - 2 kHz).
    pub absorption_mid: f32,
    /// High-frequency absorption coefficient (4 kHz - 8 kHz).
    pub absorption_high: f32,
    /// Surface scattering/diffusion coefficient [0.0 = perfect specular reflection, 1.0 = diffuse].
    pub scattering_coeff: f32,
    /// Nominal low-pass filter (LPF) cutoff frequency in Hz when fully occluded by this material.
    pub occluded_lpf_cutoff_hz: f32,
}

impl MaterialAcousticProperties {
    /// Retrieve standard acoustic properties for a given material based on empirical architectural acoustics.
    pub fn of(material: AcousticMaterial) -> Self {
        match material {
            AcousticMaterial::Air => Self {
                transmission_loss_db: 0.0,
                absorption_low: 0.001,
                absorption_mid: 0.002,
                absorption_high: 0.005,
                scattering_coeff: 0.0,
                occluded_lpf_cutoff_hz: 20000.0,
            },
            AcousticMaterial::Concrete => Self {
                transmission_loss_db: 32.0,
                absorption_low: 0.01,
                absorption_mid: 0.02,
                absorption_high: 0.05,
                scattering_coeff: 0.15,
                occluded_lpf_cutoff_hz: 380.0,
            },
            AcousticMaterial::Glass => Self {
                transmission_loss_db: 16.0,
                absorption_low: 0.04,
                absorption_mid: 0.03,
                absorption_high: 0.02,
                scattering_coeff: 0.05,
                occluded_lpf_cutoff_hz: 1850.0,
            },
            AcousticMaterial::Wood => Self {
                transmission_loss_db: 14.0,
                absorption_low: 0.15,
                absorption_mid: 0.10,
                absorption_high: 0.08,
                scattering_coeff: 0.25,
                occluded_lpf_cutoff_hz: 1200.0,
            },
            AcousticMaterial::Metal => Self {
                transmission_loss_db: 36.0,
                absorption_low: 0.02,
                absorption_mid: 0.01,
                absorption_high: 0.02,
                scattering_coeff: 0.10,
                occluded_lpf_cutoff_hz: 300.0,
            },
            AcousticMaterial::Fabric => Self {
                transmission_loss_db: 4.0,
                absorption_low: 0.10,
                absorption_mid: 0.40,
                absorption_high: 0.75,
                scattering_coeff: 0.50,
                occluded_lpf_cutoff_hz: 5500.0,
            },
            AcousticMaterial::Drywall => Self {
                transmission_loss_db: 18.0,
                absorption_low: 0.20,
                absorption_mid: 0.10,
                absorption_high: 0.05,
                scattering_coeff: 0.20,
                occluded_lpf_cutoff_hz: 950.0,
            },
            AcousticMaterial::Water => Self {
                transmission_loss_db: 24.0,
                absorption_low: 0.01,
                absorption_mid: 0.01,
                absorption_high: 0.02,
                scattering_coeff: 0.30,
                occluded_lpf_cutoff_hz: 600.0,
            },
        }
    }
}

// -----------------------------------------------------------------------------
// 3D Spatial Acoustic Ray Casting & Geometry Structures
// -----------------------------------------------------------------------------

/// Ray representation for acoustic sound paths and reflection bouncing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AcousticRay {
    /// Ray start point (emitter or previous bounce vertex).
    pub origin: AcousticVec3,
    /// Normalized propagation direction vector.
    pub direction: AcousticVec3,
    /// Maximum search distance in meters.
    pub max_distance: f32,
}

impl AcousticRay {
    pub fn new(origin: AcousticVec3, direction: AcousticVec3, max_distance: f32) -> Self {
        Self {
            origin,
            direction: direction.normalized(),
            max_distance,
        }
    }

    /// Point along the ray at distance `t` meters.
    #[inline]
    pub fn point_at(self, t: f32) -> AcousticVec3 {
        self.origin + self.direction * t
    }
}

/// Ray-surface hit intersection record.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AcousticHit {
    /// Distance in meters from ray origin to impact surface.
    pub distance: f32,
    /// Exact 3D point of impact in world space.
    pub hit_point: AcousticVec3,
    /// Surface normal vector pointing outward from the obstacle.
    pub normal: AcousticVec3,
    /// Material at the contact boundary.
    pub material: AcousticMaterial,
    /// Thickness of the penetrated obstacle in meters.
    pub thickness_m: f32,
}

/// Axis-Aligned Bounding Box (AABB) obstacle in 3D acoustic space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AcousticAabb {
    pub min: AcousticVec3,
    pub max: AcousticVec3,
    pub material: AcousticMaterial,
    pub wall_thickness_m: f32,
}

impl AcousticAabb {
    pub const fn new(
        min: AcousticVec3,
        max: AcousticVec3,
        material: AcousticMaterial,
        wall_thickness_m: f32,
    ) -> Self {
        Self { min, max, material, wall_thickness_m }
    }

    /// Fast slab test for ray-AABB intersection.
    pub fn intersect_ray(&self, ray: &AcousticRay) -> Option<AcousticHit> {
        let mut t_min = 0.0f32;
        let mut t_max = ray.max_distance;
        let mut hit_normal = AcousticVec3::zero();

        // X axis slab
        if ray.direction.x.abs() > 1e-6 {
            let inv_d = 1.0 / ray.direction.x;
            let mut t1 = (self.min.x - ray.origin.x) * inv_d;
            let mut t2 = (self.max.x - ray.origin.x) * inv_d;
            let mut n1 = AcousticVec3::new(if ray.direction.x > 0.0 { -1.0 } else { 1.0 }, 0.0, 0.0);
            if t1 > t2 {
                core::mem::swap(&mut t1, &mut t2);
                n1 = AcousticVec3::new(if ray.direction.x > 0.0 { 1.0 } else { -1.0 }, 0.0, 0.0);
            }
            if t1 > t_min {
                t_min = t1;
                hit_normal = n1;
            }
            if t2 < t_max {
                t_max = t2;
            }
            if t_min > t_max {
                return None;
            }
        } else if ray.origin.x < self.min.x || ray.origin.x > self.max.x {
            return None;
        }

        // Y axis slab
        if ray.direction.y.abs() > 1e-6 {
            let inv_d = 1.0 / ray.direction.y;
            let mut t1 = (self.min.y - ray.origin.y) * inv_d;
            let mut t2 = (self.max.y - ray.origin.y) * inv_d;
            let mut n1 = AcousticVec3::new(0.0, if ray.direction.y > 0.0 { -1.0 } else { 1.0 }, 0.0);
            if t1 > t2 {
                core::mem::swap(&mut t1, &mut t2);
                n1 = AcousticVec3::new(0.0, if ray.direction.y > 0.0 { 1.0 } else { -1.0 }, 0.0);
            }
            if t1 > t_min {
                t_min = t1;
                hit_normal = n1;
            }
            if t2 < t_max {
                t_max = t2;
            }
            if t_min > t_max {
                return None;
            }
        } else if ray.origin.y < self.min.y || ray.origin.y > self.max.y {
            return None;
        }

        // Z axis slab
        if ray.direction.z.abs() > 1e-6 {
            let inv_d = 1.0 / ray.direction.z;
            let mut t1 = (self.min.z - ray.origin.z) * inv_d;
            let mut t2 = (self.max.z - ray.origin.z) * inv_d;
            let mut n1 = AcousticVec3::new(0.0, 0.0, if ray.direction.z > 0.0 { -1.0 } else { 1.0 });
            if t1 > t2 {
                core::mem::swap(&mut t1, &mut t2);
                n1 = AcousticVec3::new(0.0, 0.0, if ray.direction.z > 0.0 { 1.0 } else { -1.0 });
            }
            if t1 > t_min {
                t_min = t1;
                hit_normal = n1;
            }
            if t2 < t_max {
                t_max = t2;
            }
            if t_min > t_max {
                return None;
            }
        } else if ray.origin.z < self.min.z || ray.origin.z > self.max.z {
            return None;
        }

        if t_min >= 0.0 && t_min <= ray.max_distance {
            Some(AcousticHit {
                distance: t_min,
                hit_point: ray.point_at(t_min),
                normal: hit_normal,
                material: self.material,
                thickness_m: self.wall_thickness_m,
            })
        } else {
            None
        }
    }
}

// -----------------------------------------------------------------------------
// Acoustic Reflection Delay Lines
// -----------------------------------------------------------------------------

/// Individual reflection path synthesized from ray bouncing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReflectionPath {
    /// Total propagation distance traveled by the reflected ray in meters.
    pub total_distance_m: f32,
    /// Propagation delay in samples at 48 kHz.
    pub delay_samples: f32,
    /// Total linear amplitude gain after distance law and material reflections.
    pub linear_gain: f32,
    /// High-frequency damping filter coefficient (one-pole low-pass).
    pub lpf_damping: f32,
    /// Order of the reflection (1 = primary wall bounce, 2 = secondary bounce).
    pub bounce_order: u8,
    /// Reflection normal vector of the last bounce.
    pub final_normal: AcousticVec3,
}

impl ReflectionPath {
    pub const fn empty() -> Self {
        Self {
            total_distance_m: 0.0,
            delay_samples: 0.0,
            linear_gain: 0.0,
            lpf_damping: 0.0,
            bounce_order: 0,
            final_normal: AcousticVec3::zero(),
        }
    }
}

/// Circular delay line buffer modeling acoustic propagation latency and reflection taps.
pub struct ReflectionDelayLine {
    /// Ring buffer storing past PCM audio samples.
    pub ring_buffer: Vec<f32>,
    /// Current write index in the ring buffer.
    pub write_cursor: usize,
    /// Buffer capacity in samples.
    pub capacity: usize,
    /// One-pole low-pass filter states per reflection tap.
    pub tap_filter_states: [f32; MAX_REFLECTION_PATHS],
}

impl ReflectionDelayLine {
    /// Allocate a new acoustic reflection delay line with given capacity.
    pub fn new(capacity_samples: usize) -> Self {
        let cap = if capacity_samples == 0 || capacity_samples > MAX_DELAY_LINE_SAMPLES {
            MAX_DELAY_LINE_SAMPLES
        } else {
            capacity_samples
        };

        Self {
            ring_buffer: alloc::vec![0.0f32; cap],
            write_cursor: 0,
            capacity: cap,
            tap_filter_states: [0.0f32; MAX_REFLECTION_PATHS],
        }
    }

    /// Push an input sample into the delay line ring.
    #[inline]
    pub fn push_sample(&mut self, sample: f32) {
        self.ring_buffer[self.write_cursor] = sample;
        self.write_cursor = (self.write_cursor + 1) % self.capacity;
    }

    /// Read a sample delayed by `delay_samples` using linear interpolation for click-free delay.
    #[inline]
    pub fn read_delayed_sample(&self, delay_samples: f32) -> f32 {
        if delay_samples <= 0.0 {
            let idx = (self.write_cursor + self.capacity - 1) % self.capacity;
            return self.ring_buffer[idx];
        }

        let clamped_delay = delay_samples.min((self.capacity - 2) as f32);
        let int_delay = clamped_delay as usize;
        let frac = clamped_delay - (int_delay as f32);

        // Compute circular indices
        let idx0 = (self.write_cursor + self.capacity - 1 - int_delay) % self.capacity;
        let idx1 = (self.write_cursor + self.capacity - 2 - int_delay) % self.capacity;

        let s0 = self.ring_buffer[idx0];
        let s1 = self.ring_buffer[idx1];

        s0 + (s1 - s0) * frac
    }

    /// Synthesize early reflections across all active reflection paths.
    pub fn process_reflections(
        &mut self,
        input_sample: f32,
        paths: &[ReflectionPath],
    ) -> f32 {
        self.push_sample(input_sample);

        let mut output = 0.0f32;
        let count = paths.len().min(MAX_REFLECTION_PATHS);

        for (i, path) in paths.iter().take(count).enumerate() {
            if path.linear_gain <= 1e-5 || path.delay_samples <= 0.0 {
                continue;
            }

            // Read delayed sample
            let raw_delayed = self.read_delayed_sample(path.delay_samples);

            // Apply one-pole frequency-dependent low-pass damping:
            // y[n] = y[n-1] + alpha * (x[n] - y[n-1])
            let prev_filtered = self.tap_filter_states[i];
            let alpha = (1.0 - path.lpf_damping).clamp(0.01, 1.0);
            let filtered = prev_filtered + alpha * (raw_delayed - prev_filtered);
            self.tap_filter_states[i] = filtered;

            output += filtered * path.linear_gain;
        }

        output
    }
}
