//! Real-Time 3D Positional Audio Spatializer & Emitter Mathematics.
//!
//! Provides 3D audio listener coordinate frame transformation, directional sound cone radiation,
//! inverse-square acoustic distance attenuation, atmospheric air absorption, Doppler pitch shifting,
//! and complete binaural stereo synthesis for headphone gaming.

use crate::graphics::types::{sqrt_f32, Vec3, PI, HALF_PI};
use super::hrtf::{
    calculate_ild_parameters, calculate_itd_samples, EarDelayLine, FirConvolver, HrtfDataset,
    IldFilter, SPEED_OF_SOUND,
};

// -----------------------------------------------------------------------------
// Pure no_std Trigonometric & Math Helpers
// -----------------------------------------------------------------------------

/// Computes arctangent of `x` in radians using a high-precision polynomial.
pub fn atan_f32(x: f32) -> f32 {
    let abs_x = if x < 0.0 { -x } else { x };
    let res = if abs_x <= 1.0 {
        let x2 = abs_x * abs_x;
        let x4 = x2 * x2;
        let x6 = x4 * x2;
        let x8 = x4 * x4;
        abs_x * (0.999866 - 0.3302995 * x2 + 0.180141 * x4 - 0.085133 * x6 + 0.0208351 * x8)
    } else {
        let inv_x = 1.0 / abs_x;
        let x2 = inv_x * inv_x;
        let x4 = x2 * x2;
        let x6 = x4 * x2;
        let x8 = x4 * x4;
        let inner = inv_x * (0.999866 - 0.3302995 * x2 + 0.180141 * x4 - 0.085133 * x6 + 0.0208351 * x8);
        HALF_PI - inner
    };

    if x < 0.0 {
        -res
    } else {
        res
    }
}

/// Computes the four-quadrant arctangent atan2(y, x) in radians within [-PI, PI].
pub fn atan2_f32(y: f32, x: f32) -> f32 {
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }
    if x > 0.0 {
        atan_f32(y / x)
    } else if x < 0.0 {
        if y >= 0.0 {
            atan_f32(y / x) + PI
        } else {
            atan_f32(y / x) - PI
        }
    } else {
        // x == 0.0
        if y > 0.0 {
            HALF_PI
        } else {
            -HALF_PI
        }
    }
}

// -----------------------------------------------------------------------------
// 3D Audio Listener
// -----------------------------------------------------------------------------

/// 3D Audio Listener representing the player's head and ears in virtual 3D space.
#[derive(Clone, Copy, Debug)]
pub struct AudioListener3D {
    pub position: Vec3,
    pub forward: Vec3,
    pub up: Vec3,
    pub right: Vec3,
    pub velocity: Vec3,
}

impl AudioListener3D {
    /// Creates a listener at the world origin facing along the positive Z axis with Y up.
    pub fn new() -> Self {
        let forward = Vec3::new(0.0, 0.0, 1.0);
        let up = Vec3::new(0.0, 1.0, 0.0);
        let right = Vec3::new(1.0, 0.0, 0.0);
        Self {
            position: Vec3::zero(),
            forward,
            up,
            right,
            velocity: Vec3::zero(),
        }
    }

    /// Sets the listener position in 3D world space.
    pub fn set_position(&mut self, pos: Vec3) {
        self.position = pos;
    }

    /// Sets the listener velocity vector in m/s (for Doppler calculations).
    pub fn set_velocity(&mut self, vel: Vec3) {
        self.velocity = vel;
    }

    /// Updates listener orientation vectors (forward heading and upward cranial axis).
    pub fn set_orientation(&mut self, forward: Vec3, up: Vec3) {
        self.forward = forward.normalize();
        let up_norm = up.normalize();
        // Right vector = up x forward (in right-handed system with +Z forward, +Y up, +X right)
        self.right = up_norm.cross(self.forward).normalize();
        // Ensure orthonormal up vector = forward x right
        self.up = self.forward.cross(self.right).normalize();
    }

    /// Transforms a 3D world-space coordinate into the listener's local cranial frame:
    /// - X: Right (+X = right ear, -X = left ear)
    /// - Y: Up (+Y = overhead, -Y = below)
    /// - Z: Forward (+Z = directly in front, -Z = behind)
    pub fn world_to_local(&self, world_pos: Vec3) -> Vec3 {
        let delta = world_pos.sub(self.position);
        Vec3::new(
            delta.dot(self.right),
            delta.dot(self.up),
            delta.dot(self.forward),
        )
    }

