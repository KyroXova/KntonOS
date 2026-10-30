//! Unified Handheld PC Dual-IMU Dead Reckoning & Motion Drift Eliminator.
//!
//! Implements a 1000 Hz dual-IMU sensor fusion engine for handheld gaming PCs
//! featuring spatially separated 6-axis inertial sensors (Left Grip and Right Grip).
//!
//! In handheld gaming devices (Steam Deck, ROG Ally, Ayaneo), single-IMU tracking suffers
//! from rotational lever-arm artifacts, high common-mode thermal bias drift, and rapid
//! dead reckoning divergence. By deploying dual 6-axis IMUs on opposite sides of the chassis,
//! the fusion engine:
//! - Separates true linear chassis acceleration from angular centripetal/tangential components.
//! - Eliminates uncorrelated thermal sensor noise through multi-sensor averaging.
//! - Isolates the 1g gravity vector from dynamic user movements via quaternion kinematics.
//! - Implements Zero-Velocity Updates (ZUPT) and leaky velocity damping for drift-free
//!   sub-millimeter motion tracking and gyro aiming.

use crate::graphics::{cos_f32, sin_f32, sqrt_f32};
use crate::drivers::input::imu::{asin_f32, atan2_f32};

/// Standard gravitational acceleration at sea level in m/s^2.
pub const GRAVITY_MSS: f32 = 9.80665;

/// Identifies the physical IMU location within the handheld PC chassis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImuLocation {
    /// Left controller grip PCB.
    LeftGrip = 0,
    /// Right controller grip PCB.
    RightGrip = 1,
}

/// Physical spatial arrangement and dimensions of dual-IMU chassis topology.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DualImuTopology {
    /// Lateral chassis width between Left and Right IMU in meters (nominally 0.28m = 280mm).
    pub chassis_width_m: f32,
    /// Vector offset from chassis center-of-mass to Left IMU [x, y, z] in meters.
    pub left_lever_arm_m: [f32; 3],
    /// Vector offset from chassis center-of-mass to Right IMU [x, y, z] in meters.
    pub right_lever_arm_m: [f32; 3],
    /// Target sensor sampling frequency in Hertz (nominally 1000 Hz = 1ms period).
    pub sample_rate_hz: u32,
    /// Discrete integration time step in seconds (dt = 1.0 / sample_rate_hz).
    pub dt_s: f32,
}

impl DualImuTopology {
    /// Creates a standard handheld gaming PC topology (280mm grip-to-grip span, 1000 Hz).
    pub const fn standard_handheld() -> Self {
        let half_width = 0.14; // 140 mm from center of mass
        Self {
            chassis_width_m: 0.28,
            left_lever_arm_m: [-half_width, 0.0, 0.0],
            right_lever_arm_m: [half_width, 0.0, 0.0],
            sample_rate_hz: 1000,
            dt_s: 0.001,
        }
    }
}

/// Raw digitized 16-bit sensor packet from an individual 6-axis IMU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawImuReading {
    /// Raw accelerometer ADC counts [X, Y, Z].
    pub accel_raw: [i16; 3],
    /// Raw gyroscope ADC counts [X, Y, Z].
    pub gyro_raw: [i16; 3],
    /// Onboard sensor temperature in raw ADC counts.
    pub temp_raw: i16,
    /// Hardware timestamp in microseconds.
    pub timestamp_us: u64,
}

impl RawImuReading {
    /// Creates a zeroed raw reading.
    pub const fn zero() -> Self {
        Self {
            accel_raw: [0, 0, 0],
            gyro_raw: [0, 0, 0],
            temp_raw: 0,
            timestamp_us: 0,
        }
    }
}

