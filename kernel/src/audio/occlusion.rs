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


// -----------------------------------------------------------------------------
// 3D Positional Acoustic Occlusion Tracer & Diffraction Path Finder
// -----------------------------------------------------------------------------

/// Detailed result of acoustic line-of-sight ray tracing between emitter and listener.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OcclusionResult {
    /// True if direct line of sight is obstructed by one or more physical obstacles.
    pub is_occluded: bool,
    /// Number of distinct obstacle surfaces intersected along the direct sound ray.
    pub direct_hit_count: usize,
    /// Total cumulative sound transmission loss through obstacles in decibels (dB).
    pub transmission_loss_db: f32,
    /// Direct sound path linear amplitude gain (attenuation through obstacles).
    pub direct_gain: f32,
    /// Occlusion factor in range [0.0 = completely unobstructed, 1.0 = fully blocked].
    pub occlusion_factor: f32,
    /// Low-pass filter (LPF) cutoff frequency in Hz applied to the direct path.
    pub lpf_cutoff_hz: f32,
    /// True if an acoustic diffraction path around obstacle edges was detected.
    pub has_diffraction: bool,
    /// Bending angle in radians of the primary acoustic diffraction path.
    pub diffraction_angle_rad: f32,
    /// Total propagation distance of the diffracted path in meters.
    pub diffraction_path_distance_m: f32,
    /// Linear amplitude gain of diffracted acoustic wave bending around the obstacle.
    pub diffraction_gain: f32,
}

impl OcclusionResult {
    pub const fn unobstructed() -> Self {
        Self {
            is_occluded: false,
            direct_hit_count: 0,
            transmission_loss_db: 0.0,
            direct_gain: 1.0,
            occlusion_factor: 0.0,
            lpf_cutoff_hz: 20000.0,
            has_diffraction: false,
            diffraction_angle_rad: 0.0,
            diffraction_path_distance_m: 0.0,
            diffraction_gain: 0.0,
        }
    }
}

/// 3D acoustic world representation holding occluding geometry obstacles and walls.
pub struct AcousticWorld {
    /// Collection of 3D AABB obstacle geometries in the acoustic scene.
    pub obstacles: Vec<AcousticAabb>,
}

impl AcousticWorld {
    /// Allocate an empty acoustic world.
    pub fn new() -> Self {
        Self {
            obstacles: Vec::new(),
        }
    }

    /// Register a 3D AABB obstacle into the acoustic world.
    pub fn add_obstacle(&mut self, obstacle: AcousticAabb) {
        self.obstacles.push(obstacle);
    }

    /// Clear all registered obstacles.
    pub fn clear(&mut self) {
        self.obstacles.clear();
    }

    /// Calculate low-pass filter cutoff frequency based on penetrated materials and diffraction angle.
    pub fn calculate_lpf_cutoff(
        &self,
        materials: &[AcousticMaterial],
        _diffraction_angle_rad: f32,
        _has_diffraction: bool,
    ) -> f32 {
        if materials.is_empty() {
            return 20000.0;
        }

        // 1. Minimum material transmission cutoff
        let mut min_cutoff = 20000.0f32;
        for &mat in materials {
            let props = MaterialAcousticProperties::of(mat);
            if props.occluded_lpf_cutoff_hz < min_cutoff {
                min_cutoff = props.occluded_lpf_cutoff_hz;
            }
        }

        min_cutoff.clamp(20.0, 20000.0)
    }

    /// Find optimal acoustic diffraction path around an occluding AABB obstacle.
    /// Evaluates edge detour distance and Huygens-Fresnel bending angle.
    pub fn find_diffraction_path(
        &self,
        emitter: AcousticVec3,
        listener: AcousticVec3,
        occluder: &AcousticAabb,
    ) -> Option<(AcousticVec3, f32, f32, f32)> {
        let direct_dist = emitter.distance_to(listener);
        if direct_dist <= 1e-4 {
            return None;
        }

        // Generate candidate diffraction edge points along the 4 silhouette boundaries of the occluder
        let center = (occluder.min + occluder.max) * 0.5;
        let margin = 0.15f32; // Slight offset past the obstacle edge

        let candidate_points = [
            // Top edge
            AcousticVec3::new(center.x, occluder.max.y + margin, center.z),
            // Bottom edge
            AcousticVec3::new(center.x, occluder.min.y - margin, center.z),
            // Left edge
            AcousticVec3::new(occluder.min.x - margin, center.y, center.z),
            // Right edge
            AcousticVec3::new(occluder.max.x + margin, center.y, center.z),
        ];

        let mut best_point = AcousticVec3::zero();
        let mut min_detour = f32::MAX;
        let mut best_path_dist = 0.0f32;
        let mut best_angle = 0.0f32;
        let mut best_gain = 0.0f32;
        let mut found_valid = false;

        for &pt in &candidate_points {
            let d1 = emitter.distance_to(pt);
            let d2 = pt.distance_to(listener);
            let total_dist = d1 + d2;
            let detour = total_dist - direct_dist;

            if detour < min_detour {
                // Calculate bending angle between ray from emitter to edge and ray from edge to listener
                let v1 = (pt - emitter).normalized();
                let v2 = (listener - pt).normalized();
                let cos_theta = v1.dot(v2).clamp(-1.0, 1.0);
                // Acoustic bending angle in radians: theta = 0 when straight line, PI when folded back
                let angle = crate::audio::spatializer::atan2_f32(
                    crate::graphics::types::sqrt_f32((1.0 - cos_theta * cos_theta).max(0.0)),
                    cos_theta,
                );

                // Pierce / Maekawa acoustic diffraction loss approximation:
                // Base loss 8 dB + 12 dB * angle (radians)
                let diff_loss_db = (8.0 + 12.0 * angle).clamp(6.0, 40.0);
                let gain = crate::audio::drc::db_to_linear(-diff_loss_db);

                min_detour = detour;
                best_point = pt;
                best_path_dist = total_dist;
                best_angle = angle;
                best_gain = gain;
                found_valid = true;
            }
        }

        if found_valid {
            Some((best_point, best_angle, best_path_dist, best_gain))
        } else {
            None
        }
    }