    /// Computes relative azimuth, elevation, and distance from listener to target.
    /// - `azimuth`: Angle in radians [-PI, +PI]. (0 = front, +PI/2 = right, -PI/2 = left, +/-PI = rear).
    /// - `elevation`: Angle in radians [-PI/2, +PI/2]. (0 = horizontal plane, +PI/2 = zenith, -PI/2 = nadir).
    /// - `distance`: Distance in meters.
    pub fn calculate_relative_direction(&self, world_pos: Vec3) -> (f32, f32, f32) {
        let local = self.world_to_local(world_pos);
        let dist = local.length();

        if dist < 1e-6 {
            return (0.0, 0.0, 0.0);
        }

        // Azimuth in X-Z plane: atan2(x, z)
        let azimuth = atan2_f32(local.x, local.z);

        // Elevation relative to horizontal plane: atan2(y, sqrt(x^2 + z^2))
        let horiz_dist = sqrt_f32(local.x * local.x + local.z * local.z);
        let elevation = atan2_f32(local.y, horiz_dist);

        (azimuth, elevation, dist)
    }
}

// -----------------------------------------------------------------------------
// 3D Audio Emitter
// -----------------------------------------------------------------------------

/// Distance attenuation model applied to 3D emitters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DistanceModel {
    /// Inverse distance (physical 1/r acoustic pressure decay with rolloff factor).
    InverseDistance,
    /// Linear distance clamp between min and max distance.
    LinearDistance,
}

/// 3D Audio Emitter representing a localized spatial sound source in 3D world space.
#[derive(Clone, Copy, Debug)]
pub struct AudioEmitter3D {
    pub id: u32,
    pub position: Vec3,
    pub velocity: Vec3,
    pub direction: Vec3,
    pub min_distance: f32,
    pub max_distance: f32,
    pub rolloff_factor: f32,
    pub inner_cone_angle: f32,
    pub outer_cone_angle: f32,
    pub outer_cone_gain: f32,
    pub volume: f32,
    pub distance_model: DistanceModel,
}

impl AudioEmitter3D {
    /// Constructs a default omnidirectional 3D sound emitter.
    pub fn new(id: u32) -> Self {
        Self {
            id,
            position: Vec3::zero(),
            velocity: Vec3::zero(),
            direction: Vec3::new(0.0, 0.0, 1.0),
            min_distance: 1.0,
            max_distance: 100.0,
            rolloff_factor: 1.0,
            inner_cone_angle: 2.0 * PI, // Omnidirectional by default
            outer_cone_angle: 2.0 * PI,
            outer_cone_gain: 0.0,
            volume: 1.0,
            distance_model: DistanceModel::InverseDistance,
        }
    }

    pub fn set_position(&mut self, pos: Vec3) {
        self.position = pos;
    }

    pub fn set_velocity(&mut self, vel: Vec3) {
        self.velocity = vel;
    }

    pub fn set_direction(&mut self, dir: Vec3) {
        self.direction = dir.normalize();
    }

    /// Configures the emitter's distance attenuation curve parameters.
    pub fn set_distance_model(&mut self, min_dist: f32, max_dist: f32, rolloff: f32) {
        self.min_distance = if min_dist < 0.1 { 0.1 } else { min_dist };
        self.max_distance = if max_dist <= self.min_distance {
            self.min_distance + 1.0
        } else {
            max_dist
        };
        self.rolloff_factor = if rolloff < 0.0 { 0.0 } else { rolloff };
    }

    /// Configures directional sound cone radiation (e.g. loudspeaker, weapon muzzle).
    pub fn set_cone(&mut self, inner_rad: f32, outer_rad: f32, outer_gain: f32) {
        self.inner_cone_angle = inner_rad.clamp(0.0, 2.0 * PI);
        self.outer_cone_angle = outer_rad.clamp(self.inner_cone_angle, 2.0 * PI);
        self.outer_cone_gain = outer_gain.clamp(0.0, 1.0);
    }