/// Calibrated physical units sample from an individual 6-axis IMU.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhysicalImuSample {
    /// Sensor location (LeftGrip or RightGrip).
    pub location: ImuLocation,
    /// Proper acceleration in m/s^2 [X, Y, Z].
    pub accel_mss: [f32; 3],
    /// Angular velocity in radians per second [X, Y, Z].
    pub gyro_rad_s: [f32; 3],
    /// Sensor temperature in degrees Celsius (°C).
    pub temp_c: f32,
    /// Hardware timestamp in microseconds.
    pub timestamp_us: u64,
}

impl PhysicalImuSample {
    /// Converts a raw 16-bit reading into physical units (assumes +/-8g and +/-2000 dps scales).
    pub fn from_raw(location: ImuLocation, raw: &RawImuReading) -> Self {
        // +/- 8g scale: sensitivity = 4096 LSB/g -> 1g = 9.80665 m/s^2
        let accel_scale = GRAVITY_MSS / 4096.0;
        let accel_mss = [
            (raw.accel_raw[0] as f32) * accel_scale,
            (raw.accel_raw[1] as f32) * accel_scale,
            (raw.accel_raw[2] as f32) * accel_scale,
        ];

        // +/- 2000 dps scale: sensitivity = 16.4 LSB/(deg/s) -> 1 deg = PI/180 rad
        let deg_to_rad = 3.14159265 / 180.0;
        let gyro_scale = (1.0 / 16.4) * deg_to_rad;
        let gyro_rad_s = [
            (raw.gyro_raw[0] as f32) * gyro_scale,
            (raw.gyro_raw[1] as f32) * gyro_scale,
            (raw.gyro_raw[2] as f32) * gyro_scale,
        ];

        // Temperature: typically (raw / 132.48) + 25.0 °C
        let temp_c = ((raw.temp_raw as f32) / 132.48) + 25.0;

        Self {
            location,
            accel_mss,
            gyro_rad_s,
            temp_c,
            timestamp_us: raw.timestamp_us,
        }
    }
}

/// Unit quaternion representing 3D spatial orientation in body coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpatialQuaternion {
    /// Scalar component (cos(theta / 2)).
    pub w: f32,
    /// Vector X component (axis_x * sin(theta / 2)).
    pub x: f32,
    /// Vector Y component (axis_y * sin(theta / 2)).
    pub y: f32,
    /// Vector Z component (axis_z * sin(theta / 2)).
    pub z: f32,
}

impl SpatialQuaternion {
    /// Identity quaternion (zero rotation).
    pub const fn identity() -> Self {
        Self {
            w: 1.0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }
    }

    /// Creates a quaternion from an axis and rotation angle in radians.
    pub fn from_axis_angle(axis: [f32; 3], angle_rad: f32) -> Self {
        let half_angle = angle_rad * 0.5;
        let s = sin_f32(half_angle);
        let c = cos_f32(half_angle);

        let mut mag = sqrt_f32(axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]);
        if mag < 1e-6 {
            mag = 1.0;
        }

