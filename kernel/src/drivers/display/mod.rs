//! Display Drivers and Backlight Control Subsystems for KntonOS.

pub mod pwm;
pub mod backlight;
pub mod dp_alt;

pub use pwm::*;
pub use backlight::*;
pub use dp_alt::*;


use crate::serial_write_str;

/// Verification self-test suite for Handheld Backlight PWM Modulator & Flicker-Free DC Dimming Governor.
pub fn test_backlight_pwm_subsystem() -> bool {
    serial_write_str("[KNT-DISPLAY] Testing backlight PWM modulator & flicker-free DC dimming governor...\r\n");

    // 1. Verify Hardware PWM Frequency Synthesizer & Duty-Cycle Modulator
    if !verify_pwm_frequency_and_duty_cycle() {
        serial_write_str("[KNT-PANIC] Backlight PWM frequency synthesizer / modulator verification failed!\r\n");
        return false;
    }

    // 2. Verify Hybrid DC Dimming Transition Governor & CIE 1931 Perceptual Curve
    if !verify_hybrid_dc_dimming_and_cie1931() {
        serial_write_str("[KNT-PANIC] Hybrid DC dimming / CIE 1931 curve verification failed!\r\n");
        return false;
    }

    // 3. Verify Smooth Fade Pacing, Ambient Minimum Clamp, and OLED ABL Scaling
    if !verify_fade_pacing_ambient_clamp_and_oled_scaling() {
        serial_write_str("[KNT-PANIC] Fade pacing / ambient clamp / OLED scaling verification failed!\r\n");
        return false;
    }

    serial_write_str("[KNT-DISPLAY-PWM-PASS] Unified backlight PWM modulator and DC dimming governor verified\r\n");
    true
}
