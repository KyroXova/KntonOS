//! Core Junction Thermal Monitoring & Closed-Loop PID Dynamic Power Governor.
//! Implements Digital Thermal Sensor (DTS) monitoring, PROCHOT / thermal throttling detection,

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

// Closed-Loop PID Dynamic TDP Power Governor (5W - 30W) & Frequency Scaling

pub const MIN_TDP_LIMIT_MW: u32 = 5_000;    // 5 Watts minimum
pub const MAX_TDP_LIMIT_MW: u32 = 30_000;   // 30 Watts maximum
pub const DEFAULT_BALANCED_TDP_MW: u32 = 15_000;   // 15 Watts
pub const DEFAULT_POWER_SAVER_TDP_MW: u32 = 7_500; // 7.5 Watts
pub const DEFAULT_PERFORMANCE_TDP_MW: u32 = 28_000;// 28 Watts

/// Power Governor Operating Profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GovernorProfile {
    PowerSaver,
    Balanced,
    Performance,
    Custom,
}

/// Action output generated by the closed-loop governor after each control step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GovernorAction {
    /// Commanded Package PL1 limit in milliwatts (5,000 mW to 30,000 mW).
    pub target_pl1_mw: u32,
    /// Commanded CPU throttling duty cycle (0 = unthrottled, 100 = maximum throttling).
    pub throttle_duty_percent: u8,
    /// True if an emergency thermal threshold override was triggered.
    pub emergency_thermal_throttle: bool,
}

/// Closed-Loop PID Dynamic Power Governor.
#[derive(Debug, Clone)]
pub struct PidPowerGovernor {
    pub profile: GovernorProfile,
    pub target_tdp_mw: u32,
    pub kp: f32,
    pub ki: f32,
    pub kd: f32,
    pub integral_accum: f32,
    pub prev_error: f32,
    pub current_pl1_mw: u32,
    pub current_throttle_duty: u8,
}

impl PidPowerGovernor {
    /// Constructs a PID governor initialized for the chosen profile.
    pub fn new(profile: GovernorProfile) -> Self {
        let (target, kp, ki, kd) = match profile {
            GovernorProfile::PowerSaver => (DEFAULT_POWER_SAVER_TDP_MW, 0.8f32, 0.08f32, 0.1f32),
            GovernorProfile::Balanced => (DEFAULT_BALANCED_TDP_MW, 0.5f32, 0.05f32, 0.1f32),
            GovernorProfile::Performance => (DEFAULT_PERFORMANCE_TDP_MW, 0.4f32, 0.03f32, 0.05f32),
            GovernorProfile::Custom => (DEFAULT_BALANCED_TDP_MW, 0.5f32, 0.05f32, 0.1f32),
        };

        Self {
            profile,
            target_tdp_mw: target,
            kp,
            ki,
            kd,
            integral_accum: 0.0,
            prev_error: 0.0,
            current_pl1_mw: target,
            current_throttle_duty: 0,
        }
    }

    /// Updates governor profile and resets integral accumulator.
    pub fn set_profile(&mut self, profile: GovernorProfile) {
        self.profile = profile;
        match profile {
            GovernorProfile::PowerSaver => self.target_tdp_mw = DEFAULT_POWER_SAVER_TDP_MW,
            GovernorProfile::Balanced => self.target_tdp_mw = DEFAULT_BALANCED_TDP_MW,
            GovernorProfile::Performance => self.target_tdp_mw = DEFAULT_PERFORMANCE_TDP_MW,
            GovernorProfile::Custom => {}
        }
        self.integral_accum = 0.0;
        self.prev_error = 0.0;
    }

    /// Sets custom target TDP bounded strictly between 5W and 30W.
    pub fn set_target_tdp(&mut self, target_mw: u32) {
        self.target_tdp_mw = target_mw.clamp(MIN_TDP_LIMIT_MW, MAX_TDP_LIMIT_MW);
    }