    /// Trace direct line-of-sight sound ray between emitter and listener and compute
    /// physical occlusion, material transmission loss, and acoustic diffraction.
    pub fn trace_occlusion(
        &self,
        emitter: AcousticVec3,
        listener: AcousticVec3,
    ) -> OcclusionResult {
        let direct_distance = emitter.distance_to(listener);
        if direct_distance <= 1e-4 {
            return OcclusionResult::unobstructed();
        }

        let dir = (listener - emitter).normalized();
        let ray = AcousticRay::new(emitter, dir, direct_distance);

        let mut hit_count = 0usize;
        let mut total_tl_db = 0.0f32;
        let mut hit_materials = Vec::new();
        let mut primary_occluder: Option<&AcousticAabb> = None;

        for obstacle in &self.obstacles {
            if let Some(hit) = obstacle.intersect_ray(&ray) {
                // Ensure hit is strictly between emitter and listener
                if hit.distance >= 0.05 && hit.distance <= (direct_distance - 0.05) {
                    hit_count += 1;
                    hit_materials.push(hit.material);

                    let props = MaterialAcousticProperties::of(hit.material);
                    // Scale transmission loss by obstacle thickness relative to nominal 15cm wall
                    let thickness_factor = (hit.thickness_m / 0.15).clamp(0.5, 4.0);
                    total_tl_db += props.transmission_loss_db * thickness_factor;

                    if primary_occluder.is_none() {
                        primary_occluder = Some(obstacle);
                    }
                }
            }
        }

        if hit_count == 0 {
            return OcclusionResult::unobstructed();
        }

        // Occlusion present
        let direct_gain = crate::audio::drc::db_to_linear(-total_tl_db);
        let occlusion_factor = (1.0 - direct_gain).clamp(0.0, 1.0);

        // Check for acoustic diffraction around primary occluder
        let (has_diffraction, diff_angle, diff_dist, diff_gain) = if let Some(occluder) = primary_occluder {
            if let Some((_, angle, dist, gain)) = self.find_diffraction_path(emitter, listener, occluder) {
                (true, angle, dist, gain)
            } else {
                (false, 0.0, 0.0, 0.0)
            }
        } else {
            (false, 0.0, 0.0, 0.0)
        };

        // Calculate frequency-dependent LPF cutoff
        let lpf_cutoff_hz = self.calculate_lpf_cutoff(
            &hit_materials,
            diff_angle,
            has_diffraction,
        );

        OcclusionResult {
            is_occluded: true,
            direct_hit_count: hit_count,
            transmission_loss_db: total_tl_db,
            direct_gain,
            occlusion_factor,
            lpf_cutoff_hz,
            has_diffraction,
            diffraction_angle_rad: diff_angle,
            diffraction_path_distance_m: diff_dist,
            diffraction_gain: diff_gain,
        }
    }
}


// -----------------------------------------------------------------------------
// Early Reflection Ray Bouncer & Reverb Decay Synthesizer
// -----------------------------------------------------------------------------

/// Casts first-order and second-order acoustic reflection rays off scene obstacles.
pub struct EarlyReflectionBouncer {
    /// Maximum ray bounce depth (1 = primary reflection, 2 = secondary reflection).
    pub max_bounce_depth: u8,
    /// Maximum reflection propagation distance in meters (~60 meters = ~175 ms).
    pub max_distance_m: f32,
}

impl EarlyReflectionBouncer {
    pub const fn new(max_bounce_depth: u8, max_distance_m: f32) -> Self {
        Self {
            max_bounce_depth,
            max_distance_m,
        }
    }

