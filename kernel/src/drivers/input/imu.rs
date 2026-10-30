//! 6-axis IMU accelerometer and gyroscope sensor structures and fusion filters.

/// Standard gravitational acceleration in m/s^2.
pub const GRAVITY_MSS: f32 = 9.80665;

/// Conversion multiplier from degrees to radians.
pub const DEG_TO_RAD: f32 = 0.017453292519943295;

/// Conversion multiplier from radians to degrees.
pub const RAD_TO_DEG: f32 = 57.29577951308232;

/// Mathematical constant Pi.
pub const PI: f32 = 3.14159265358979323846;

/// Mathematical constant Pi / 2.
pub const HALF_PI: f32 = PI / 2.0;

// Trigonometric and polynomial math helpers.

/// Computes the square root of a floating-point number using Newton-Raphson iteration.
#[inline(always)]
pub fn sqrt_f32(x: f32) -> f32 {
    crate::graphics::sqrt_f32(x)
}

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

/// Computes the arcsine of `x` in radians within [-HALF_PI, HALF_PI].
pub fn asin_f32(mut x: f32) -> f32 {
    if x > 1.0 {
        x = 1.0;
    } else if x < -1.0 {
        x = -1.0;
    }
    let denom = sqrt_f32(1.0 - x * x);
    atan2_f32(x, denom)
}

// Sensor ranges and Vector3.

/// Accelerometer full-scale range configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AccelRange {
    /// +/- 2g range (16384 LSB/g)
    Range2g = 0,
    /// +/- 4g range (8192 LSB/g)
    Range4g = 1,
    /// +/- 8g range (4096 LSB/g)
    Range8g = 2,
    /// +/- 16g range (2048 LSB/g)
    Range16g = 3,
}

impl AccelRange {
    /// Returns the sensitivity divisor in LSB per 1.0 g.
    #[inline(always)]
    pub fn lsb_per_g(&self) -> f32 {
        match self {
            AccelRange::Range2g => 16384.0,
            AccelRange::Range4g => 8192.0,
            AccelRange::Range8g => 4096.0,
            AccelRange::Range16g => 2048.0,
        }
    }

    /// Converts raw integer 16-bit reading into acceleration in m/s^2.
    #[inline(always)]
    pub fn raw_to_mss(&self, raw: i16) -> f32 {
        (raw as f32 / self.lsb_per_g()) * GRAVITY_MSS
    }

    /// Converts raw integer 16-bit reading into acceleration in g.
    #[inline(always)]
    pub fn raw_to_g(&self, raw: i16) -> f32 {
        raw as f32 / self.lsb_per_g()
    }
}

/// Gyroscope full-scale angular rate range configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum GyroRange {
    /// +/- 250 degrees per second (131.0 LSB / (deg/s))
    Dps250 = 0,
    /// +/- 500 degrees per second (65.5 LSB / (deg/s))
    Dps500 = 1,
    /// +/- 1000 degrees per second (32.8 LSB / (deg/s))
    Dps1000 = 2,
    /// +/- 2000 degrees per second (16.4 LSB / (deg/s))
    Dps2000 = 3,
}

impl GyroRange {
    /// Returns the sensitivity divisor in LSB per (deg/s).
    #[inline(always)]
    pub fn lsb_per_dps(&self) -> f32 {
        match self {
            GyroRange::Dps250 => 131.0,
            GyroRange::Dps500 => 65.5,
            GyroRange::Dps1000 => 32.8,
            GyroRange::Dps2000 => 16.4,
        }
    }

    /// Converts raw integer 16-bit reading into angular velocity in degrees per second.
    #[inline(always)]
    pub fn raw_to_dps(&self, raw: i16) -> f32 {
        raw as f32 / self.lsb_per_dps()
    }

    /// Converts raw integer 16-bit reading into angular velocity in radians per second.
    #[inline(always)]
    pub fn raw_to_rad_per_sec(&self, raw: i16) -> f32 {
        (raw as f32 / self.lsb_per_dps()) * DEG_TO_RAD
    }
}

