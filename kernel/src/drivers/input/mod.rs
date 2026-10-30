//! Input Subsystems for KntonOS (IMU Sensor Fusion & High-Performance Gamepad Pipeline).

pub mod imu;
pub mod gamepad;
pub mod waveform;
pub mod haptic;
pub mod fast_ring;

pub use imu::*;
pub use gamepad::*;
pub use waveform::*;
pub use haptic::*;
pub use fast_ring::*;

use crate::serial_write_str;

/// Verification self-test suite for 6-axis IMU sensor fusion and 1000Hz gamepad pipeline.
pub fn test_imu_and_gamepad_subsystem() -> bool {
    serial_write_str("[KNT-INPUT] Testing 6-axis IMU sensor fusion & 1000Hz gamepad pipeline...\r\n");

    // -------------------------------------------------------------------------
    // 1. Test IMU Sensitivity & Scale Conversions
    // -------------------------------------------------------------------------
    {
        let accel_2g = AccelRange::Range2g;
        // 16384 LSB = 1.0 g = 9.80665 m/s^2
        let mss = accel_2g.raw_to_mss(16384);
        if (mss - GRAVITY_MSS).abs() > 0.01 {
            serial_write_str("[KNT-PANIC] Accel Range2g conversion mismatch!\r\n");
            return false;
        }

        let accel_8g = AccelRange::Range8g;
        // 4096 LSB = 1.0 g
        let g_val = accel_8g.raw_to_g(4096);
        if (g_val - 1.0).abs() > 0.001 {
            serial_write_str("[KNT-PANIC] Accel Range8g raw_to_g mismatch!\r\n");
            return false;
        }

        let gyro_250 = GyroRange::Dps250;
        // 131 LSB = 1.0 dps
        let dps = gyro_250.raw_to_dps(131);
        if (dps - 1.0).abs() > 0.01 {
            serial_write_str("[KNT-PANIC] Gyro Dps250 conversion mismatch!\r\n");
            return false;
        }

        let rad_s = gyro_250.raw_to_rad_per_sec(131);
        if (rad_s - DEG_TO_RAD).abs() > 0.001 {
            serial_write_str("[KNT-PANIC] Gyro rad/s conversion mismatch!\r\n");
            return false;
        }
    }

    // -------------------------------------------------------------------------
    // 2. Test IMU Static Calibration & Low-Pass Filtering
    // -------------------------------------------------------------------------
    {
        // Stationary samples pointing +1g upward along Z, with deliberate 50 LSB gyro bias
        let mut samples = [RawImuSample::default(); 16];
        for (i, s) in samples.iter_mut().enumerate() {
            s.ax = 0;
            s.ay = 0;
            s.az = 4096; // +1g on 8g scale (4096 LSB/g)
            s.gx = 33;   // ~1 deg/s bias on 1000 dps scale
            s.gy = -33;
            s.gz = 0;
            s.timestamp_us = (i as u64) * 1000;
        }

        let cal = match ImuCalibration::compute_static_bias(
            &samples,
            AccelRange::Range8g,
            GyroRange::Dps1000,
            true,
        ) {
            Some(c) => c,
            None => {
                serial_write_str("[KNT-PANIC] IMU static calibration failed!\r\n");
                return false;
            }
        };

        // Z bias should be close to 0 because measured AZ (9.80665) equals +1g GRAVITY_MSS
        if cal.accel_bias.z.abs() > 0.05 {
            serial_write_str("[KNT-PANIC] Accel Z bias computation error!\r\n");
            return false;
        }

        // Gyro biases should capture the +1 / -1 dps offset
        if cal.gyro_bias.x <= 0.0 || cal.gyro_bias.y >= 0.0 {
            serial_write_str("[KNT-PANIC] Gyro bias computation error!\r\n");
            return false;
        }

        // Test ImuProcessor with calibration applied
        let config = ImuConfig {
            accel_range: AccelRange::Range8g,
            gyro_range: GyroRange::Dps1000,
            sample_rate_hz: 1000,
            lpf_cutoff_hz: 50.0,
        };
        let mut processor = ImuProcessor::new(config, cal);
        let processed = processor.process_raw(&samples[0]);

        // Calibrated gyro rate should be approximately 0
        if processed.gyro.x.abs() > 0.01 || processed.gyro.y.abs() > 0.01 {
            serial_write_str("[KNT-PANIC] Processed gyro bias compensation failed!\r\n");
            return false;
        }
    }

    // -------------------------------------------------------------------------
    // 3. Test Quaternion Mathematics & Orientation Angles
    // -------------------------------------------------------------------------
    {
        // Identity quaternion
        let q_id = Quaternion::IDENTITY;
        if (q_id.norm() - 1.0).abs() > 1e-5 {
            serial_write_str("[KNT-PANIC] Quaternion identity norm invalid!\r\n");
            return false;
        }

        // Rotate vector (1, 0, 0) by 90 degrees (HALF_PI) around Z axis -> should yield (0, 1, 0)
        let q_rot_z = Quaternion::from_axis_angle(Vector3::new(0.0, 0.0, 1.0), HALF_PI);
        let v = Vector3::new(1.0, 0.0, 0.0);
        let v_rot = q_rot_z.rotate_vector(v);

        if v_rot.x.abs() > 1e-4 || (v_rot.y - 1.0).abs() > 1e-4 || v_rot.z.abs() > 1e-4 {
            serial_write_str("[KNT-PANIC] Quaternion vector rotation mismatch!\r\n");
            return false;
        }

        // Quaternion conjugate multiplication: q * q* = Identity
        let q_conj = q_rot_z.conjugate();
        let q_prod = q_rot_z.multiply(q_conj);
        if (q_prod.w - 1.0).abs() > 1e-4 || q_prod.x.abs() > 1e-4 || q_prod.y.abs() > 1e-4 || q_prod.z.abs() > 1e-4 {
            serial_write_str("[KNT-PANIC] Quaternion conjugate product not identity!\r\n");
            return false;
        }

        // Euler angles for 90 degree yaw
        let euler = q_rot_z.to_euler_angles();
        if (euler.yaw - HALF_PI).abs() > 1e-3 {
            serial_write_str("[KNT-PANIC] Euler yaw angle mismatch!\r\n");
            return false;
        }
        if (euler.yaw_deg() - 90.0).abs() > 0.1 {
            serial_write_str("[KNT-PANIC] Euler yaw degrees mismatch!\r\n");
            return false;
        }
    }

    // -------------------------------------------------------------------------
    // 4. Test Sensor Fusion Filters (Madgwick & Mahony) & Dynamic Acceleration
    // -------------------------------------------------------------------------
    {
        let config = ImuConfig::default();
        let cal = ImuCalibration::default();
        let mut fusion = SensorFusion::new(config, cal, 0.041);

        // Feed stationary vertical gravity samples: [0, 0, +GRAVITY_MSS], zero gyro
        let raw = RawImuSample {
            ax: 0,
            ay: 0,
            az: 4096, // 1g on Range8g
            gx: 0,
            gy: 0,
            gz: 0,
            temp_raw: 0,
            timestamp_us: 1000,
        };

        let (_q, euler, dyn_accel) = fusion.update_raw(&raw);

        // Resting orientation should have negligible roll and pitch
        if euler.roll.abs() > 0.05 || euler.pitch.abs() > 0.05 {
            serial_write_str("[KNT-PANIC] Sensor fusion resting attitude inaccurate!\r\n");
            return false;
        }

        // Dynamic linear acceleration (gravity subtracted) should be near zero
        if dyn_accel.norm() > 0.1 {
            serial_write_str("[KNT-PANIC] Dynamic acceleration subtraction inaccurate!\r\n");
            return false;
        }

        // Test Mahony filter convergence
        let mut mahony = MahonyFilter::new(2.5, 0.05);
        let accel = Vector3::new(0.0, 0.0, GRAVITY_MSS);
        let gyro = Vector3::zero();
        for _ in 0..10 {
            mahony.update(accel, gyro, 0.001);
        }
        let mq = mahony.q;
        if (mq.norm() - 1.0).abs() > 1e-4 {
            serial_write_str("[KNT-PANIC] Mahony filter quaternion not normalized!\r\n");
            return false;
        }
    }

    // -------------------------------------------------------------------------
    // 5. Test Analog Stick Deadzone Shaping & Response Curves
    // -------------------------------------------------------------------------
    {
        let deadzone = StickDeadzone::new(0.10, 0.90, 1.0, DeadzoneShape::Circular);

        // Center position -> 0.0
        let (x0, y0, mag0, _) = deadzone.shape(0, 0);
        if x0 != 0.0 || y0 != 0.0 || mag0 != 0.0 {
            serial_write_str("[KNT-PANIC] Center stick deadzone shaping failed!\r\n");
            return false;
        }

        // Small input inside inner deadzone (5% deflection) -> must clamp to 0.0
        let (x_in, y_in, mag_in, _) = deadzone.shape(1600, 0);
        if x_in != 0.0 || y_in != 0.0 || mag_in != 0.0 {
            serial_write_str("[KNT-PANIC] Inner stick deadzone suppression failed!\r\n");
            return false;
        }

        // Full deflection at or beyond outer threshold (95% deflection) -> must clamp to 1.0
        let (x_out, y_out, mag_out, _) = deadzone.shape(32000, 0);
        if (x_out - 1.0).abs() > 1e-4 || y_out.abs() > 1e-4 || (mag_out - 1.0).abs() > 1e-4 {
            serial_write_str("[KNT-PANIC] Outer stick deadzone clamping failed!\r\n");
            return false;
        }

        // Quadratic curve exponent shaping
        let quad_deadzone = StickDeadzone::new(0.0, 1.0, 2.0, DeadzoneShape::Circular);
        let (qx, _, qmag, _) = quad_deadzone.shape(16384, 0); // 0.5 linear
        // In quadratic curve: 0.5^2 = 0.25
        if (qx - 0.25).abs() > 0.02 || (qmag - 0.25).abs() > 0.02 {
            serial_write_str("[KNT-PANIC] Quadratic stick response curve mismatch!\r\n");
            return false;
        }
    }

    // -------------------------------------------------------------------------
    // 6. Test Trigger Normalization & Digital Click Actuation
    // -------------------------------------------------------------------------
    {
        let trig_deadzone = TriggerDeadzone::new(0.05, 0.95, 0.50);

        // Zero / resting pull
        let (val0, pressed0) = trig_deadzone.normalize_u16(0);
        if val0 != 0.0 || pressed0 {
            serial_write_str("[KNT-PANIC] Trigger zero normalization failed!\r\n");
            return false;
        }

        // Resting finger touch (< 5%) -> 0.0
        let (val_touch, pressed_touch) = trig_deadzone.normalize_u16(1500);
        if val_touch != 0.0 || pressed_touch {
            serial_write_str("[KNT-PANIC] Trigger resting deadzone suppression failed!\r\n");
            return false;
        }

        // Full pull (> 95%) -> 1.0 and pressed
        let (val_full, pressed_full) = trig_deadzone.normalize_u16(64000);
        if (val_full - 1.0).abs() > 1e-4 || !pressed_full {
            serial_write_str("[KNT-PANIC] Trigger full pull normalization failed!\r\n");
            return false;
        }
    }

    // -------------------------------------------------------------------------
    // 7. Test 1000Hz Gamepad Pipeline, Jitter Tracking & Gyro Aiming
    // -------------------------------------------------------------------------
    {
        let mut pipeline = GamepadPipeline::new();
        let fusion = SensorFusion::new(ImuConfig::default(), ImuCalibration::default(), 0.041);
        pipeline.attach_imu_fusion(fusion);
        pipeline.gyro_sensitivity = 1.5;

        // Simulate 10 frames at 1000Hz (1000 microseconds per frame = 1ms)
        let mut report = RawGamepadReport::default();
        report.buttons = GamepadButtons::A.0 | GamepadButtons::RB.0;
        report.left_stick_x = 0;
        report.left_stick_y = 0;
        report.right_stick_x = 16384;
        report.right_stick_y = 0;
        report.left_trigger = 0;
        report.right_trigger = 60000;
        report.imu_sample = Some(RawImuSample {
            ax: 0,
            ay: 0,
            az: 4096,
            gx: 0,
            gy: 0,
            gz: 33, // yaw angular rate
            temp_raw: 0,
            timestamp_us: 1000,
        });
        report.timestamp_us = 1000;

        let state1 = pipeline.process_report(&report);

        // Frame 1 button edge detection
        if !state1.buttons_just_pressed.contains(GamepadButtons::A)
            || !state1.buttons_just_pressed.contains(GamepadButtons::RB)
        {
            serial_write_str("[KNT-PANIC] Gamepad just_pressed edge detection failed!\r\n");
            return false;
        }

        // Stream remaining 9 frames at exact 1ms interval (1000Hz)
        for i in 2..=10 {
            report.timestamp_us = (i as u64) * 1000;
            // On frame 5, release button RB
            if i == 5 {
                report.buttons = GamepadButtons::A.0;
            }
            if let Some(ref mut imu) = report.imu_sample {
                imu.timestamp_us = report.timestamp_us;
            }
            let s = pipeline.process_report(&report);
            if i == 5 {
                if !s.buttons_just_released.contains(GamepadButtons::RB) {
                    serial_write_str("[KNT-PANIC] Gamepad just_released edge detection failed!\r\n");
                    return false;
                }
            }
        }

        if pipeline.total_reports() != 10 {
            serial_write_str("[KNT-PANIC] Gamepad report count mismatch!\r\n");
            return false;
        }

        // Verify timing intervals tracked 1000 us (1000Hz)
        if pipeline.min_interval_us() != 1000 || pipeline.max_interval_us() != 1000 {
            serial_write_str("[KNT-PANIC] 1000Hz interval tracking mismatch!\r\n");
            return false;
        }

        // Polling rate estimate should be ~1000Hz
        if (pipeline.current_polling_rate_hz() - 1000.0).abs() > 50.0 {
            serial_write_str("[KNT-PANIC] Polling rate estimate deviates from 1000Hz!\r\n");
            return false;
        }

        // Verify gyro aiming crosshair delta
        let (dx, dy) = pipeline.calculate_gyro_aiming(pipeline.last_state(), 0.001);
        if dx == 0.0 {
            serial_write_str("[KNT-PANIC] Gyro aiming crosshair delta calculation zero!\r\n");
            return false;
        }
        if dy != 0.0 {
            serial_write_str("[KNT-PANIC] Unexpected pitch crosshair delta!\r\n");
            return false;
        }

        // Test haptic rumble commands
        pipeline.set_haptics(HapticFeedback::new(200, 150));
        let haptics = pipeline.current_haptics();
        if haptics.strong_motor != 200 || haptics.weak_motor != 150 {
            serial_write_str("[KNT-PANIC] Haptic feedback setting mismatch!\r\n");
            return false;
        }
    }

    serial_write_str("[KNT-INPUT-IMU-PASS] 6-axis IMU sensor fusion and 1000Hz gamepad input pipeline verified\r\n");
    true
}

/// Comprehensive verification self-test suite for Gamepad Haptic Vibration Synthesizer
/// and Dual-Motor Rumble Controller.
pub fn test_gamepad_haptic_subsystem() -> bool {
    // 1. Verify vibration waveform synthesizer, ADSR envelopes, and gain scaling
    if !waveform::verify_waveform_synthesizer() {
        return false;
    }

    // 2. Verify dual-motor rumble controller, playback sequence queue, and timeline mixing
    if !haptic::verify_dual_motor_controller() {
        return false;
    }

    // 3. Verify USB HID and gamepad haptic report packet formatting
    if !haptic::verify_haptic_report_formatting() {
        return false;
    }

    // 4. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-HAPTIC-RUMBLE-PASS] Gamepad haptic waveform synthesizer and dual-motor rumble verified\r\n");

    true
}