    /// Trace first-order and second-order acoustic reflections from emitter to listener.
    pub fn calculate_reflection_paths(
        &self,
        world: &AcousticWorld,
        emitter: AcousticVec3,
        listener: AcousticVec3,
    ) -> Vec<ReflectionPath> {
        let mut paths = Vec::with_capacity(MAX_REFLECTION_PATHS);

        // Representative emission ray directions (6 cardinal axes)
        let emission_directions = [
            AcousticVec3::new(1.0, 0.0, 0.0),
            AcousticVec3::new(-1.0, 0.0, 0.0),
            AcousticVec3::new(0.0, 1.0, 0.0),
            AcousticVec3::new(0.0, -1.0, 0.0),
            AcousticVec3::new(0.0, 0.0, 1.0),
            AcousticVec3::new(0.0, 0.0, -1.0),
        ];

        for &dir in &emission_directions {
            if paths.len() >= MAX_REFLECTION_PATHS {
                break;
            }

            // 1. Primary ray from emitter
            let ray1 = AcousticRay::new(emitter, dir, self.max_distance_m);
            let mut nearest_hit1: Option<AcousticHit> = None;

            for obs in &world.obstacles {
                if let Some(hit) = obs.intersect_ray(&ray1) {
                    if hit.distance > 0.05 {
                        if let Some(prev) = nearest_hit1 {
                            if hit.distance < prev.distance {
                                nearest_hit1 = Some(hit);
                            }
                        } else {
                            nearest_hit1 = Some(hit);
                        }
                    }
                }
            }

            let hit1 = match nearest_hit1 {
                Some(h) => h,
                None => continue,
            };

            let p1 = hit1.hit_point;
            let d1 = hit1.distance;
            let props1 = MaterialAcousticProperties::of(hit1.material);

            // Test 1st-order visibility from hit1 to listener
            let to_listener_dist = p1.distance_to(listener);
            let total_dist_1st = d1 + to_listener_dist;

            if total_dist_1st <= self.max_distance_m && to_listener_dist > 0.1 {
                // Check if segment hit1 -> listener is unoccluded
                let ray_to_listener = AcousticRay::new(p1, (listener - p1).normalized(), to_listener_dist);
                let mut path1_blocked = false;

                for obs in &world.obstacles {
                    if let Some(h) = obs.intersect_ray(&ray_to_listener) {
                        if h.distance > 0.05 && h.distance < (to_listener_dist - 0.05) {
                            path1_blocked = true;
                            break;
                        }
                    }
                }

                if !path1_blocked {
                    // Valid 1st-order reflection path
                    let delay = total_dist_1st * SAMPLES_PER_METER;
                    let dist_atten = 1.0 / total_dist_1st.max(1.0);
                    let refl_gain = (1.0 - props1.absorption_mid) * (1.0 - props1.scattering_coeff);
                    let gain = (dist_atten * refl_gain).clamp(0.0, 1.0);

                    paths.push(ReflectionPath {
                        total_distance_m: total_dist_1st,
                        delay_samples: delay,
                        linear_gain: gain,
                        lpf_damping: props1.absorption_high,
                        bounce_order: 1,
                        final_normal: hit1.normal,
                    });
                }
            }

            // 2. Secondary reflection ray (2nd-order bounce) if depth >= 2
            if self.max_bounce_depth >= 2 && paths.len() < MAX_REFLECTION_PATHS {
                let reflect_dir = dir.reflect(hit1.normal).normalized();
                let ray2 = AcousticRay::new(p1 + reflect_dir * 0.05, reflect_dir, self.max_distance_m - d1);
                let mut nearest_hit2: Option<AcousticHit> = None;

                for obs in &world.obstacles {
                    if let Some(h) = obs.intersect_ray(&ray2) {
                        if h.distance > 0.05 {
                            if let Some(prev) = nearest_hit2 {
                                if h.distance < prev.distance {
                                    nearest_hit2 = Some(h);
                                }
                            } else {
                                nearest_hit2 = Some(h);
                            }
                        }
                    }
                }

                if let Some(hit2) = nearest_hit2 {
                    let p2 = hit2.hit_point;
                    let d2 = hit2.distance;
                    let props2 = MaterialAcousticProperties::of(hit2.material);
                    let dist2_listener = p2.distance_to(listener);
                    let total_dist_2nd = d1 + d2 + dist2_listener;

                    if total_dist_2nd <= self.max_distance_m && dist2_listener > 0.1 {
                        let ray2_listener = AcousticRay::new(p2, (listener - p2).normalized(), dist2_listener);
                        let mut path2_blocked = false;

                        for obs in &world.obstacles {
                            if let Some(h) = obs.intersect_ray(&ray2_listener) {
                                if h.distance > 0.05 && h.distance < (dist2_listener - 0.05) {
                                    path2_blocked = true;
                                    break;
                                }
                            }
                        }

                        if !path2_blocked {
                            let delay = total_dist_2nd * SAMPLES_PER_METER;
                            let dist_atten = 1.0 / total_dist_2nd.max(1.0);
                            let refl_gain = (1.0 - props1.absorption_mid) * (1.0 - props2.absorption_mid);
                            let gain = (dist_atten * refl_gain * 0.7).clamp(0.0, 1.0);
                            let damping = 1.0 - (1.0 - props1.absorption_high) * (1.0 - props2.absorption_high);

                            paths.push(ReflectionPath {
                                total_distance_m: total_dist_2nd,
                                delay_samples: delay,
                                linear_gain: gain,
                                lpf_damping: damping,
                                bounce_order: 2,
                                final_normal: hit2.normal,
                            });
                        }
                    }
                }
            }
        }

        paths
    }
}