/// 3D floating-point vector used for spatial acceleration, angular rate, and geometry.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vector3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vector3 {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub const fn zero() -> Self {
        Self { x: 0.0, y: 0.0, z: 0.0 }
    }

    #[inline(always)]
    pub fn add(self, other: Vector3) -> Vector3 {
        Vector3::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }

    #[inline(always)]
    pub fn sub(self, other: Vector3) -> Vector3 {
        Vector3::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }

    #[inline(always)]
    pub fn scale(self, factor: f32) -> Vector3 {
        Vector3::new(self.x * factor, self.y * factor, self.z * factor)
    }

    #[inline(always)]
    pub fn mul_components(self, other: Vector3) -> Vector3 {
        Vector3::new(self.x * other.x, self.y * other.y, self.z * other.z)
    }

    #[inline(always)]
    pub fn dot(self, other: Vector3) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    #[inline(always)]
    pub fn cross(self, other: Vector3) -> Vector3 {
        Vector3::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    #[inline(always)]
    pub fn norm_sq(self) -> f32 {
        self.x * self.x + self.y * self.y + self.z * self.z
    }

    #[inline(always)]
    pub fn norm(self) -> f32 {
        let sq = self.norm_sq();
        if sq <= 0.0 {
            0.0
        } else {
            sqrt_f32(sq)
        }
    }

    #[inline(always)]
    pub fn normalize(self) -> Vector3 {
        let n = self.norm();
        if n > 1e-7 {
            let inv = 1.0 / n;
            self.scale(inv)
        } else {
            Vector3::zero()
        }
    }
}

/// Raw sensor reading packet as sampled directly from the IMU hardware registers / FIFO.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RawImuSample {
    /// Raw 16-bit signed acceleration along X axis.
    pub ax: i16,
    /// Raw 16-bit signed acceleration along Y axis.
    pub ay: i16,
    /// Raw 16-bit signed acceleration along Z axis.
    pub az: i16,
    /// Raw 16-bit signed angular rate around X axis.
    pub gx: i16,
    /// Raw 16-bit signed angular rate around Y axis.
    pub gy: i16,
    /// Raw 16-bit signed angular rate around Z axis.
    pub gz: i16,
    /// Raw 16-bit internal temperature sensor reading.
    pub temp_raw: i16,
    /// Sample timestamp in microseconds.
    pub timestamp_us: u64,
}

/// Calibrated physical unit IMU sample.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ImuSample {
    /// Linear acceleration in m/s^2 (X, Y, Z).
    pub accel: Vector3,
    /// Angular velocity in radians per second (X, Y, Z).
    pub gyro: Vector3,
    /// Temperature reading in degrees Celsius.
    pub temp_celsius: f32,
    /// Timestamp in microseconds.
    pub timestamp_us: u64,
}

/// Calibration offsets and scale sensitivity corrections.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImuCalibration {
    /// Accelerometer zero-bias offset in m/s^2.
    pub accel_bias: Vector3,
    /// Gyroscope zero-rate bias offset in rad/s.
    pub gyro_bias: Vector3,
    /// Accelerometer axis scale correction factor (nominal 1.0).
    pub accel_scale: Vector3,
}

impl Default for ImuCalibration {
    fn default() -> Self {
        Self {
            accel_bias: Vector3::zero(),
            gyro_bias: Vector3::zero(),
            accel_scale: Vector3::new(1.0, 1.0, 1.0),
        }
    }
}

impl ImuCalibration {
    pub const fn new() -> Self {
        Self {
            accel_bias: Vector3::zero(),
            gyro_bias: Vector3::zero(),
            accel_scale: Vector3::new(1.0, 1.0, 1.0),
        }
    }