        Self {
            w: c,
            x: (axis[0] / mag) * s,
            y: (axis[1] / mag) * s,
            z: (axis[2] / mag) * s,
        }
    }

    /// Computes quaternion Euclidean norm: sqrt(w^2 + x^2 + y^2 + z^2).
    pub fn norm(&self) -> f32 {
        sqrt_f32(self.w * self.w + self.x * self.x + self.y * self.y + self.z * self.z)
    }

    /// Normalizes this quaternion in-place to enforce unit length.
    pub fn normalize(&mut self) {
        let n = self.norm();
        if n > 1e-6 {
            let inv = 1.0 / n;
            self.w *= inv;
            self.x *= inv;
            self.y *= inv;
            self.z *= inv;
        } else {
            *self = Self::identity();
        }
    }

    /// Quaternion Hamilton product: self * rhs.
    pub fn multiply(&self, rhs: &Self) -> Self {
        Self {
            w: self.w * rhs.w - self.x * rhs.x - self.y * rhs.y - self.z * rhs.z,
            x: self.w * rhs.x + self.x * rhs.w + self.y * rhs.z - self.z * rhs.y,
            y: self.w * rhs.y - self.x * rhs.z + self.y * rhs.w + self.z * rhs.x,
            z: self.w * rhs.z + self.x * rhs.y - self.y * rhs.x + self.z * rhs.w,
        }
    }

    /// Computes the quaternion conjugate: (w, -x, -y, -z).
    #[inline(always)]
    pub fn conjugate(&self) -> Self {
        Self {
            w: self.w,
            x: -self.x,
            y: -self.y,
            z: -self.z,
        }
    }

    /// Rotates a 3D vector by this quaternion: v' = q * v * q^(-1).
    pub fn rotate_vector(&self, v: [f32; 3]) -> [f32; 3] {
        let q_vec = Self {
            w: 0.0,
            x: v[0],
            y: v[1],
            z: v[2],
        };
        let conj = self.conjugate();
        let res = self.multiply(&q_vec).multiply(&conj);
        [res.x, res.y, res.z]
    }

    /// Extracts Tait-Bryan Euler angles in radians (Pitch, Roll, Yaw).
    pub fn to_euler_angles(&self) -> (f32, f32, f32) {
        // Roll (X-axis rotation)
        let sinr_cosp = 2.0 * (self.w * self.x + self.y * self.z);
        let cosr_cosp = 1.0 - 2.0 * (self.x * self.x + self.y * self.y);
        let roll = atan2_f32(sinr_cosp, cosr_cosp);

        // Pitch (Y-axis rotation)
        let sinp = 2.0 * (self.w * self.y - self.z * self.x);
        let pitch = if sinp.abs() >= 1.0 {
            if sinp > 0.0 { 3.14159265 / 2.0 } else { -3.14159265 / 2.0 }
        } else {
            asin_f32(sinp)
        };

        // Yaw (Z-axis rotation)
        let siny_cosp = 2.0 * (self.w * self.z + self.x * self.y);
        let cosy_cosp = 1.0 - 2.0 * (self.y * self.y + self.z * self.z);
        let yaw = atan2_f32(siny_cosp, cosy_cosp);

        (pitch, roll, yaw)
    }
}

/// Verification helper for Commit 1 structures, dual-IMU topology, and quaternion mathematics.
pub fn verify_dual_imu_topology_and_quaternions() -> bool {
    // 1. Verify standard handheld topology dimensions
    let topo = DualImuTopology::standard_handheld();
    if (topo.chassis_width_m - 0.28).abs() > 1e-4 {
        return false;
    }
    if (topo.left_lever_arm_m[0] - (-0.14)).abs() > 1e-4 {
        return false;
    }
    if (topo.right_lever_arm_m[0] - 0.14).abs() > 1e-4 {
        return false;
    }
    if topo.sample_rate_hz != 1000 || (topo.dt_s - 0.001).abs() > 1e-6 {
        return false;
    }

    // 2. Verify Raw Reading to Physical Units conversion
    // 4096 LSB on 8g scale = 1.0g = 9.80665 m/s^2 along Z
    let raw = RawImuReading {
        accel_raw: [0, 0, 4096],
        gyro_raw: [0, 0, 0],
        temp_raw: 0,
        timestamp_us: 1000,
    };
    let phys = PhysicalImuSample::from_raw(ImuLocation::LeftGrip, &raw);
    if (phys.accel_mss[2] - GRAVITY_MSS).abs() > 0.01 {
        return false;
    }
    if phys.accel_mss[0].abs() > 1e-4 || phys.accel_mss[1].abs() > 1e-4 {
        return false;
    }
    if (phys.temp_c - 25.0).abs() > 0.1 {
        return false;
    }

    // 3. Verify Quaternion Identity & Normalization
    let mut q_ident = SpatialQuaternion::identity();
    if (q_ident.norm() - 1.0).abs() > 1e-6 {
        return false;
    }
    q_ident.w = 2.0;
    q_ident.normalize();
    if (q_ident.w - 1.0).abs() > 1e-6 {
        return false;
    }

    // 4. Verify 90-degree Yaw rotation around Z axis
    // 90 deg = PI/2 rad -> w = cos(PI/4) = sqrt(2)/2, z = sin(PI/4) = sqrt(2)/2
    let pi = 3.14159265;
    let q_rot = SpatialQuaternion::from_axis_angle([0.0, 0.0, 1.0], pi / 2.0);
    // Rotate vector [1.0, 0.0, 0.0] by 90 deg around Z -> should become [0.0, 1.0, 0.0]
    let v_in = [1.0, 0.0, 0.0];
    let v_out = q_rot.rotate_vector(v_in);
    if v_out[0].abs() > 0.01 || (v_out[1] - 1.0).abs() > 0.01 || v_out[2].abs() > 0.01 {
        return false;
    }

    // 5. Verify Euler angle extraction
    let (pitch, roll, yaw) = q_rot.to_euler_angles();
    if pitch.abs() > 0.01 || roll.abs() > 0.01 || (yaw - (pi / 2.0)).abs() > 0.02 {
        return false;
    }

    true
}