    /// Evaluates inverse-square sound pressure distance attenuation.
    /// Returns gain multiplier in range [0.0, 1.0].
    pub fn calculate_distance_attenuation(&self, distance: f32) -> f32 {
        if distance <= self.min_distance {
            return 1.0;
        }
        if distance >= self.max_distance {
            return 0.0;
        }

        match self.distance_model {
            DistanceModel::InverseDistance => {
                let d = distance.clamp(self.min_distance, self.max_distance);
                // Inverse distance formula: min / (min + rolloff * (d - min))
                let denom = self.min_distance + self.rolloff_factor * (d - self.min_distance);
                if denom > 1e-6 {
                    self.min_distance / denom
                } else {
                    1.0
                }
            }
            DistanceModel::LinearDistance => {
                let d = distance.clamp(self.min_distance, self.max_distance);
                let range = self.max_distance - self.min_distance;
                1.0 - (self.rolloff_factor * (d - self.min_distance) / range).clamp(0.0, 1.0)
            }
        }
    }

    /// Evaluates directional sound cone attenuation based on listener orientation relative to emitter.
    pub fn calculate_cone_attenuation(&self, emitter_to_listener: Vec3) -> f32 {
        if self.inner_cone_angle >= 2.0 * PI && self.outer_cone_angle >= 2.0 * PI {
            return 1.0;
        }

        let dir = emitter_to_listener.normalize();
        let cos_angle = self.direction.dot(dir).clamp(-1.0, 1.0);

        let angle = if cos_angle >= 1.0 {
            0.0
        } else if cos_angle <= -1.0 {
            PI
        } else {
            let x = cos_angle;
            let sin_approx = sqrt_f32(1.0 - x * x);
            atan2_f32(sin_approx, x)
        };

        let half_inner = self.inner_cone_angle * 0.5;
        let half_outer = self.outer_cone_angle * 0.5;

        if angle <= half_inner {
            1.0
        } else if angle >= half_outer {
            self.outer_cone_gain
        } else {
            let t = (angle - half_inner) / (half_outer - half_inner);
            1.0 - t * (1.0 - self.outer_cone_gain)
        }
    }

    /// Evaluates high-frequency atmospheric air absorption damping factor (1-pole alpha).
    pub fn calculate_air_absorption(&self, distance: f32) -> f32 {
        if distance <= self.min_distance {
            return 1.0;
        }
        let t = ((distance - self.min_distance) / (self.max_distance - self.min_distance)).clamp(0.0, 1.0);
        1.0 - (0.65 * t)
    }
}

// -----------------------------------------------------------------------------
// Doppler Effect Frequency Shift
// -----------------------------------------------------------------------------

/// Computes the acoustic Doppler pitch shift multiplier based on relative velocities:
/// `f_observed = f_source * (c - v_l) / (c - v_s)`
/// Returns a frequency scaling factor in range [0.2, 5.0].
pub fn calculate_doppler_factor(
    source_pos: Vec3,
    source_vel: Vec3,
    listener_pos: Vec3,
    listener_vel: Vec3,
) -> f32 {
    let delta = listener_pos.sub(source_pos);
    let dist = delta.length();
    if dist < 1e-4 {
        return 1.0;
    }

    // Unit vector directed from source to listener
    let u = delta.scale(1.0 / dist);

    // Radial velocity projections
    let v_s = source_vel.dot(u);
    let v_l = listener_vel.dot(u);

    // Clamp velocities to 0.9 * SPEED_OF_SOUND to prevent division by zero or negative pitch
    let max_v = 0.9 * SPEED_OF_SOUND;
    let clamped_vs = v_s.clamp(-max_v, max_v);
    let clamped_vl = v_l.clamp(-max_v, max_v);

    let factor = (SPEED_OF_SOUND - clamped_vl) / (SPEED_OF_SOUND - clamped_vs);
    factor.clamp(0.2, 5.0)
}

/// Circular resampler applying fractional Doppler pitch shifting.
#[derive(Clone, Copy)]
pub struct DopplerResampler {
    pub buffer: [i16; 64],
    pub write_idx: usize,
    pub read_cursor: f32,
}

impl DopplerResampler {
    pub const fn new() -> Self {
        Self {
            buffer: [0; 64],
            write_idx: 0,
            read_cursor: 0.0,
        }
    }

    /// Pushes an incoming PCM sample into the resampler buffer.
    #[inline]
    pub fn push(&mut self, sample: i16) {
        self.buffer[self.write_idx] = sample;
        self.write_idx = (self.write_idx + 1) % 64;
    }

