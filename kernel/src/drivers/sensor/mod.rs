//! Sensor Drivers and Governors for Handheld Devices.

pub mod als;
pub mod brightness;

pub use als::*;
pub use brightness::*;

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