    /// Computes zero-motion calibration biases from a buffer of resting raw samples.
    pub fn compute_static_bias(
        samples: &[RawImuSample],
        accel_range: AccelRange,
        gyro_range: GyroRange,
        z_up_gravity: bool,
    ) -> Option<Self> {
        if samples.is_empty() {
            return None;
        }

        let n = samples.len() as f32;
        let mut sum_ax = 0.0f32;
        let mut sum_ay = 0.0f32;
        let mut sum_az = 0.0f32;
        let mut sum_gx = 0.0f32;
        let mut sum_gy = 0.0f32;
        let mut sum_gz = 0.0f32;

        for s in samples {
            sum_ax += accel_range.raw_to_mss(s.ax);
            sum_ay += accel_range.raw_to_mss(s.ay);
            sum_az += accel_range.raw_to_mss(s.az);

            sum_gx += gyro_range.raw_to_rad_per_sec(s.gx);
            sum_gy += gyro_range.raw_to_rad_per_sec(s.gy);
            sum_gz += gyro_range.raw_to_rad_per_sec(s.gz);
        }

        let avg_ax = sum_ax / n;
        let avg_ay = sum_ay / n;
        let avg_az = sum_az / n;

        // If Z is aligned with gravity (+1g = +9.80665 m/s^2), bias is avg_az - GRAVITY_MSS
        let bias_az = if z_up_gravity {
            avg_az - GRAVITY_MSS
        } else {
            avg_az
        };

        Some(Self {
            accel_bias: Vector3::new(avg_ax, avg_ay, bias_az),
            gyro_bias: Vector3::new(sum_gx / n, sum_gy / n, sum_gz / n),
            accel_scale: Vector3::new(1.0, 1.0, 1.0),
        })
    }
}

/// Configuration settings for the IMU frontend.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImuConfig {
    pub accel_range: AccelRange,
    pub gyro_range: GyroRange,
    pub sample_rate_hz: u32,
    pub lpf_cutoff_hz: f32,
}

impl Default for ImuConfig {
    fn default() -> Self {
        Self {
            accel_range: AccelRange::Range8g,
            gyro_range: GyroRange::Dps2000,
            sample_rate_hz: 1000,
            lpf_cutoff_hz: 100.0,
        }
    }
}

/// IMU sample processing pipeline with calibration, temperature compensation, and IIR low-pass filter.
pub struct ImuProcessor {
    pub config: ImuConfig,
    pub calibration: ImuCalibration,
    filtered_accel: Vector3,
    filtered_gyro: Vector3,
    lpf_alpha: f32,
    sample_count: u64,
}

impl ImuProcessor {
    pub fn new(config: ImuConfig, calibration: ImuCalibration) -> Self {
        let dt = 1.0 / (config.sample_rate_hz as f32);
        let rc = 1.0 / (2.0 * PI * config.lpf_cutoff_hz);
        let alpha = dt / (rc + dt);

        Self {
            config,
            calibration,
            filtered_accel: Vector3::zero(),
            filtered_gyro: Vector3::zero(),
            lpf_alpha: alpha,
            sample_count: 0,
        }
    }

    pub fn set_calibration(&mut self, calibration: ImuCalibration) {
        self.calibration = calibration;
    }

    pub fn process_raw(&mut self, raw: &RawImuSample) -> ImuSample {
        let uncal_ax = self.config.accel_range.raw_to_mss(raw.ax);
        let uncal_ay = self.config.accel_range.raw_to_mss(raw.ay);
        let uncal_az = self.config.accel_range.raw_to_mss(raw.az);

        let uncal_gx = self.config.gyro_range.raw_to_rad_per_sec(raw.gx);
        let uncal_gy = self.config.gyro_range.raw_to_rad_per_sec(raw.gy);
        let uncal_gz = self.config.gyro_range.raw_to_rad_per_sec(raw.gz);

        let cal_accel = Vector3::new(
            (uncal_ax - self.calibration.accel_bias.x) * self.calibration.accel_scale.x,
            (uncal_ay - self.calibration.accel_bias.y) * self.calibration.accel_scale.y,
            (uncal_az - self.calibration.accel_bias.z) * self.calibration.accel_scale.z,
        );

        let cal_gyro = Vector3::new(
            uncal_gx - self.calibration.gyro_bias.x,
            uncal_gy - self.calibration.gyro_bias.y,
            uncal_gz - self.calibration.gyro_bias.z,
        );

        let temp_c = (raw.temp_raw as f32 / 333.87) + 21.0;

        if self.sample_count == 0 {
            self.filtered_accel = cal_accel;
            self.filtered_gyro = cal_gyro;
        } else {
            let a = self.lpf_alpha;
            self.filtered_accel = Vector3::new(
                self.filtered_accel.x + a * (cal_accel.x - self.filtered_accel.x),
                self.filtered_accel.y + a * (cal_accel.y - self.filtered_accel.y),
                self.filtered_accel.z + a * (cal_accel.z - self.filtered_accel.z),
            );
            self.filtered_gyro = Vector3::new(
                self.filtered_gyro.x + a * (cal_gyro.x - self.filtered_gyro.x),
                self.filtered_gyro.y + a * (cal_gyro.y - self.filtered_gyro.y),
                self.filtered_gyro.z + a * (cal_gyro.z - self.filtered_gyro.z),
            );
        }

        self.sample_count = self.sample_count.saturating_add(1);

        ImuSample {
            accel: self.filtered_accel,
            gyro: self.filtered_gyro,
            temp_celsius: temp_c,
            timestamp_us: raw.timestamp_us,
        }
    }