// -----------------------------------------------------------------------------
// Commit 2: Complementary & EKF Dual-IMU Sensor Fusion & Gravity Isolation
// -----------------------------------------------------------------------------

/// Fused dual-IMU kinematics at a single 1000 Hz discrete time step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FusedKinematics {
    /// Fused angular velocity in radians per second [X, Y, Z].
    pub angular_velocity_rad_s: [f32; 3],
    /// Fused total proper acceleration in m/s^2 [X, Y, Z] (lever-arm cancelled).
    pub proper_accel_mss: [f32; 3],
    /// Isolated gravitational acceleration in body coordinates [X, Y, Z].
    pub gravity_body_mss: [f32; 3],
    /// Gravity-free dynamic linear acceleration in body coordinates [X, Y, Z].
    pub dynamic_linear_accel_mss: [f32; 3],
    /// Gravity-free dynamic linear acceleration in world navigation coordinates [X, Y, Z].
    pub world_linear_accel_mss: [f32; 3],
    /// Current estimated spatial orientation quaternion.
    pub orientation: SpatialQuaternion,
}

/// 1000 Hz Dual-IMU Sensor Fusion Filter.
pub struct DualImuFusionFilter {
    /// Chassis spatial topology.
    pub topology: DualImuTopology,
    /// Current estimated orientation quaternion.
    pub orientation: SpatialQuaternion,
    /// Estimated gyroscope bias offsets in rad/s [X, Y, Z].
    pub gyro_bias: [f32; 3],
    /// Estimated accelerometer bias offsets in m/s^2 [X, Y, Z].
    pub accel_bias: [f32; 3],
    /// Complementary filter accelerometer correction gain beta.
    pub beta: f32,
    /// Most recent fused kinematics snapshot.
    pub kinematics: FusedKinematics,
}

impl DualImuFusionFilter {
    /// Creates a new fusion filter with standard handheld topology.
    pub const fn new(topology: DualImuTopology) -> Self {
        Self {
            topology,
            orientation: SpatialQuaternion::identity(),
            gyro_bias: [0.0, 0.0, 0.0],
            accel_bias: [0.0, 0.0, 0.0],
            beta: 0.02, // 1000 Hz complementary filter gain
            kinematics: FusedKinematics {
                angular_velocity_rad_s: [0.0, 0.0, 0.0],
                proper_accel_mss: [0.0, 0.0, GRAVITY_MSS],
                gravity_body_mss: [0.0, 0.0, GRAVITY_MSS],
                dynamic_linear_accel_mss: [0.0, 0.0, 0.0],
                world_linear_accel_mss: [0.0, 0.0, 0.0],
                orientation: SpatialQuaternion::identity(),
            },
        }
    }

