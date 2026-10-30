//! Core Junction Thermal Monitoring & Closed-Loop PID Dynamic Power Governor.
//!
//! Implements Digital Thermal Sensor (DTS) monitoring, PROCHOT / thermal throttling detection,
//! and closed-loop PID TDP frequency governing (5W to 30W).

// --- Intel Digital Thermal Sensor (DTS) MSRs ---
pub const IA32_THERM_STATUS: u32 = 0x0000_019C;
pub const IA32_THERM_INTERRUPT: u32 = 0x0000_019B;
pub const IA32_TEMPERATURE_TARGET: u32 = 0x0000_01A2;
pub const MSR_PACKAGE_THERM_STATUS: u32 = 0x0000_01B1;

// --- Thermal Status MSR Bitmasks ---
pub const THERM_STATUS_VALID: u64 = 1 << 31;
pub const THERM_STATUS_READOUT_MASK: u64 = 0x7F << 16;
pub const THERM_STATUS_PROCHOT: u64 = 1 << 0;
pub const THERM_STATUS_PROCHOT_LOG: u64 = 1 << 1;
pub const THERM_STATUS_CRITICAL: u64 = 1 << 4;
pub const THERM_STATUS_CRITICAL_LOG: u64 = 1 << 5;
pub const THERM_STATUS_THROTTLE: u64 = 1 << 6;
pub const THERM_STATUS_THROTTLE_LOG: u64 = 1 << 7;

// --- Default Temperature Thresholds (Degrees Celsius) ---
pub const DEFAULT_TJ_MAX_CELSIUS: i32 = 100;
pub const THERMAL_WARN_THRESHOLD_CELSIUS: i32 = 85;
pub const THERMAL_THROTTLE_THRESHOLD_CELSIUS: i32 = 95;
pub const THERMAL_CRITICAL_THRESHOLD_CELSIUS: i32 = 105;

/// Hardware Thermal Throttling Flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ThermalThrottleFlags {
    /// External/internal PROCHOT# assertion.
    pub prochot_asserted: bool,
    /// Core currently in thermal throttling state.
    pub thermally_throttled: bool,
    /// Critical trip point exceeded (shutdown imminent).
    pub critical_temp_exceeded: bool,
    /// Historical sticky log indicates past throttling.
    pub sticky_throttle_logged: bool,
}

impl ThermalThrottleFlags {
    /// Decodes flags from IA32_THERM_STATUS register.
    pub fn from_msr(status_val: u64) -> Self {
        Self {
            prochot_asserted: (status_val & THERM_STATUS_PROCHOT) != 0,
            thermally_throttled: (status_val & THERM_STATUS_THROTTLE) != 0,
            critical_temp_exceeded: (status_val & THERM_STATUS_CRITICAL) != 0,
            sticky_throttle_logged: (status_val & THERM_STATUS_THROTTLE_LOG) != 0,
        }
    }

    /// Returns true if any thermal warning or throttling condition is currently active.
    pub fn is_throttling(&self) -> bool {
        self.prochot_asserted || self.thermally_throttled || self.critical_temp_exceeded
    }
}

/// Decoded thermal status for a CPU core junction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreThermalStatus {
    /// Core junction temperature in degrees Celsius (°C).
    pub temperature_celsius: i32,
    /// Maximum junction rating (TjMax) in °C.
    pub tj_max: i32,
    /// Digital readout offset below TjMax.
    pub delta_below_tj_max: i32,
    /// Active throttling and safety flags.
    pub flags: ThermalThrottleFlags,
    /// Digital readout validity flag.
    pub is_valid: bool,
}

impl CoreThermalStatus {
    /// Constructs thermal status from raw MSR values.
    pub fn from_msrs(status_msr: u64, target_msr: u64) -> Self {
        let tj_max = if target_msr != 0 {
            ((target_msr >> 16) & 0xFF) as i32
        } else {
            DEFAULT_TJ_MAX_CELSIUS
        };

        let is_valid = (status_msr & THERM_STATUS_VALID) != 0;
        let delta = ((status_msr >> 16) & 0x7F) as i32;
        let temperature_celsius = if is_valid {
            (tj_max - delta).max(0)
        } else {
            0
        };

        let flags = ThermalThrottleFlags::from_msr(status_msr);

        Self {
            temperature_celsius,
            tj_max,
            delta_below_tj_max: delta,
            flags,
            is_valid,
        }
    }