    #[inline(always)]
    pub fn current_accel(&self) -> Vector3 {
        self.filtered_accel
    }

    #[inline(always)]
    pub fn current_gyro(&self) -> Vector3 {
        self.filtered_gyro
    }

    #[inline(always)]
    pub fn sample_count(&self) -> u64 {
        self.sample_count
    }
}

// Quaternion and orientation math.

/// Unit quaternion representing spatial 3D orientation: q = w + xi + yj + zk.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quaternion {
    pub w: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Default for Quaternion {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Quaternion {
    /// Identity orientation quaternion (no rotation).
    pub const IDENTITY: Self = Self {
        w: 1.0,
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    pub const fn new(w: f32, x: f32, y: f32, z: f32) -> Self {
        Self { w, x, y, z }
    }

    /// Constructs a quaternion from an axis of rotation and an angle in radians.
    pub fn from_axis_angle(axis: Vector3, angle_rad: f32) -> Self {
        let half_angle = angle_rad * 0.5;
        let s = crate::graphics::sin_f32(half_angle);
        let c = crate::graphics::cos_f32(half_angle);
        let norm_axis = axis.normalize();
        Self {
            w: c,
            x: norm_axis.x * s,
            y: norm_axis.y * s,
            z: norm_axis.z * s,
        }
    }

    /// Hamilton product of two quaternions (composition of rotations self * other).
    #[inline(always)]
    pub fn multiply(self, other: Quaternion) -> Quaternion {
        Quaternion {
            w: self.w * other.w - self.x * other.x - self.y * other.y - self.z * other.z,
            x: self.w * other.x + self.x * other.w + self.y * other.z - self.z * other.y,
            y: self.w * other.y - self.x * other.z + self.y * other.w + self.z * other.x,
            z: self.w * other.z + self.x * other.y - self.y * other.x + self.z * other.w,
        }
    }

    /// Complex conjugate of the quaternion (inverse for unit quaternions).
    #[inline(always)]
    pub fn conjugate(self) -> Quaternion {
        Quaternion {
            w: self.w,
            x: -self.x,
            y: -self.y,
            z: -self.z,
        }
    }

    #[inline(always)]
    pub fn norm_sq(self) -> f32 {
        self.w * self.w + self.x * self.x + self.y * self.y + self.z * self.z
    }

    #[inline(always)]
    pub fn norm(self) -> f32 {
        let sq = self.norm_sq();
        if sq <= 0.0 {
            0.0
        } else {
            sqrt_f32(sq)
        }
    }

    /// Normalizes the quaternion to unit length.
    #[inline(always)]
    pub fn normalize(self) -> Quaternion {
        let n = self.norm();
        if n > 1e-7 {
            let inv = 1.0 / n;
            Quaternion {
                w: self.w * inv,
                x: self.x * inv,
                y: self.y * inv,
                z: self.z * inv,
            }
        } else {
            Quaternion::IDENTITY
        }
    }

    /// Rotates a 3D vector `v` by this quaternion: v' = q * v * q^-1.
    pub fn rotate_vector(self, v: Vector3) -> Vector3 {
        // Efficient vector rotation formula: v' = v + 2 * r x (r x v + w * v) where r = [x, y, z]
        let r = Vector3::new(self.x, self.y, self.z);
        let rxv = r.cross(v);
        let rxv_plus_wv = rxv.add(v.scale(self.w));
        let two_rx = r.cross(rxv_plus_wv).scale(2.0);
        v.add(two_rx)
    }

    /// Converts orientation quaternion into Tait-Bryan Euler angles (Roll, Pitch, Yaw) in radians.
    pub fn to_euler_angles(self) -> EulerAngles {
        // Roll (x-axis rotation): atan2(2(wx + yz), 1 - 2(x^2 + y^2))
        let sinr_cosp = 2.0 * (self.w * self.x + self.y * self.z);
        let cosr_cosp = 1.0 - 2.0 * (self.x * self.x + self.y * self.y);
        let roll = atan2_f32(sinr_cosp, cosr_cosp);

        // Pitch (y-axis rotation): asin(2(wy - zx))
        let sinp = 2.0 * (self.w * self.y - self.z * self.x);
        let pitch = asin_f32(sinp);

        // Yaw (z-axis rotation): atan2(2(wz + xy), 1 - 2(y^2 + z^2))
        let siny_cosp = 2.0 * (self.w * self.z + self.x * self.y);
        let cosy_cosp = 1.0 - 2.0 * (self.y * self.y + self.z * self.z);
        let yaw = atan2_f32(siny_cosp, cosy_cosp);

        EulerAngles { roll, pitch, yaw }
    }

    /// Converts orientation quaternion into a 3x3 rotation matrix.
    pub fn to_rotation_matrix(self) -> [[f32; 3]; 3] {
        let q = self.normalize();
        let xx = q.x * q.x;
        let yy = q.y * q.y;
        let zz = q.z * q.z;
        let xy = q.x * q.y;
        let xz = q.x * q.z;
        let yz = q.y * q.z;
        let wx = q.w * q.x;
        let wy = q.w * q.y;
        let wz = q.w * q.z;

        [
            [1.0 - 2.0 * (yy + zz), 2.0 * (xy - wz), 2.0 * (xz + wy)],
            [2.0 * (xy + wz), 1.0 - 2.0 * (xx + zz), 2.0 * (yz - wx)],
            [2.0 * (xz - wy), 2.0 * (yz + wx), 1.0 - 2.0 * (xx + yy)],
        ]
    }
}

/// Tait-Bryan Euler angles (Roll, Pitch, Yaw) describing angular orientation.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct EulerAngles {
    /// Roll angle around X axis (radians).
    pub roll: f32,
    /// Pitch angle around Y axis (radians).
    pub pitch: f32,
    /// Yaw angle around Z axis (radians).
    pub yaw: f32,
}

impl EulerAngles {
    pub const fn new(roll: f32, pitch: f32, yaw: f32) -> Self {
        Self { roll, pitch, yaw }
    }

    #[inline(always)]
    pub fn roll_deg(&self) -> f32 {
        self.roll * RAD_TO_DEG
    }

    #[inline(always)]
    pub fn pitch_deg(&self) -> f32 {
        self.pitch * RAD_TO_DEG
    }

    #[inline(always)]
    pub fn yaw_deg(&self) -> f32 {
        self.yaw * RAD_TO_DEG
    }
}

// Real-time quaternion sensor fusion filters.

/// Sebastian Madgwick 6-Axis IMU Gradient Descent Quaternion Fusion Filter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MadgwickFilter {
    /// Filter gain beta representing divergence rate / gyroscope error.
    pub beta: f32,
    /// Current estimated orientation quaternion.
    pub q: Quaternion,
}

