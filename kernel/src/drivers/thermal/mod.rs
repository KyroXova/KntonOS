//! Thermal Management & Thermoelectric Cooling Subsystems.

pub mod peltier;

pub use peltier::*;

/// Verification self-test suite for Handheld PC Multi-Zone Thermocouple & Peltier Active Cooler Module.
pub fn test_peltier_active_cooler_subsystem() -> bool {
    crate::serial_write_str("[KNT-THERMAL] Testing multi-zone thermocouple & Peltier active cooler module...\r\n");

    // 1. Verify Multi-Zone Thermocouple Telemetry, Registers, and TEC State
    if !verify_thermal_registers_and_tec_state() {
        crate::serial_write_str("[KNT-PANIC] Thermal registers and TEC state verification failed!\r\n");
        return false;
    }

    // 2. Verify Closed-Loop PID Controller and Dew-Point Condensation Prevention
    if !verify_pid_and_dew_point_prevention() {
        crate::serial_write_str("[KNT-PANIC] Thermal PID and dew-point condensation prevention failed!\r\n");
        return false;
    }

    // 3. Verify Dynamic Turbo Cooling Burst Governor and Synchronized Fan Curve
    if !verify_turbo_burst_and_synchronized_fan_curve() {
        crate::serial_write_str("[KNT-PANIC] Turbo cooling burst and synchronized fan curve failed!\r\n");
        return false;
    }

    // 4. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-THERMAL-PELTIER-PASS] Multi-zone thermocouple and Peltier active cooler verified\r\n");
    true
}