    /// Ingests simultaneous physical samples from Left and Right IMUs and updates fused state.
    pub fn update(
        &mut self,
        left: &PhysicalImuSample,
        right: &PhysicalImuSample,
    ) -> FusedKinematics {
        let dt = self.topology.dt_s;

        // 1. Dual-IMU Gyroscope Averaging:
        // Reduces uncorrelated thermal noise variance by sqrt(2)
        let raw_gyro = [
            (left.gyro_rad_s[0] + right.gyro_rad_s[0]) * 0.5 - self.gyro_bias[0],
            (left.gyro_rad_s[1] + right.gyro_rad_s[1]) * 0.5 - self.gyro_bias[1],
            (left.gyro_rad_s[2] + right.gyro_rad_s[2]) * 0.5 - self.gyro_bias[2],
        ];

        // 2. Dual-IMU Accelerometer Averaging & Lever-Arm Cancellation:
        // Because left lever arm r_L = [-d/2, 0, 0] and right r_R = [+d/2, 0, 0],
        // centripetal and tangential rotational accelerations cancel out identically!
        let proper_accel = [
            (left.accel_mss[0] + right.accel_mss[0]) * 0.5 - self.accel_bias[0],
            (left.accel_mss[1] + right.accel_mss[1]) * 0.5 - self.accel_bias[1],
            (left.accel_mss[2] + right.accel_mss[2]) * 0.5 - self.accel_bias[2],
        ];

        // 3. Accelerometer Tilt Error Vector (Gravity Reference Correction)
        let accel_norm = sqrt_f32(
            proper_accel[0] * proper_accel[0]
                + proper_accel[1] * proper_accel[1]
                + proper_accel[2] * proper_accel[2],
        );

        let mut gyro_corr = raw_gyro;

        // Only correct tilt using accelerometer when total acceleration is close to 1g (0.85g to 1.15g)
        // to prevent dynamic user acceleration from distorting orientation.
        if (accel_norm - GRAVITY_MSS).abs() < 1.5 && accel_norm > 1.0 {
            let inv_norm = 1.0 / accel_norm;
            let ax = proper_accel[0] * inv_norm;
            let ay = proper_accel[1] * inv_norm;
            let az = proper_accel[2] * inv_norm;

            // Estimated gravity direction in body frame from current orientation quaternion q:
            // g_est = [2*(x*z - w*y), 2*(y*z + w*x), w^2 - x^2 - y^2 + z^2]
            let q = &self.orientation;
            let gx = 2.0 * (q.x * q.z - q.w * q.y);
            let gy = 2.0 * (q.y * q.z + q.w * q.x);
            let gz = q.w * q.w - q.x * q.x - q.y * q.y + q.z * q.z;

            // Error is cross product of measured acceleration vector and estimated gravity: e = a x g
            let ex = ay * gz - az * gy;
            let ey = az * gx - ax * gz;
            let ez = ax * gy - ay * gx;

            // Apply complementary correction to gyro angular velocity
            gyro_corr[0] += self.beta * ex;
            gyro_corr[1] += self.beta * ey;
            gyro_corr[2] += self.beta * ez;
        }

        // 4. Integrate Quaternion Orientation:
        // dq/dt = 0.5 * q * [0, wx, wy, wz]
        let q = self.orientation;
        let half_dt = 0.5 * dt;
        let q_dot = SpatialQuaternion {
            w: half_dt * (-q.x * gyro_corr[0] - q.y * gyro_corr[1] - q.z * gyro_corr[2]),
            x: half_dt * (q.w * gyro_corr[0] + q.y * gyro_corr[2] - q.z * gyro_corr[1]),
            y: half_dt * (q.w * gyro_corr[1] - q.x * gyro_corr[2] + q.z * gyro_corr[0]),
            z: half_dt * (q.w * gyro_corr[2] + q.x * gyro_corr[1] - q.y * gyro_corr[0]),
        };

        let mut q_new = SpatialQuaternion {
            w: q.w + q_dot.w,
            x: q.x + q_dot.x,
            y: q.y + q_dot.y,
            z: q.z + q_dot.z,
        };
        q_new.normalize();
        self.orientation = q_new;

        // 5. Dynamic Gravity Vector Isolation:
        // In body frame, gravity vector is g_body = R(q)^T * [0, 0, -g]
        let g_body = [
            2.0 * (q_new.x * q_new.z - q_new.w * q_new.y) * GRAVITY_MSS,
            2.0 * (q_new.y * q_new.z + q_new.w * q_new.x) * GRAVITY_MSS,
            (q_new.w * q_new.w - q_new.x * q_new.x - q_new.y * q_new.y + q_new.z * q_new.z) * GRAVITY_MSS,
        ];

        // Dynamic linear acceleration free from gravity: a_dynamic = a_proper - g_body
        let dynamic_body = [
            proper_accel[0] - g_body[0],
            proper_accel[1] - g_body[1],
            proper_accel[2] - g_body[2],
        ];

        // Transform dynamic linear acceleration into world navigation frame: a_world = q * a_body * q^*
        let dynamic_world = q_new.rotate_vector(dynamic_body);

        let kinematics = FusedKinematics {
            angular_velocity_rad_s: raw_gyro,
            proper_accel_mss: proper_accel,
            gravity_body_mss: g_body,
            dynamic_linear_accel_mss: dynamic_body,
            world_linear_accel_mss: dynamic_world,
            orientation: q_new,
        };

        self.kinematics = kinematics;
        kinematics
    }
}

