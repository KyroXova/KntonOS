//! Sensor Drivers and Governors for Handheld Devices.

pub mod als;
pub mod brightness;
pub mod imu_fusion;

pub use als::*;
pub use brightness::*;
pub use imu_fusion::*;

/// Comprehensive verification self-test suite for Handheld Ambient Light Sensor (ALS)
/// and HDR Auto-Brightness / Contrast Governor.
///
/// Emits sentinel [KNT-ALS-BRIGHTNESS-PASS] upon successful validation.
pub fn test_als_and_brightness_subsystem() -> bool {
    // 1. Verify ALS hardware interface, lux categorization, and EMA noise filter
    if !verify_als_driver_and_lux_filter() {
        return false;
    }

    // 2. Verify Stevens' power law logarithmic brightness curve, interpolation, and PWM duty
    if !verify_brightness_curve_and_pwm_table() {
        return false;
    }

    // 3. Verify asymmetric temporal slewing and ambient-adaptive HDR contrast governor
    if !verify_slewing_and_hdr_contrast_governor() {
        return false;
    }

    // 4. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-ALS-BRIGHTNESS-PASS] Handheld ALS ambient light sensor and HDR auto-brightness governor verified\r\n");

    true
}

/// Verification self-test suite for Handheld PC Dual-IMU Dead Reckoning & Motion Drift Eliminator.
pub fn test_dual_imu_dead_reckoning_subsystem() -> bool {
    crate::serial_write_str("[KNT-SENSOR] Testing dual-IMU sensor fusion & dead-reckoning engine...\r\n");

    // 1. Verify Dual-IMU Spatial Topology & Quaternion Kinematics
    if !verify_dual_imu_topology_and_quaternions() {
        crate::serial_write_str("[KNT-PANIC] Dual-IMU topology and quaternion verification failed!\r\n");
        return false;
    }

    // 2. Verify Dual-IMU Signal Fusion, Lever-Arm Cancellation & Gravity Isolation
    if !verify_dual_imu_fusion_and_gravity_isolation() {
        crate::serial_write_str("[KNT-PANIC] Dual-IMU sensor fusion and gravity isolation verification failed!\r\n");
        return false;
    }

    // 3. Verify Zero-Velocity Update (ZUPT) & Sub-Millimeter Dead Reckoning Tracker
    if !verify_zupt_drift_cancellation_and_dead_reckoning() {
        crate::serial_write_str("[KNT-PANIC] Dual-IMU ZUPT drift cancellation verification failed!\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-INPUT-IMU-FUSION-PASS] Dual-IMU dead reckoning and motion drift eliminator verified\r\n");
    true
}
