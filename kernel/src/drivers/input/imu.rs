//! 6-Axis Inertial Measurement Unit (IMU) Accelerometer & Gyroscope Structures.
//!
//! Provides raw sensor representations, configurable sensitivities (LSB/g and LSB/dps),
//! zero-bias calibration, and first-order low-pass filtering for high-rate IMUs (e.g. 1000Hz).

/// Standard gravitational acceleration in m/s^2.
pub const GRAVITY_MSS: f32 = 9.80665;

/// Conversion multiplier from degrees to radians.
pub const DEG_TO_RAD: f32 = 0.017453292519943295;

/// Conversion multiplier from radians to degrees.
pub const RAD_TO_DEG: f32 = 57.29577951308232;

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

/// 3D floating-point vector used for spatial acceleration and angular velocity.
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
            crate::graphics::sqrt_f32(sq)
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
    ///
    /// When stationary with the Z axis pointing up against gravity (+1g along Z),
    /// the expected acceleration is [0, 0, +GRAVITY_MSS].
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
        // Calculate 1st order IIR filter smoothing factor alpha = dt / (RC + dt)
        // RC = 1 / (2 * PI * fc)
        let dt = 1.0 / (config.sample_rate_hz as f32);
        let rc = 1.0 / (2.0 * crate::graphics::PI * config.lpf_cutoff_hz);
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

    /// Updates calibration offsets in real time.
    pub fn set_calibration(&mut self, calibration: ImuCalibration) {
        self.calibration = calibration;
    }

    /// Converts raw IMU sensor reading into calibrated physical units and applies low-pass filtering.
    pub fn process_raw(&mut self, raw: &RawImuSample) -> ImuSample {
        let uncal_ax = self.config.accel_range.raw_to_mss(raw.ax);
        let uncal_ay = self.config.accel_range.raw_to_mss(raw.ay);
        let uncal_az = self.config.accel_range.raw_to_mss(raw.az);

        let uncal_gx = self.config.gyro_range.raw_to_rad_per_sec(raw.gx);
        let uncal_gy = self.config.gyro_range.raw_to_rad_per_sec(raw.gy);
        let uncal_gz = self.config.gyro_range.raw_to_rad_per_sec(raw.gz);

        // Apply zero-bias subtraction and axis scaling
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

        // Temperature formula for typical IMUs (e.g., MPU/ICM): Temp = (raw / 333.87) + 21.0
        let temp_c = (raw.temp_raw as f32 / 333.87) + 21.0;

        // Apply low pass filter
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

    /// Returns the current low-pass filtered acceleration.
    #[inline(always)]
    pub fn current_accel(&self) -> Vector3 {
        self.filtered_accel
    }

    /// Returns the current low-pass filtered angular rate.
    #[inline(always)]
    pub fn current_gyro(&self) -> Vector3 {
        self.filtered_gyro
    }

    /// Returns total processed samples.
    #[inline(always)]
    pub fn sample_count(&self) -> u64 {
        self.sample_count
    }
}