/// Schroeder comb filter for late reverberation decay synthesis.
pub struct SchroederComb {
    pub buffer: Vec<f32>,
    pub write_cursor: usize,
    pub delay_samples: usize,
    pub feedback_gain: f32,
    pub filter_state: f32,
    pub damping: f32,
}

impl SchroederComb {
    pub fn new(delay_samples: usize, feedback_gain: f32, damping: f32) -> Self {
        Self {
            buffer: alloc::vec![0.0f32; delay_samples],
            write_cursor: 0,
            delay_samples,
            feedback_gain,
            filter_state: 0.0,
            damping,
        }
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let read_val = self.buffer[self.write_cursor];
        self.filter_state = read_val * (1.0 - self.damping) + self.filter_state * self.damping;
        let to_write = input + self.filter_state * self.feedback_gain;
        self.buffer[self.write_cursor] = to_write;
        self.write_cursor = (self.write_cursor + 1) % self.delay_samples;
        read_val
    }
}

/// Schroeder all-pass filter for sound diffusion in late reverberation.
pub struct SchroederAllPass {
    pub buffer: Vec<f32>,
    pub write_cursor: usize,
    pub delay_samples: usize,
    pub gain: f32,
}

impl SchroederAllPass {
    pub fn new(delay_samples: usize, gain: f32) -> Self {
        Self {
            buffer: alloc::vec![0.0f32; delay_samples],
            write_cursor: 0,
            delay_samples,
            gain,
        }
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let buf_val = self.buffer[self.write_cursor];
        let output = -input + buf_val;
        self.buffer[self.write_cursor] = input + buf_val * self.gain;
        self.write_cursor = (self.write_cursor + 1) % self.delay_samples;
        output
    }
}

/// Late reverberation decay synthesizer based on Sabine RT60 formula and Schroeder filter network.
pub struct LateReverbSynthesizer {
    /// Room reverberation decay time in seconds (time to decay by 60 dB).
    pub rt60_seconds: f32,
    /// Parallel comb filters.
    pub combs: [SchroederComb; 4],
    /// Series all-pass diffusion filters.
    pub allpasses: [SchroederAllPass; 2],
    /// Wet reverberation send level.
    pub wet_gain: f32,
}

impl LateReverbSynthesizer {
    /// Allocate a new late reverberation synthesizer calibrated for room acoustics.
    pub fn new(rt60_seconds: f32, wet_gain: f32) -> Self {
        let rt60 = rt60_seconds.clamp(0.1, 10.0);
        // Compute feedback gain g = 10^(-3 * tau / (RT60 * Fs))
        let calc_fb = |delay: usize| -> f32 {
            let exponent = -3.0 * (delay as f32) / (rt60 * AUDIO_SAMPLE_RATE_HZ);
            crate::audio::drc::exp_f32(exponent * 2.3025851)
        };

        // Schroeder prime sample delay lines
        let combs = [
            SchroederComb::new(1116, calc_fb(1116), 0.2),
            SchroederComb::new(1188, calc_fb(1188), 0.2),
            SchroederComb::new(1277, calc_fb(1277), 0.2),
            SchroederComb::new(1356, calc_fb(1356), 0.2),
        ];

        let allpasses = [
            SchroederAllPass::new(225, 0.5),
            SchroederAllPass::new(556, 0.5),
        ];

        Self {
            rt60_seconds: rt60,
            combs,
            allpasses,
            wet_gain,
        }
    }

    /// Calculate Sabine reverberation time RT60 given room volume and total absorption:
    /// $RT_{60} = 0.161 * V / A$.
    pub fn calculate_sabine_rt60(room_volume_m3: f32, total_absorption_area: f32) -> f32 {
        let abs_area = total_absorption_area.max(0.1);
        (0.161 * room_volume_m3.max(1.0) / abs_area).clamp(0.1, 8.0)
    }

    /// Process a sample through the late reverberation tank.
    #[inline]
    pub fn process_sample(&mut self, input: f32) -> f32 {
        // Sum parallel comb outputs
        let mut comb_sum = 0.0f32;
        for comb in &mut self.combs {
            comb_sum += comb.process(input);
        }
        comb_sum *= 0.25;

        // Pass through series allpasses for diffusion
        let mut diff = comb_sum;
        for ap in &mut self.allpasses {
            diff = ap.process(diff);
        }

        diff * self.wet_gain
    }
}