    /// Resamples an input slice into an output slice using Doppler pitch factor with linear interpolation.
    pub fn resample_block(&mut self, input: &[i16], output: &mut [i16], doppler_factor: f32) {
        let factor = doppler_factor.clamp(0.25, 4.0);

        for &sample in input {
            self.push(sample);
        }

        let mut read_pos = 0.0f32;
        let in_len = input.len() as f32;

        for out_sample in output.iter_mut() {
            if read_pos >= in_len {
                *out_sample = 0;
                continue;
            }

            let idx0 = read_pos as usize;
            let frac = read_pos - (idx0 as f32);
            let s0 = input[idx0] as f32;
            let s1 = if idx0 + 1 < input.len() {
                input[idx0 + 1] as f32
            } else {
                s0
            };

            let interp = s0 * (1.0 - frac) + s1 * frac;
            *out_sample = interp.clamp(i16::MIN as f32, i16::MAX as f32) as i16;

            read_pos += factor;
        }
    }
}

// -----------------------------------------------------------------------------
// Binaural Audio Spatializer Voice & Engine
// -----------------------------------------------------------------------------

/// Individual active 3D spatialized voice processing a mono source into binaural stereo.
#[derive(Clone, Copy)]
pub struct BinauralVoice {
    pub emitter: AudioEmitter3D,
    pub left_delay: EarDelayLine,
    pub right_delay: EarDelayLine,
    pub left_ild: IldFilter,
    pub right_ild: IldFilter,
    pub air_filter: IldFilter,
    pub left_convolver: FirConvolver,
    pub right_convolver: FirConvolver,
    pub active: bool,
}

impl BinauralVoice {
    pub const fn new(id: u32) -> Self {
        Self {
            emitter: AudioEmitter3D {
                id,
                position: Vec3::zero(),
                velocity: Vec3::zero(),
                direction: Vec3::new(0.0, 0.0, 1.0),
                min_distance: 1.0,
                max_distance: 100.0,
                rolloff_factor: 1.0,
                inner_cone_angle: 2.0 * PI,
                outer_cone_angle: 2.0 * PI,
                outer_cone_gain: 0.0,
                volume: 1.0,
                distance_model: DistanceModel::InverseDistance,
            },
            left_delay: EarDelayLine::new(),
            right_delay: EarDelayLine::new(),
            left_ild: IldFilter::new(),
            right_ild: IldFilter::new(),
            air_filter: IldFilter::new(),
            left_convolver: FirConvolver::new(),
            right_convolver: FirConvolver::new(),
            active: false,
        }
    }

    /// Processes a single mono audio sample through the 3D acoustic pipeline:
    /// Distance attenuation -> Directional cone -> Air absorption -> ITD delay line -> ILD filter -> HRTF FIR convolution.
    /// Returns `(left_sample, right_sample)`.
    #[inline]
    pub fn process_sample(
        &mut self,
        input: i16,
        listener: &AudioListener3D,
        dataset: &HrtfDataset,
    ) -> (i16, i16) {
        if !self.active || input == 0 {
            return (0, 0);
        }

        // 1. Calculate relative spatial direction & distance
        let (azimuth_rad, _elevation_rad, distance) =
            listener.calculate_relative_direction(self.emitter.position);

        // 2. Distance and directional sound cone attenuation
        let dist_gain = self.emitter.calculate_distance_attenuation(distance);
        let cone_gain = self.emitter.calculate_cone_attenuation(listener.position.sub(self.emitter.position));
        let overall_gain = dist_gain * cone_gain * self.emitter.volume;

        if overall_gain < 1e-4 {
            return (0, 0);
        }

        // 3. Atmospheric air absorption high-frequency filtering
        let air_alpha = self.emitter.calculate_air_absorption(distance);
        let absorbed = self.air_filter.process(input, air_alpha, overall_gain);

        // 4. Interaural Time Difference (ITD) delay line
        self.left_delay.push(absorbed);
        self.right_delay.push(absorbed);

        let (left_delay_samps, right_delay_samps) = calculate_itd_samples(azimuth_rad);
        let l_itd = self.left_delay.read_delayed(left_delay_samps);
        let r_itd = self.right_delay.read_delayed(right_delay_samps);

        // 5. Interaural Level Difference (ILD) head-shadow filtering
        let ((l_alpha, l_ild_gain), (r_alpha, r_ild_gain)) = calculate_ild_parameters(azimuth_rad);
        let l_ild = self.left_ild.process(l_itd, l_alpha, l_ild_gain);
        let r_ild = self.right_ild.process(r_itd, r_alpha, r_ild_gain);

        // 6. Binaural HRIR FIR convolution
        let ir = dataset.lookup(azimuth_rad);
        let l_out = self.left_convolver.process(l_ild, &ir.left_taps);
        let r_out = self.right_convolver.process(r_ild, &ir.right_taps);

        (l_out, r_out)
    }