impl Default for MadgwickFilter {
    fn default() -> Self {
        Self::new(0.041)
    }
}

impl MadgwickFilter {
    pub const fn new(beta: f32) -> Self {
        Self {
            beta,
            q: Quaternion::IDENTITY,
        }
    }

    /// Resets the filter quaternion to identity.
    pub fn reset(&mut self) {
        self.q = Quaternion::IDENTITY;
    }

    /// Updates orientation quaternion using accelerometer and gyroscope measurements.
    pub fn update(&mut self, accel: Vector3, gyro: Vector3, dt: f32) {
        let q = self.q;

        // Rate of change of quaternion from gyroscope: q_dot = 0.5 * q * [0, gx, gy, gz]
        let q_dot_w = 0.5 * (-q.x * gyro.x - q.y * gyro.y - q.z * gyro.z);
        let q_dot_x = 0.5 * ( q.w * gyro.x + q.y * gyro.z - q.z * gyro.y);
        let q_dot_y = 0.5 * ( q.w * gyro.y - q.x * gyro.z + q.z * gyro.x);
        let q_dot_z = 0.5 * ( q.w * gyro.z + q.x * gyro.y - q.y * gyro.x);

        let accel_norm = accel.norm();
        if accel_norm > 1e-4 {
            // Normalize accelerometer measurement
            let ax = accel.x / accel_norm;
            let ay = accel.y / accel_norm;
            let az = accel.z / accel_norm;

            // Auxiliary variables to avoid repeated calculations
            let q_w_2 = 2.0 * q.w;
            let q_x_2 = 2.0 * q.x;
            let q_y_2 = 2.0 * q.y;
            let q_z_2 = 2.0 * q.z;
            let q_x_4 = 4.0 * q.x;
            let q_y_4 = 4.0 * q.y;
            let q_x_sq = q.x * q.x;
            let q_y_sq = q.y * q.y;

            // Objective function f(q, a)
            let f_1 = q_x_2 * q.z - q_w_2 * q.y - ax;
            let f_2 = q_w_2 * q.x + q_y_2 * q.z - ay;
            let f_3 = 1.0 - 2.0 * q_x_sq - 2.0 * q_y_sq - az;

            // Gradient descent step: J^T * f
            let mut s_w = -q_y_2 * f_1 + q_x_2 * f_2;
            let mut s_x =  q_z_2 * f_1 + q_w_2 * f_2 - q_x_4 * f_3;
            let mut s_y = -q_w_2 * f_1 + q_z_2 * f_2 - q_y_4 * f_3;
            let mut s_z =  q_x_2 * f_1 + q_y_2 * f_2;

            // Normalize gradient
            let s_norm = sqrt_f32(s_w * s_w + s_x * s_x + s_y * s_y + s_z * s_z);
            if s_norm > 1e-7 {
                let inv_s = 1.0 / s_norm;
                s_w *= inv_s;
                s_x *= inv_s;
                s_y *= inv_s;
                s_z *= inv_s;

                // Apply feedback step
                let final_dot_w = q_dot_w - self.beta * s_w;
                let final_dot_x = q_dot_x - self.beta * s_x;
                let final_dot_y = q_dot_y - self.beta * s_y;
                let final_dot_z = q_dot_z - self.beta * s_z;

                self.q = Quaternion {
                    w: q.w + final_dot_w * dt,
                    x: q.x + final_dot_x * dt,
                    y: q.y + final_dot_y * dt,
                    z: q.z + final_dot_z * dt,
                }.normalize();
                return;
            }
        }

        // Gyroscope-only integration if accelerometer is near-zero (freefall) or invalid
        self.q = Quaternion {
            w: q.w + q_dot_w * dt,
            x: q.x + q_dot_x * dt,
            y: q.y + q_dot_y * dt,
            z: q.z + q_dot_z * dt,
        }.normalize();
    }
}