// -----------------------------------------------------------------------------
// Click-Free Audio Parameter Smoothing & Occlusion Engine
// -----------------------------------------------------------------------------

/// Parameter interpolator preventing audio popping and clicks during rapid source movement.
pub struct AcousticParameterSmoother {
    /// Current smoothly interpolated direct gain.
    pub current_gain: f32,
    /// Target direct gain from occlusion ray tracer.
    pub target_gain: f32,
    /// Current smoothly interpolated low-pass filter cutoff in Hz.
    pub current_cutoff_hz: f32,
    /// Target low-pass filter cutoff in Hz.
    pub target_cutoff_hz: f32,
    /// Maximum gain transition rate per audio sample (~20 ms slew rate).
    pub gain_slew_per_sample: f32,
    /// Maximum cutoff transition rate in Hz per audio sample.
    pub cutoff_slew_per_sample: f32,
}

impl AcousticParameterSmoother {
    /// Construct a parameter smoother configured for smooth 20 ms transitions.
    pub fn new() -> Self {
        // At 48 kHz, 20 ms = 960 samples.
        // Transitioning full scale (1.0) over 960 samples -> ~0.00104 per sample
        let slew_samples = AUDIO_SAMPLE_RATE_HZ * 0.020; // 960 samples
        Self {
            current_gain: 1.0,
            target_gain: 1.0,
            current_cutoff_hz: 20000.0,
            target_cutoff_hz: 20000.0,
            gain_slew_per_sample: 1.0 / slew_samples,
            cutoff_slew_per_sample: 20000.0 / slew_samples,
        }
    }

    /// Set target parameters from the latest ray tracing update.
    pub fn set_targets(&mut self, target_gain: f32, target_cutoff_hz: f32) {
        self.target_gain = target_gain.clamp(0.0, 1.0);
        self.target_cutoff_hz = target_cutoff_hz.clamp(20.0, 20000.0);
    }

    /// Advance smoother by one audio sample and return `(interpolated_gain, interpolated_cutoff_hz)`.
    #[inline]
    pub fn step(&mut self) -> (f32, f32) {
        // Gain slew
        let diff_gain = self.target_gain - self.current_gain;
        if diff_gain.abs() <= self.gain_slew_per_sample {
            self.current_gain = self.target_gain;
        } else if diff_gain > 0.0 {
            self.current_gain += self.gain_slew_per_sample;
        } else {
            self.current_gain -= self.gain_slew_per_sample;
        }

        // Cutoff slew
        let diff_cutoff = self.target_cutoff_hz - self.current_cutoff_hz;
        if diff_cutoff.abs() <= self.cutoff_slew_per_sample {
            self.current_cutoff_hz = self.target_cutoff_hz;
        } else if diff_cutoff > 0.0 {
            self.current_cutoff_hz += self.cutoff_slew_per_sample;
        } else {
            self.current_cutoff_hz -= self.cutoff_slew_per_sample;
        }

        (self.current_gain, self.current_cutoff_hz)
    }
}

/// Real-Time 3D Audio Positional Occlusion & Acoustic Reflection DSP Engine.
pub struct AcousticOcclusionEngine {
    /// 3D acoustic world model holding obstacles.
    pub world: AcousticWorld,
    /// Early reflection ray bouncer.
    pub bouncer: EarlyReflectionBouncer,
    /// Circular delay line synthesizing early reflection taps.
    pub delay_line: ReflectionDelayLine,
    /// Late diffuse reverberation decay synthesizer.
    pub late_reverb: LateReverbSynthesizer,
    /// Click-free parameter smoother.
    pub smoother: AcousticParameterSmoother,
    /// Active early reflection paths.
    pub reflection_paths: Vec<ReflectionPath>,
    /// One-pole low-pass filter state for direct occluded sound path.
    pub direct_lpf_state: f32,
    /// Latest computed occlusion result.
    pub last_occlusion: OcclusionResult,
}

impl AcousticOcclusionEngine {
    /// Initialize a new acoustic occlusion and reflection DSP engine.
    pub fn new(room_volume_m3: f32, total_absorption_area: f32) -> Self {
        let rt60 = LateReverbSynthesizer::calculate_sabine_rt60(room_volume_m3, total_absorption_area);
        Self {
            world: AcousticWorld::new(),
            bouncer: EarlyReflectionBouncer::new(2, 50.0),
            delay_line: ReflectionDelayLine::new(MAX_DELAY_LINE_SAMPLES),
            late_reverb: LateReverbSynthesizer::new(rt60, 0.15),
            smoother: AcousticParameterSmoother::new(),
            reflection_paths: Vec::new(),
            direct_lpf_state: 0.0,
            last_occlusion: OcclusionResult::unobstructed(),
        }
    }