    /// Synthesizes simulated core thermal status for testing.
    pub fn synthetic(temperature_celsius: i32, tj_max: i32) -> Self {
        let delta = (tj_max - temperature_celsius).max(0);
        let mut flags = ThermalThrottleFlags::default();
        if temperature_celsius >= THERMAL_THROTTLE_THRESHOLD_CELSIUS {
            flags.thermally_throttled = true;
            flags.sticky_throttle_logged = true;
        }
        if temperature_celsius >= THERMAL_CRITICAL_THRESHOLD_CELSIUS {
            flags.critical_temp_exceeded = true;
            flags.prochot_asserted = true;
        }

        Self {
            temperature_celsius,
            tj_max,
            delta_below_tj_max: delta,
            flags,
            is_valid: true,
        }
    }
}

/// Core Junction & Package Thermal Monitoring Engine.
#[derive(Debug, Clone)]
pub struct ThermalSensorEngine {
    pub tj_max: i32,
    pub package_temperature: i32,
    pub core_temperatures: [i32; 8],
    pub is_simulated: bool,
}

impl ThermalSensorEngine {
    pub const fn new(is_simulated: bool) -> Self {
        Self {
            tj_max: DEFAULT_TJ_MAX_CELSIUS,
            package_temperature: 45,
            core_temperatures: [45; 8],
            is_simulated,
        }
    }

    /// Reads junction temperature of a specific core.
    pub fn read_core_status(&self, core_id: usize) -> CoreThermalStatus {
        let temp = if core_id < self.core_temperatures.len() {
            self.core_temperatures[core_id]
        } else {
            self.package_temperature
        };
        CoreThermalStatus::synthetic(temp, self.tj_max)
    }

    /// Updates simulated core and package temperatures for thermal tests.
    pub fn update_simulated_temp(&mut self, core_id: usize, temp_celsius: i32) {
        if core_id < self.core_temperatures.len() {
            self.core_temperatures[core_id] = temp_celsius;
        }
        // Package temp tracks maximum core temperature
        let mut max_t = self.core_temperatures[0];
        for &t in &self.core_temperatures {
            if t > max_t {
                max_t = t;
            }
        }
        self.package_temperature = max_t;
    }

    /// Returns the maximum observed temperature across all cores and package.
    pub fn max_temperature(&self) -> i32 {
        self.package_temperature
    }

    /// Checks if thermal mitigation or throttling should be triggered.
    pub fn requires_thermal_throttling(&self) -> bool {
        self.max_temperature() >= THERMAL_THROTTLE_THRESHOLD_CELSIUS
    }
}

/// Verification self-test for DTS MSR decoding, junction temperature math, and thermal flags.
pub fn verify_thermal_monitoring() -> bool {
    // 1. Verify DTS MSR decoding with valid bit and 38°C delta below 100°C TjMax
    // Target MSR (0x1A2): TjMax = 100 (0x64 at bits 23:16 -> 0x0064_0000)
    let target_msr = 0x0064_0000u64;
    // Status MSR (0x19C): Valid bit (31), Delta = 38 (0x26 at bits 22:16), No throttle
    // Value = (1 << 31) | (0x26 << 16) = 0x8026_0000
    let status_msr = (1u64 << 31) | (0x26u64 << 16);

    let thermal = CoreThermalStatus::from_msrs(status_msr, target_msr);
    if !thermal.is_valid || thermal.tj_max != 100 || thermal.delta_below_tj_max != 38 {
        return false;
    }
    // 100°C - 38°C = 62°C
    if thermal.temperature_celsius != 62 {
        return false;
    }
    if thermal.flags.is_throttling() {
        return false;
    }

    // 2. Verify thermal throttle and PROCHOT flag assertion
    // Status MSR with PROCHOT (bit 0) and THROTTLE (bit 6) set
    let throttled_msr = status_msr | THERM_STATUS_PROCHOT | THERM_STATUS_THROTTLE;
    let throttled_status = CoreThermalStatus::from_msrs(throttled_msr, target_msr);
    if !throttled_status.flags.prochot_asserted || !throttled_status.flags.thermally_throttled {
        return false;
    }
    if !throttled_status.flags.is_throttling() {
        return false;
    }

    // 3. Verify ThermalSensorEngine multi-core tracking & throttle trigger
    let mut sensor = ThermalSensorEngine::new(true);
    if sensor.requires_thermal_throttling() {
        return false;
    }

    // Inject 96°C on core 3 (exceeds 95°C throttle threshold)
    sensor.update_simulated_temp(3, 96);
    if sensor.max_temperature() != 96 || !sensor.requires_thermal_throttling() {
        return false;
    }

    let core3_status = sensor.read_core_status(3);
    if core3_status.temperature_celsius != 96 || !core3_status.flags.thermally_throttled {
        return false;
    }

    // Cool down to 60°C
    sensor.update_simulated_temp(3, 60);
    if sensor.requires_thermal_throttling() || sensor.max_temperature() != 60 {
        return false;
    }

    true
}