    /// Resets all internal delay buffers and filters for this voice.
    pub fn reset(&mut self) {
        self.left_delay.clear();
        self.right_delay.clear();
        self.left_ild.reset();
        self.right_ild.reset();
        self.air_filter.reset();
        self.left_convolver.reset();
        self.right_convolver.reset();
    }
}

/// Maximum concurrent 3D spatial audio voices in the kernel mixer.
pub const MAX_SPATIAL_VOICES: usize = 8;

/// Real-time 3D Audio Spatialization Subsystem.
pub struct BinauralSpatializer {
    pub listener: AudioListener3D,
    pub dataset: HrtfDataset,
    pub voices: [BinauralVoice; MAX_SPATIAL_VOICES],
}

impl BinauralSpatializer {
    /// Initializes the spatializer with default listener, precomputed HRTF dataset, and idle voices.
    pub fn new() -> Self {
        let dataset = HrtfDataset::new();
        let mut voices = [BinauralVoice::new(0); MAX_SPATIAL_VOICES];
        for i in 0..MAX_SPATIAL_VOICES {
            voices[i] = BinauralVoice::new(i as u32);
        }

        Self {
            listener: AudioListener3D::new(),
            dataset,
            voices,
        }
    }

    /// Allocates an available spatial voice for the given emitter.
    pub fn allocate_voice(&mut self, emitter: AudioEmitter3D) -> Option<usize> {
        for (idx, voice) in self.voices.iter_mut().enumerate() {
            if !voice.active {
                voice.emitter = emitter;
                voice.active = true;
                voice.reset();
                return Some(idx);
            }
        }
        None
    }

    /// Frees an active spatial voice.
    pub fn free_voice(&mut self, voice_idx: usize) {
        if voice_idx < MAX_SPATIAL_VOICES {
            self.voices[voice_idx].active = false;
            self.voices[voice_idx].reset();
        }
    }

    /// Renders a mono PCM audio buffer through an active spatial voice into stereo (Left, Right) buffers.
    pub fn render_voice(
        &mut self,
        voice_idx: usize,
        input: &[i16],
        output_left: &mut [i16],
        output_right: &mut [i16],
    ) {
        if voice_idx >= MAX_SPATIAL_VOICES || !self.voices[voice_idx].active {
            for s in output_left.iter_mut() {
                *s = 0;
            }
            for s in output_right.iter_mut() {
                *s = 0;
            }
            return;
        }

        let voice = &mut self.voices[voice_idx];
        let count = input.len().min(output_left.len()).min(output_right.len());

        for i in 0..count {
            let (l, r) = voice.process_sample(input[i], &self.listener, &self.dataset);
            output_left[i] = l;
            output_right[i] = r;
        }
    }