/// Robert Mahony 6-Axis IMU Complementary Quaternion Fusion Filter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MahonyFilter {
    /// Proportional gain Kp.
    pub kp: f32,
    /// Integral gain Ki.
    pub ki: f32,
    /// Current estimated orientation quaternion.
    pub q: Quaternion,
    /// Accumulated integral error.
    pub integral_error: Vector3,
}

impl Default for MahonyFilter {
    fn default() -> Self {
        Self::new(2.5, 0.05)
    }
}

impl MahonyFilter {
    pub const fn new(kp: f32, ki: f32) -> Self {
        Self {
            kp,
            ki,
            q: Quaternion::IDENTITY,
            integral_error: Vector3::zero(),
        }
    }

    pub fn reset(&mut self) {
        self.q = Quaternion::IDENTITY;
        self.integral_error = Vector3::zero();
    }

    /// Updates orientation quaternion using Mahony complementary feedback.
    pub fn update(&mut self, accel: Vector3, gyro: Vector3, dt: f32) {
        let q = self.q;
        let mut adj_gyro = gyro;

        let accel_norm = accel.norm();
        if accel_norm > 1e-4 {
            let ax = accel.x / accel_norm;
            let ay = accel.y / accel_norm;
            let az = accel.z / accel_norm;

            // Estimated gravity direction from current quaternion: v = [2(xy - wz), 2(wx + yz), w^2 - x^2 - y^2 + z^2]
            let vx = 2.0 * (q.x * q.z - q.w * q.y);
            let vy = 2.0 * (q.w * q.x + q.y * q.z);
            let vz = q.w * q.w - q.x * q.x - q.y * q.y + q.z * q.z;

            // Error is cross product of measured acceleration and estimated gravity
            let ex = ay * vz - az * vy;
            let ey = az * vx - ax * vz;
            let ez = ax * vy - ay * vx;
            let error = Vector3::new(ex, ey, ez);

            // Accumulate integral feedback
            if self.ki > 0.0 {
                self.integral_error = self.integral_error.add(error.scale(self.ki * dt));
            } else {
                self.integral_error = Vector3::zero();
            }

            // Adjust angular rates
            adj_gyro = adj_gyro.add(error.scale(self.kp)).add(self.integral_error);
        }

        // Integrate quaternion rate: q_dot = 0.5 * q * [0, gx, gy, gz]
        let q_dot_w = 0.5 * (-q.x * adj_gyro.x - q.y * adj_gyro.y - q.z * adj_gyro.z);
        let q_dot_x = 0.5 * ( q.w * adj_gyro.x + q.y * adj_gyro.z - q.z * adj_gyro.y);
        let q_dot_y = 0.5 * ( q.w * adj_gyro.y - q.x * adj_gyro.z + q.z * adj_gyro.x);
        let q_dot_z = 0.5 * ( q.w * adj_gyro.z + q.x * adj_gyro.y - q.y * adj_gyro.x);

        self.q = Quaternion {
            w: q.w + q_dot_w * dt,
            x: q.x + q_dot_x * dt,
            y: q.y + q_dot_y * dt,
            z: q.z + q_dot_z * dt,
        }.normalize();
    }
}