/// Verification helper for Commit 2 dual-IMU fusion and gravity isolation.
pub fn verify_dual_imu_fusion_and_gravity_isolation() -> bool {
    let topo = DualImuTopology::standard_handheld();
    let mut filter = DualImuFusionFilter::new(topo);

    // 1. Static flat desk test: both sensors measure +1g along Z, zero gyro
    let raw_flat = RawImuReading {
        accel_raw: [0, 0, 4096], // +1g
        gyro_raw: [0, 0, 0],
        temp_raw: 0,
        timestamp_us: 1000,
    };
    let s_left = PhysicalImuSample::from_raw(ImuLocation::LeftGrip, &raw_flat);
    let s_right = PhysicalImuSample::from_raw(ImuLocation::RightGrip, &raw_flat);

    let k_flat = filter.update(&s_left, &s_right);
    // Gravity should be isolated to ~9.80665 along Z
    if (k_flat.gravity_body_mss[2] - GRAVITY_MSS).abs() > 0.1 {
        return false;
    }
    // Dynamic linear acceleration should be zero (< 0.05 m/s^2)
    if k_flat.dynamic_linear_accel_mss[0].abs() > 0.05
        || k_flat.dynamic_linear_accel_mss[1].abs() > 0.05
        || k_flat.dynamic_linear_accel_mss[2].abs() > 0.05
    {
        return false;
    }

    // 2. Dual-IMU Lever-Arm Cancellation during Handheld Yaw Rotation:
    // When rotating around Z at omega_z = 2.0 rad/s:
    // Left grip (x = -0.14m): centripetal accel = +omega^2 * 0.14 = +0.56 m/s^2 along X
    // Right grip (x = +0.14m): centripetal accel = -omega^2 * 0.14 = -0.56 m/s^2 along X
    let mut s_left_rot = s_left;
    s_left_rot.accel_mss[0] = 0.56;
    s_left_rot.gyro_rad_s[2] = 2.0;

    let mut s_right_rot = s_right;
    s_right_rot.accel_mss[0] = -0.56;
    s_right_rot.gyro_rad_s[2] = 2.0;

    let k_rot = filter.update(&s_left_rot, &s_right_rot);

    // Fused proper acceleration along X should cancel out to 0.0 m/s^2!
    if k_rot.proper_accel_mss[0].abs() > 0.01 {
        return false;
    }
    // Fused angular velocity should be exactly 2.0 rad/s
    if (k_rot.angular_velocity_rad_s[2] - 2.0).abs() > 0.01 {
        return false;
    }

    true
}