    /// Renders multiple active audio voices and mixes them into a unified stereo buffer.
    pub fn mix_spatial_audio(
        &mut self,
        voice_inputs: &[(usize, &[i16])],
        out_stereo: &mut [(i16, i16)],
    ) {
        // Zero output buffer
        for frame in out_stereo.iter_mut() {
            *frame = (0, 0);
        }

        for &(voice_idx, in_buf) in voice_inputs {
            if voice_idx >= MAX_SPATIAL_VOICES || !self.voices[voice_idx].active {
                continue;
            }

            let voice = &mut self.voices[voice_idx];
            let count = in_buf.len().min(out_stereo.len());

            for i in 0..count {
                let (l, r) = voice.process_sample(in_buf[i], &self.listener, &self.dataset);
                let (cur_l, cur_r) = out_stereo[i];
                let mix_l = (cur_l as i32 + l as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                let mix_r = (cur_r as i32 + r as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                out_stereo[i] = (mix_l, mix_r);
            }
        }
    }
}

// -----------------------------------------------------------------------------
// Unit Tests
// -----------------------------------------------------------------------------

/// Unit test verifying 3D audio listener coordinate transformation and emitter distance attenuation.
pub fn test_3d_emitter_positioning_and_attenuation() -> bool {
    // 1. Verify AudioListener3D coordinate frame
    let mut listener = AudioListener3D::new();
    listener.set_position(Vec3::new(10.0, 0.0, 10.0));
    listener.set_orientation(Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 1.0, 0.0));

    // Emitter directly to the RIGHT at (15, 0, 10)
    let (az_r, el_r, dist_r) = listener.calculate_relative_direction(Vec3::new(15.0, 0.0, 10.0));
    if (dist_r - 5.0).abs() > 1e-4 {
        return false;
    }
    if (az_r - HALF_PI).abs() > 1e-3 || el_r.abs() > 1e-3 {
        return false;
    }

    // Emitter directly to the LEFT at (5, 0, 10)
    let (az_l, el_l, dist_l) = listener.calculate_relative_direction(Vec3::new(5.0, 0.0, 10.0));
    if (dist_l - 5.0).abs() > 1e-4 {
        return false;
    }
    if (az_l - (-HALF_PI)).abs() > 1e-3 || el_l.abs() > 1e-3 {
        return false;
    }

    // Emitter directly in FRONT at (10, 0, 25)
    let (az_f, el_f, dist_f) = listener.calculate_relative_direction(Vec3::new(10.0, 0.0, 25.0));
    if (dist_f - 15.0).abs() > 1e-4 || az_f.abs() > 1e-3 || el_f.abs() > 1e-3 {
        return false;
    }

    // Emitter OVERHEAD at (10.0, 10.0, 10.0)
    let (_az_up, el_up, dist_up) = listener.calculate_relative_direction(Vec3::new(10.0, 10.0, 10.0));
    if (dist_up - 10.0).abs() > 1e-4 || (el_up - HALF_PI).abs() > 1e-3 {
        return false;
    }

    // 2. Verify Listener rotation: turn listener 90 degrees to face +X
    listener.set_orientation(Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0));
    let (az_turned, _el_turned, dist_turned) = listener.calculate_relative_direction(Vec3::new(20.0, 0.0, 10.0));
    if (dist_turned - 10.0).abs() > 1e-4 || az_turned.abs() > 1e-3 {
        return false;
    }

    // 3. Verify Inverse-Square Distance Attenuation
    let mut emitter = AudioEmitter3D::new(1);
    emitter.set_distance_model(1.0, 100.0, 1.0);

    if emitter.calculate_distance_attenuation(0.5) != 1.0 {
        return false;
    }
    if emitter.calculate_distance_attenuation(1.0) != 1.0 {
        return false;
    }

    let g2 = emitter.calculate_distance_attenuation(2.0);
    if (g2 - 0.5).abs() > 1e-4 {
        return false;
    }

    let g5 = emitter.calculate_distance_attenuation(5.0);
    if (g5 - 0.2).abs() > 1e-4 {
        return false;
    }

    if emitter.calculate_distance_attenuation(105.0) != 0.0 {
        return false;
    }

    // 4. Verify Directional Sound Cone Attenuation
    let mut cone_emitter = AudioEmitter3D::new(2);
    cone_emitter.set_direction(Vec3::new(0.0, 0.0, 1.0));
    cone_emitter.set_cone(PI / 4.0, PI / 2.0, 0.1);

    let inside_gain = cone_emitter.calculate_cone_attenuation(Vec3::new(0.0, 0.0, 1.0));
    if (inside_gain - 1.0).abs() > 1e-4 {
        return false;
    }

    let outside_gain = cone_emitter.calculate_cone_attenuation(Vec3::new(0.0, 0.0, -1.0));
    if (outside_gain - 0.1).abs() > 1e-4 {
        return false;
    }

    // 5. Verify Atmospheric Air Absorption
    let close_abs = emitter.calculate_air_absorption(1.0);
    let far_abs = emitter.calculate_air_absorption(100.0);
    if close_abs != 1.0 || (far_abs - 0.35).abs() > 1e-4 {
        return false;
    }

    true
}