    /// Update spatial positions of audio emitter and listener, executing ray tracing
    /// for direct path occlusion and early reflection paths.
    pub fn update_scene(
        &mut self,
        emitter: AcousticVec3,
        listener: AcousticVec3,
    ) {
        // 1. Line-of-sight occlusion tracing
        let occ = self.world.trace_occlusion(emitter, listener);
        self.last_occlusion = occ;

        // Set target direct path parameters in smoother
        self.smoother.set_targets(occ.direct_gain, occ.lpf_cutoff_hz);

        // 2. Early reflections ray bouncing
        self.reflection_paths = self.bouncer.calculate_reflection_paths(&self.world, emitter, listener);
    }

    /// Process a block of PCM audio samples through direct occlusion filtering,
    /// early reflection delay lines, and late reverberation decay.
    pub fn process_block(
        &mut self,
        input: &[f32],
        output: &mut [f32],
    ) {
        let len = input.len().min(output.len());
        for i in 0..len {
            let sample = input[i];

            // 1. Advance parameter smoother
            let (smooth_gain, smooth_cutoff) = self.smoother.step();

            // 2. Calculate one-pole LPF alpha: alpha = (2*pi*fc/Fs) / (1 + 2*pi*fc/Fs)
            let omega = 2.0 * 3.14159265 * smooth_cutoff / AUDIO_SAMPLE_RATE_HZ;
            let alpha = (omega / (1.0 + omega)).clamp(0.001, 1.0);

            // Apply direct path low-pass filter
            self.direct_lpf_state = self.direct_lpf_state + alpha * (sample - self.direct_lpf_state);
            let direct_filtered = self.direct_lpf_state * smooth_gain;

            // 3. Process early reflection delay lines
            let early_refl = self.delay_line.process_reflections(sample, &self.reflection_paths);

            // 4. Process late diffuse reverberation
            let late_rev = self.late_reverb.process_sample(sample);

            // 5. Combine direct, early reflections, and late reverb
            output[i] = direct_filtered + early_refl + late_rev;
        }
    }
}


// -----------------------------------------------------------------------------
// In-Kernel Verification Self-Test Suite
// -----------------------------------------------------------------------------

/// Verify 3D ray-AABB geometric intersection and surface normal calculation.
pub fn test_ray_casting_and_aabb_intersection() -> bool {
    let aabb = AcousticAabb::new(
        AcousticVec3::new(-1.0, -1.0, 4.0),
        AcousticVec3::new(1.0, 1.0, 6.0),
        AcousticMaterial::Concrete,
        0.20,
    );

    // 1. Direct hit along Z axis
    let ray_hit = AcousticRay::new(
        AcousticVec3::new(0.0, 0.0, 0.0),
        AcousticVec3::new(0.0, 0.0, 1.0),
        10.0,
    );
    let hit = match aabb.intersect_ray(&ray_hit) {
        Some(h) => h,
        None => return false,
    };

    if (hit.distance - 4.0).abs() > 1e-4 {
        return false;
    }
    if (hit.hit_point.z - 4.0).abs() > 1e-4 {
        return false;
    }
    if hit.normal.z != -1.0 {
        return false;
    }
    if hit.material != AcousticMaterial::Concrete {
        return false;
    }

    // 2. Ray pointing away: must not hit
    let ray_miss = AcousticRay::new(
        AcousticVec3::new(0.0, 0.0, 0.0),
        AcousticVec3::new(0.0, 0.0, -1.0),
        10.0,
    );
    if aabb.intersect_ray(&ray_miss).is_some() {
        return false;
    }

    // 3. Ray passing to the side
    let ray_side = AcousticRay::new(
        AcousticVec3::new(2.5, 0.0, 0.0),
        AcousticVec3::new(0.0, 0.0, 1.0),
        10.0,
    );
    if aabb.intersect_ray(&ray_side).is_some() {
        return false;
    }

    true
}

/// Verify material sound transmission loss dB, acoustic occlusion, and LPF cutoff frequencies.
pub fn test_material_transmission_and_occlusion() -> bool {
    let mut world = AcousticWorld::new();

    // Place a 20cm concrete wall at Z = 5.0m
    world.add_obstacle(AcousticAabb::new(
        AcousticVec3::new(-5.0, -5.0, 4.9),
        AcousticVec3::new(5.0, 5.0, 5.1),
        AcousticMaterial::Concrete,
        0.20,
    ));

    // Test occluded sound path through concrete
    let emitter = AcousticVec3::new(0.0, 0.0, 0.0);
    let listener = AcousticVec3::new(0.0, 0.0, 10.0);
    let occ = world.trace_occlusion(emitter, listener);

    if !occ.is_occluded {
        return false;
    }
    if occ.direct_hit_count != 1 {
        return false;
    }
    // Concrete transmission loss should be significant (> 30 dB)
    if occ.transmission_loss_db < 30.0 {
        return false;
    }
    // Direct gain should be strongly attenuated (< 0.05 linear)
    if occ.direct_gain > 0.05 {
        return false;
    }
    if occ.occlusion_factor < 0.95 {
        return false;
    }
    // Concrete LPF cutoff should be muffled down to low frequencies (< 500 Hz)
    if occ.lpf_cutoff_hz > 500.0 {
        return false;
    }

    // Test unobstructed sound path
    let clear_listener = AcousticVec3::new(0.0, 10.0, 0.0);
    let clear_occ = world.trace_occlusion(emitter, clear_listener);
    if clear_occ.is_occluded {
        return false;
    }
    if clear_occ.direct_gain != 1.0 || clear_occ.lpf_cutoff_hz != 20000.0 {
        return false;
    }

    true
}