/// Unified 6-axis IMU sensor fusion pipeline combining filtering and linear acceleration extraction.
pub struct SensorFusion {
    pub processor: ImuProcessor,
    pub madgwick: MadgwickFilter,
    pub last_timestamp_us: u64,
}

impl SensorFusion {
    pub fn new(config: ImuConfig, calibration: ImuCalibration, beta: f32) -> Self {
        Self {
            processor: ImuProcessor::new(config, calibration),
            madgwick: MadgwickFilter::new(beta),
            last_timestamp_us: 0,
        }
    }

    /// Feeds a raw IMU sample into the fusion pipeline and updates real-time orientation.
    pub fn update_raw(&mut self, raw: &RawImuSample) -> (Quaternion, EulerAngles, Vector3) {
        let sample = self.processor.process_raw(raw);
        let dt = if self.last_timestamp_us == 0 || raw.timestamp_us <= self.last_timestamp_us {
            1.0 / (self.processor.config.sample_rate_hz as f32)
        } else {
            let diff_us = raw.timestamp_us - self.last_timestamp_us;
            (diff_us as f32) * 1e-6
        };
        self.last_timestamp_us = raw.timestamp_us;

        // Run Madgwick filter update
        self.madgwick.update(sample.accel, sample.gyro, dt);
        let q = self.madgwick.q;
        let euler = q.to_euler_angles();

        // Calculate dynamic linear acceleration (removes rotated 1g gravity vector from sensor reading)
        // Gravity in world frame is [0, 0, +GRAVITY_MSS] (or pointing upward in sensor reference when flat)
        let gravity_world = Vector3::new(0.0, 0.0, GRAVITY_MSS);
        let gravity_sensor = q.conjugate().rotate_vector(gravity_world);
        let dynamic_accel = sample.accel.sub(gravity_sensor);

        (q, euler, dynamic_accel)
    }

    /// Directly updates orientation given calibrated physical units.
    pub fn update_calibrated(&mut self, accel: Vector3, gyro: Vector3, dt: f32) -> (Quaternion, EulerAngles, Vector3) {
        self.madgwick.update(accel, gyro, dt);
        let q = self.madgwick.q;
        let euler = q.to_euler_angles();

        let gravity_world = Vector3::new(0.0, 0.0, GRAVITY_MSS);
        let gravity_sensor = q.conjugate().rotate_vector(gravity_world);
        let dynamic_accel = accel.sub(gravity_sensor);

        (q, euler, dynamic_accel)
    }

    #[inline(always)]
    pub fn quaternion(&self) -> Quaternion {
        self.madgwick.q
    }

    #[inline(always)]
    pub fn euler_angles(&self) -> EulerAngles {
        self.madgwick.q.to_euler_angles()
    }
}