/// Unit test verifying Doppler effect frequency shift and binaural stereo synthesis.
pub fn test_doppler_and_binaural_convolver() -> bool {
    // 1. Verify Doppler frequency shift factor calculation
    let src_pos = Vec3::new(0.0, 0.0, 0.0);
    let lis_pos = Vec3::new(0.0, 0.0, 100.0);

    // Both stationary: factor = 1.0
    let f_stat = calculate_doppler_factor(src_pos, Vec3::zero(), lis_pos, Vec3::zero());
    if (f_stat - 1.0).abs() > 1e-4 {
        return false;
    }

    // Source moving towards listener at 34.3 m/s (0.1 Mach, c = 343 m/s)
    // f' = c / (c - 34.3) = 343 / 308.7 = 1.1111
    let src_vel_towards = Vec3::new(0.0, 0.0, 34.3);
    let f_towards = calculate_doppler_factor(src_pos, src_vel_towards, lis_pos, Vec3::zero());
    if (f_towards - (343.0 / 308.7)).abs() > 1e-3 {
        return false;
    }

    // Source moving away from listener at 34.3 m/s
    // f' = c / (c + 34.3) = 343 / 377.3 = 0.9090
    let src_vel_away = Vec3::new(0.0, 0.0, -34.3);
    let f_away = calculate_doppler_factor(src_pos, src_vel_away, lis_pos, Vec3::zero());
    if (f_away - (343.0 / 377.3)).abs() > 1e-3 {
        return false;
    }

    // 2. Verify DopplerResampler block processing
    let mut resampler = DopplerResampler::new();
    let mut input = [0i16; 32];
    for i in 0..32 {
        input[i] = (i * 200) as i16;
    }
    let mut output = [0i16; 32];
    resampler.resample_block(&input, &mut output, 1.0);
    // At factor 1.0, output should match input closely
    if output[0] != input[0] || output[10] != input[10] {
        return false;
    }

    // 3. Verify BinauralVoice 3D spatialization: Right vs Left ear acoustic panning
    let mut spatializer = BinauralSpatializer::new();
    let mut emitter_right = AudioEmitter3D::new(10);
    emitter_right.set_position(Vec3::new(5.0, 0.0, 0.0)); // 5m to the RIGHT (+X)
    emitter_right.set_distance_model(1.0, 50.0, 1.0);

    let voice_right_idx = spatializer.allocate_voice(emitter_right).expect("Voice alloc failed");

    // Feed a series of pulses into the right voice
    let pulse_in = [10_000i16; 16];
    let mut out_l = [0i16; 16];
    let mut out_r = [0i16; 16];
    spatializer.render_voice(voice_right_idx, &pulse_in, &mut out_l, &mut out_r);

    // Sum energy across frames
    let mut energy_l = 0i64;
    let mut energy_r = 0i64;
    for i in 0..16 {
        energy_l += out_l[i].abs() as i64;
        energy_r += out_r[i].abs() as i64;
    }

    // Right ear must be louder than left ear due to head shadow (ILD) and distance
    if energy_r <= energy_l || energy_r == 0 {
        return false;
    }

    // 4. Verify Left voice symmetry
    let mut emitter_left = AudioEmitter3D::new(11);
    emitter_left.set_position(Vec3::new(-5.0, 0.0, 0.0)); // 5m to the LEFT (-X)
    emitter_left.set_distance_model(1.0, 50.0, 1.0);

    let voice_left_idx = spatializer.allocate_voice(emitter_left).expect("Voice alloc 2 failed");
    let mut out_ll = [0i16; 16];
    let mut out_lr = [0i16; 16];
    spatializer.render_voice(voice_left_idx, &pulse_in, &mut out_ll, &mut out_lr);

    let mut energy_ll = 0i64;
    let mut energy_lr = 0i64;
    for i in 0..16 {
        energy_ll += out_ll[i].abs() as i64;
        energy_lr += out_lr[i].abs() as i64;
    }

    // Left ear must be louder than right ear
    if energy_ll <= energy_lr || energy_ll == 0 {
        return false;
    }

    // 5. Verify Multi-Voice Spatial Mixing
    let mut mix_out = [(0i16, 0i16); 16];
    spatializer.mix_spatial_audio(
        &[(voice_right_idx, &pulse_in), (voice_left_idx, &pulse_in)],
        &mut mix_out,
    );
    // Both left and right channels in mixed output must have energy
    let (first_l, first_r) = mix_out[4];
    if first_l == 0 && first_r == 0 {
        return false;
    }

    true
}