    /// Executes one discrete PID control iteration given measured power, temperature, and delta time.
    pub fn step(&mut self, current_power_mw: u32, current_temp_c: i32, dt_ms: u32) -> GovernorAction {
        // 1. Emergency thermal safety check
        if current_temp_c >= THERMAL_THROTTLE_THRESHOLD_CELSIUS {
            self.current_pl1_mw = MIN_TDP_LIMIT_MW;
            self.current_throttle_duty = 75; // Aggressive throttle duty cycle
            self.integral_accum = 0.0;
            self.prev_error = 0.0;
            return GovernorAction {
                target_pl1_mw: MIN_TDP_LIMIT_MW,
                throttle_duty_percent: 75,
                emergency_thermal_throttle: true,
            };
        }

        // 2. Calculate error: positive = under target (can boost), negative = over budget (must throttle)
        let error = (self.target_tdp_mw as f32) - (current_power_mw as f32);
        let dt = ((dt_ms.max(1)) as f32) / 1000.0;

        // Proportional term
        let p_term = self.kp * error;

        // Integral term with anti-windup clamping (-10,000 to +10,000)
        self.integral_accum = (self.integral_accum + self.ki * error * dt).clamp(-10_000.0, 10_000.0);
        let i_term = self.integral_accum;

        // Derivative term
        let d_term = self.kd * (error - self.prev_error) / dt;
        self.prev_error = error;

        let output = p_term + i_term + d_term;

        // 3. Modulate commanded PL1 TDP limit (5W - 30W)
        let new_pl1 = ((self.target_tdp_mw as f32) + output).clamp(
            MIN_TDP_LIMIT_MW as f32,
            MAX_TDP_LIMIT_MW as f32,
        );
        self.current_pl1_mw = new_pl1 as u32;

        // 4. Modulate throttling duty cycle
        // If power exceeds target by > 10%, apply duty-cycle throttling proportionally
        self.current_throttle_duty = if error < 0.0 {
            let over_watts = (-error) / 1000.0;
            ((over_watts * 5.0) as u8).min(75)
        } else {
            0
        };

        GovernorAction {
            target_pl1_mw: self.current_pl1_mw,
            throttle_duty_percent: self.current_throttle_duty,
            emergency_thermal_throttle: false,
        }
    }
}

/// Verification self-test for closed-loop PID governor, TDP bounds (5W-30W), and thermal overrides.
pub fn verify_pid_power_governor() -> bool {
    let mut gov = PidPowerGovernor::new(GovernorProfile::Balanced);
    if gov.target_tdp_mw != DEFAULT_BALANCED_TDP_MW {
        return false;
    }

    // 1. Test power overshoot: Measured 25W on a 15W target (10W over budget)
    let action1 = gov.step(25_000, 60, 100);
    // Commanded PL1 must decrease below target (15W) to shed load
    if action1.target_pl1_mw >= DEFAULT_BALANCED_TDP_MW {
        return false;
    }
    // Must engage throttling duty cycle to counteract excess heat
    if action1.throttle_duty_percent == 0 || action1.emergency_thermal_throttle {
        return false;
    }

    // 2. Test power undershoot: Measured 8W on a 15W target (7W under budget)
    let action2 = gov.step(8_000, 50, 100);
    // Commanded PL1 should be higher to allow more compute throughput
    if action2.target_pl1_mw <= action1.target_pl1_mw {
        return false;
    }
    if action2.throttle_duty_percent != 0 {
        return false;
    }

    // 3. Test clamping bounds (5,000 mW to 30,000 mW)
    gov.set_target_tdp(2_000); // Below 5W minimum
    if gov.target_tdp_mw != MIN_TDP_LIMIT_MW {
        return false;
    }
    gov.set_target_tdp(50_000); // Above 30W maximum
    if gov.target_tdp_mw != MAX_TDP_LIMIT_MW {
        return false;
    }

    // 4. Test emergency thermal trip point override
    let emergency_action = gov.step(12_000, 98, 100); // 98°C exceeds 95°C threshold
    if !emergency_action.emergency_thermal_throttle {
        return false;
    }
    if emergency_action.target_pl1_mw != MIN_TDP_LIMIT_MW || emergency_action.throttle_duty_percent != 75 {
        return false;
    }

    true
}
