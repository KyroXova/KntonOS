//! Unified Handheld PC RGB LED Lighting Controller & Game State Aura Sync Engine.

pub mod rgb;

pub use rgb::*;

use crate::serial_write_str;

/// Comprehensive verification self-test suite for Handheld PC Multi-Zone RGB Lighting Controller
/// and Game State Aura Sync Engine.
///
/// Emits sentinel [KNT-LIGHTING-RGB-PASS] upon successful validation.
pub fn test_rgb_lighting_subsystem() -> bool {
    serial_write_str("[KNT-LED] Testing handheld multi-zone RGB lighting & Aura Sync engine...\r\n");

    // 1. Verify Rgb8 Color Mathematics, HSV Conversions, 2.2 Gamma Table, and Multi-Zone Layout
    if !verify_rgb_math_and_gamma() {
        serial_write_str("[KNT-PANIC] RGB color math / gamma table verification failed!\r\n");
        return false;
    }

    // 2. Verify Lighting Effect Generators (Breathing, Rainbow, Battery) and Aura Sync Synchronization
    if !verify_effects_and_aura_sync() {
        serial_write_str("[KNT-PANIC] Lighting effect generators / Aura Sync verification failed!\r\n");
        return false;
    }

    // 3. Verify Game-State Reactive Lighting Triggers and Hardware EC / I2C Register Mapping
    if !verify_game_state_reactive_and_ec_mapping() {
        serial_write_str("[KNT-PANIC] Game-state reactive lighting / EC register mapping verification failed!\r\n");
        return false;
    }

    serial_write_str("[KNT-LIGHTING-RGB-PASS] Handheld RGB lighting controller and Aura Sync verified\r\n");
    true
}
