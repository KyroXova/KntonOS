//! Real-Time 3D Positional Audio Spatializer & Emitter Mathematics.
//!
//! Provides 3D audio listener coordinate frame transformation, directional sound cone radiation,
//! inverse-square acoustic distance attenuation, and high-frequency atmospheric air absorption.

use crate::graphics::types::{sqrt_f32, Vec3, PI, HALF_PI};

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
        // Right vector = forward x up
        self.right = self.forward.cross(up_norm).normalize();
        // Ensure orthonormal up vector = right x forward
        self.up = self.right.cross(self.forward).normalize();
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
        // If omnidirectional, no cone attenuation
        if self.inner_cone_angle >= 2.0 * PI && self.outer_cone_angle >= 2.0 * PI {
            return 1.0;
        }

        let dir = emitter_to_listener.normalize();
        // Dot product between emitter's forward radiation axis and direction to listener
        let cos_angle = self.direction.dot(dir).clamp(-1.0, 1.0);
        // Angle in radians between emitter forward axis and listener
        // Using Taylor approximation or acos helper:
        let angle = if cos_angle >= 1.0 {
            0.0
        } else if cos_angle <= -1.0 {
            PI
        } else {
            // acos(x) = HALF_PI - asin(x)
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
            // Linear transition between inner and outer cone
            let t = (angle - half_inner) / (half_outer - half_inner);
            1.0 - t * (1.0 - self.outer_cone_gain)
        }
    }

    /// Evaluates high-frequency atmospheric air absorption damping factor (1-pole alpha).
    /// As sound travels through air over tens of meters, higher frequencies attenuate faster.
    pub fn calculate_air_absorption(&self, distance: f32) -> f32 {
        if distance <= self.min_distance {
            return 1.0;
        }
        let t = ((distance - self.min_distance) / (self.max_distance - self.min_distance)).clamp(0.0, 1.0);
        // Alpha drops from 1.0 (crisp high end at close range) to 0.35 (muffled/damped at max range)
        1.0 - (0.65 * t)
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
    // Azimuth must be +PI/2 (+90 degrees right)
    if (az_r - HALF_PI).abs() > 1e-3 || el_r.abs() > 1e-3 {
        return false;
    }

    // Emitter directly to the LEFT at (5, 0, 10)
    let (az_l, el_l, dist_l) = listener.calculate_relative_direction(Vec3::new(5.0, 0.0, 10.0));
    if (dist_l - 5.0).abs() > 1e-4 {
        return false;
    }
    // Azimuth must be -PI/2 (-90 degrees left)
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
    // Emitter at (20, 0, 10) is now directly in front of listener!
    let (az_turned, _el_turned, dist_turned) = listener.calculate_relative_direction(Vec3::new(20.0, 0.0, 10.0));
    if (dist_turned - 10.0).abs() > 1e-4 || az_turned.abs() > 1e-3 {
        return false;
    }

    // 3. Verify Inverse-Square Distance Attenuation
    let mut emitter = AudioEmitter3D::new(1);
    emitter.set_distance_model(1.0, 100.0, 1.0);

    // Distance <= min_distance: unity gain (1.0)
    if emitter.calculate_distance_attenuation(0.5) != 1.0 {
        return false;
    }
    if emitter.calculate_distance_attenuation(1.0) != 1.0 {
        return false;
    }

    // Distance = 2.0 (d = 2 * min_dist): gain = 1 / (1 + 1*(2-1)) = 0.5
    let g2 = emitter.calculate_distance_attenuation(2.0);
    if (g2 - 0.5).abs() > 1e-4 {
        return false;
    }

    // Distance = 5.0 (d = 5 * min_dist): gain = 1 / (1 + 1*(5-1)) = 0.2
    let g5 = emitter.calculate_distance_attenuation(5.0);
    if (g5 - 0.2).abs() > 1e-4 {
        return false;
    }

    // Distance >= max_distance: gain = 0.0
    if emitter.calculate_distance_attenuation(105.0) != 0.0 {
        return false;
    }

    // 4. Verify Directional Sound Cone Attenuation
    let mut cone_emitter = AudioEmitter3D::new(2);
    cone_emitter.set_direction(Vec3::new(0.0, 0.0, 1.0)); // Facing forward
    cone_emitter.set_cone(PI / 4.0, PI / 2.0, 0.1); // Inner 45 deg, outer 90 deg, outer gain 0.1

    // Listener inside inner cone: gain 1.0
    let inside_gain = cone_emitter.calculate_cone_attenuation(Vec3::new(0.0, 0.0, 1.0));
    if (inside_gain - 1.0).abs() > 1e-4 {
        return false;
    }

    // Listener outside outer cone: outer gain 0.1
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