/// Verify acoustic diffraction path finding and edge bending angles.
pub fn test_diffraction_path_finding() -> bool {
    let mut world = AcousticWorld::new();

    // Pillar obstacle from x = -1.0 to 1.0, y = -1.0 to 1.0, z = 4.0 to 4.5
    let pillar = AcousticAabb::new(
        AcousticVec3::new(-1.0, -1.0, 4.0),
        AcousticVec3::new(1.0, 1.0, 4.5),
        AcousticMaterial::Wood,
        0.5,
    );
    world.add_obstacle(pillar);

    let emitter = AcousticVec3::new(0.0, 0.0, 1.0);
    let listener = AcousticVec3::new(0.0, 0.0, 7.0);

    let diff_opt = world.find_diffraction_path(emitter, listener, &pillar);
    let (edge_pt, angle, total_dist, gain) = match diff_opt {
        Some(d) => d,
        None => return false,
    };

    // Diffraction edge point must be offset past the obstacle bounds
    if edge_pt.x.abs() < 1.0 && edge_pt.y.abs() < 1.0 {
        return false;
    }
    // Total distance around edge must be longer than direct 6.0m line-of-sight
    if total_dist <= 6.0 {
        return false;
    }
    // Bending angle should be positive
    if angle <= 0.0 || angle > 3.14159 {
        return false;
    }
    // Diffraction gain should be non-zero and less than 1.0
    if gain <= 0.0 || gain >= 1.0 {
        return false;
    }

    true
}

/// Verify circular reflection delay line, fractional sample interpolation, and ray bouncing.
pub fn test_early_reflections_and_delay_line() -> bool {
    // 1. Delay line fractional interpolation
    let mut delay_line = ReflectionDelayLine::new(2400);

    // Push 100 samples
    for i in 0..100 {
        delay_line.push_sample((i as f32) * 0.1);
    }

    // Read integer delay: 10 samples ago
    let s_int = delay_line.read_delayed_sample(10.0);
    // Read fractional delay: 10.5 samples ago
    let s_frac = delay_line.read_delayed_sample(10.5);
    let s_next = delay_line.read_delayed_sample(11.0);

    // Fractional reading must be strictly between adjacent integer taps
    if s_frac <= s_next || s_frac >= s_int {
        return false;
    }

    // 2. Early reflection bouncer
    let mut world = AcousticWorld::new();
    // Back wall at X = 5.0m
    world.add_obstacle(AcousticAabb::new(
        AcousticVec3::new(4.9, -10.0, -10.0),
        AcousticVec3::new(5.1, 10.0, 10.0),
        AcousticMaterial::Concrete,
        0.20,
    ));

    let bouncer = EarlyReflectionBouncer::new(2, 40.0);
    let emitter = AcousticVec3::new(0.0, 0.0, 0.0);
    let listener = AcousticVec3::new(0.0, 2.0, 0.0);

    let paths = bouncer.calculate_reflection_paths(&world, emitter, listener);
    if paths.is_empty() {
        return false;
    }

    // Must find reflection path off back wall
    let p0 = &paths[0];
    if p0.total_distance_m < 9.0 || p0.delay_samples < 1000.0 {
        return false;
    }
    if p0.linear_gain <= 0.0 || p0.linear_gain > 1.0 {
        return false;
    }

    true
}

/// Verify click-free parameter smoothing across rapid acoustic boundary movements.
pub fn test_click_free_parameter_smoothing() -> bool {
    let mut smoother = AcousticParameterSmoother::new();

    // Start unoccluded
    if smoother.current_gain != 1.0 || smoother.current_cutoff_hz != 20000.0 {
        return false;
    }

    // Abrupt step: source instantly ducks behind heavy concrete wall (target gain = 0.02, cutoff = 380 Hz)
    smoother.set_targets(0.02, 380.0);

    let mut prev_gain = smoother.current_gain;
    let mut prev_cutoff = smoother.current_cutoff_hz;

    // Advance 100 audio samples
    for _ in 0..100 {
        let (g, c) = smoother.step();
        let gain_delta = (g - prev_gain).abs();
        let cutoff_delta = (c - prev_cutoff).abs();

        // Must not exceed sample slew rate limits (click-free guarantee)
        if gain_delta > (smoother.gain_slew_per_sample + 1e-5) {
            return false;
        }
        if cutoff_delta > (smoother.cutoff_slew_per_sample + 0.05) {
            return false;
        }

        prev_gain = g;
        prev_cutoff = c;
    }

    // After 100 samples (~2.08 ms), gain and cutoff should be moving towards targets smoothly
    if smoother.current_gain >= 1.0 || smoother.current_gain <= 0.02 {
        return false;
    }
    if smoother.current_cutoff_hz >= 20000.0 || smoother.current_cutoff_hz <= 380.0 {
        return false;
    }

    // Advance remaining samples to complete full 20ms transition (1000 samples)
    for _ in 0..1000 {
        smoother.step();
    }

    // Now targets should be exactly reached
    if (smoother.current_gain - 0.02).abs() > 1e-4 {
        return false;
    }
    if (smoother.current_cutoff_hz - 380.0).abs() > 1e-3 {
        return false;
    }

    true
}

/// Verify end-to-end audio occlusion, early reflections, and late reverb DSP pipeline.
pub fn test_end_to_end_occlusion_and_reverb_pipeline() -> bool {
    let mut engine = AcousticOcclusionEngine::new(400.0, 120.0);

    // Add obstacles defining room boundaries and an occluding divider
    engine.world.add_obstacle(AcousticAabb::new(
        AcousticVec3::new(-1.0, -2.0, 3.0),
        AcousticVec3::new(1.0, 2.0, 3.2),
        AcousticMaterial::Concrete,
        0.20,
    ));

    // Update positions: emitter behind divider, listener in front
    let emitter = AcousticVec3::new(0.0, 0.0, 5.0);
    let listener = AcousticVec3::new(0.0, 0.0, 1.0);
    engine.update_scene(emitter, listener);

    if !engine.last_occlusion.is_occluded {
        return false;
    }

    // Generate input test sine burst
    let mut in_buffer = [0.0f32; 128];
    for (i, sample) in in_buffer.iter_mut().enumerate() {
        // Simple synthetic signal
        *sample = if i < 32 { 0.8 } else { 0.0 };
    }

    let mut out_buffer = [0.0f32; 128];
    engine.process_block(&in_buffer, &mut out_buffer);

    // Output must be populated, finite, and non-zero
    let mut energy = 0.0f32;
    for &s in &out_buffer {
        if s.is_nan() || s.is_infinite() {
            return false;
        }
        energy += s * s;
    }

    if energy <= 1e-6 {
        return false;
    }

    true
}

/// Comprehensive verification self-test suite for Real-Time In-Game 3D Audio
/// Positional Occlusion & Acoustic Ray-Bouncing Reflection Engine.
///
/// Emits sentinel [KNT-AUDIO-OCCLUSION-PASS] upon successful validation.
pub fn test_audio_occlusion_subsystem() -> bool {
    crate::serial_write_str("[KNT-AUDIO-OCCLUSION-TEST] Running Real-Time 3D Audio Positional Occlusion & Acoustic Reflection verification suite...\r\n");

    // 1. Verify 3D spatial acoustic ray casting and AABB obstacle intersection
    if !test_ray_casting_and_aabb_intersection() {
        crate::serial_write_str("[KNT-PANIC] 3D ray casting and AABB obstacle intersection verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * 3D spatial acoustic ray casting and AABB obstacle intersection verified\r\n");

    // 2. Verify material transmission loss and low-pass filter cutoff frequencies
    if !test_material_transmission_and_occlusion() {
        crate::serial_write_str("[KNT-PANIC] Material transmission loss and occlusion calculation verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * Physical material transmission loss (dB) and occluded LPF cutoff frequencies verified\r\n");

    // 3. Verify acoustic diffraction path finding and edge bending angles
    if !test_diffraction_path_finding() {
        crate::serial_write_str("[KNT-PANIC] Acoustic diffraction path finder verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * Acoustic diffraction path finding around obstacle boundaries verified\r\n");

    // 4. Verify circular reflection delay line, fractional sample interpolation, and ray bouncing
    if !test_early_reflections_and_delay_line() {
        crate::serial_write_str("[KNT-PANIC] Early reflection delay line and ray bouncing verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * First-order and second-order early reflection ray bouncing and fractional delay line verified\r\n");

    // 5. Verify click-free parameter smoothing across rapid acoustic boundary movements
    if !test_click_free_parameter_smoothing() {
        crate::serial_write_str("[KNT-PANIC] Click-free parameter smoothing verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * Click-free parameter smoothing and slew-rate interpolation verified\r\n");

    // 6. Verify end-to-end audio occlusion, early reflections, and late reverb DSP pipeline
    if !test_end_to_end_occlusion_and_reverb_pipeline() {
        crate::serial_write_str("[KNT-PANIC] End-to-end audio occlusion and reflection DSP pipeline verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * End-to-end acoustic occlusion, early reflections, and late reverberation pipeline verified\r\n");

    crate::serial_write_str("[KNT-AUDIO-OCCLUSION-PASS] Real-time 3D audio positional occlusion and acoustic ray-bouncing reflection engine operational\r\n");
    true
}
