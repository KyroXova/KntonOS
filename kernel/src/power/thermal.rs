//! Multi-Zone Thermal Temperature Aggregator & Acoustic Pacing Governor.
//!
//! Collects thermal sensor readings across handheld physical zones:
//! - CPU Package
//! - GPU Core
//! - Battery Pack
//! - Chassis / Skin Touch Surface
//!
//! Computes a weighted composite thermal index and controls fan acoustics across
//! operational modes: Quiet, Balanced, and Turbo (Performance).

use crate::power::fan::{DampedFanController, MultiPointFanCurve};

// -----------------------------------------------------------------------------
// Thermal Zones & Weighted Sensor Telemetry
// -----------------------------------------------------------------------------

/// Multi-zone temperature readings in degrees Celsius.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultiZoneTemperatures {
    /// CPU package temperature (°C).
    pub cpu_pkg_celsius: u8,
    /// GPU core temperature (°C).
    pub gpu_core_celsius: u8,
    /// Battery pack temperature (°C).
    pub battery_celsius: u8,
    /// Chassis / skin surface contact temperature (°C).
    pub chassis_skin_celsius: u8,
}

impl MultiZoneTemperatures {
    pub const fn new(cpu: u8, gpu: u8, battery: u8, chassis: u8) -> Self {
        Self {
            cpu_pkg_celsius: cpu,
            gpu_core_celsius: gpu,
            battery_celsius: battery,
            chassis_skin_celsius: chassis,
        }
    }

    /// Peak temperature across all active thermal zones.
    pub fn peak_temperature(&self) -> u8 {
        self.cpu_pkg_celsius
            .max(self.gpu_core_celsius)
            .max(self.battery_celsius)
            .max(self.chassis_skin_celsius)
    }

    /// Computes a weighted thermal index taking skin touch comfort and silicon heat into account:
    /// - CPU Weight: 35%
    /// - GPU Weight: 35%
    /// - Battery Weight: 10%
    /// - Chassis Skin Weight: 20%
    pub fn weighted_index(&self) -> u8 {
        let weighted = (self.cpu_pkg_celsius as u32 * 35
            + self.gpu_core_celsius as u32 * 35
            + self.battery_celsius as u32 * 10
            + self.chassis_skin_celsius as u32 * 20)
            / 100;
        weighted.min(255) as u8
    }
}

// -----------------------------------------------------------------------------
// Acoustic Pacing Governor Profiles
// -----------------------------------------------------------------------------

/// Acoustic governor operating profiles for handheld gaming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcousticGovernorMode {
    /// Quiet Mode: Prioritizes low fan acoustics, caps fan noise below 60% unless critical.
    Quiet,
    /// Balanced Mode: Default handheld curve balancing temperature and noise.
    Balanced,
    /// Turbo / Performance Mode: Aggressive fan curve targeting sub-70°C silicon temperatures.
    Turbo,
}

/// Unified Handheld Thermal & Fan Profile Controller.
#[derive(Debug, Clone)]
pub struct HandheldThermalGovernor {
    pub mode: AcousticGovernorMode,
    pub fan_controller: DampedFanController,
    pub last_telemetry: MultiZoneTemperatures,
    pub max_audible_cap_percent: u8,
    pub skin_temp_limit_celsius: u8,
}

impl HandheldThermalGovernor {
    pub fn new(mode: AcousticGovernorMode) -> Self {
        let curve = match mode {
            AcousticGovernorMode::Quiet => MultiPointFanCurve::quiet_handheld_preset(),
            AcousticGovernorMode::Balanced => MultiPointFanCurve::balanced_handheld_preset(),
            AcousticGovernorMode::Turbo => {
                let mut c = MultiPointFanCurve::new(crate::power::fan::CurveInterpolationMode::Linear);
                c.add_point(crate::power::fan::FanCurvePoint::new(35, 20));
                c.add_point(crate::power::fan::FanCurvePoint::new(45, 40));
                c.add_point(crate::power::fan::FanCurvePoint::new(60, 70));
                c.add_point(crate::power::fan::FanCurvePoint::new(75, 90));
                c.add_point(crate::power::fan::FanCurvePoint::new(85, 100));
                c
            }
        };

        let max_audible_cap_percent = match mode {
            AcousticGovernorMode::Quiet => 60,
            _ => 100,
        };

        Self {
            mode,
            fan_controller: DampedFanController::new(curve),
            last_telemetry: MultiZoneTemperatures::new(40, 40, 30, 32),
            max_audible_cap_percent,
            skin_temp_limit_celsius: 48, // 48°C skin touch comfort ceiling
        }
    }

    /// Evaluates multi-zone temperatures and computes the output PWM percentage.
    pub fn step(&mut self, telemetry: MultiZoneTemperatures, elapsed_ms: u32) -> u8 {
        self.last_telemetry = telemetry;

        // Effective temperature is the maximum of weighted index, peak silicon temperature,
        // or boosted temperature if skin touch comfort is violated.
        let weighted = telemetry.weighted_index();
        let peak = telemetry.peak_temperature();

        // If skin temperature exceeds comfort threshold (48°C), bias temperature upwards by 15°C
        let skin_boost = if telemetry.chassis_skin_celsius > self.skin_temp_limit_celsius {
            (telemetry.chassis_skin_celsius - self.skin_temp_limit_celsius) * 3
        } else {
            0
        };

        let effective_temp = weighted.max(peak).saturating_add(skin_boost);

        let mut raw_pwm = self.fan_controller.update(effective_temp, elapsed_ms);

        // Apply acoustic mode cap unless silicon is in danger (peak >= 85°C)
        if self.mode == AcousticGovernorMode::Quiet && peak < 85 && effective_temp < 90 {
            raw_pwm = raw_pwm.min(self.max_audible_cap_percent);
        }

        raw_pwm
    }
}

// -----------------------------------------------------------------------------
// Unit Tests & Verification
// -----------------------------------------------------------------------------

/// Unit test verifying multi-zone temperature aggregation and acoustic pacing governor.
pub fn verify_thermal_governor_and_zones() -> bool {
    // 1. Verify weighted thermal index calculation
    let zones = MultiZoneTemperatures::new(70, 65, 35, 40);
    // (70*35 + 65*35 + 35*10 + 40*20) / 100 = (2450 + 2275 + 350 + 800) / 100 = 5875 / 100 = 58°C
    if zones.weighted_index() != 58 {
        return false;
    }
    if zones.peak_temperature() != 70 {
        return false;
    }

    // 2. Verify Quiet Mode acoustic cap (below 85°C silicon, capped at 60%)
    let mut quiet_gov = HandheldThermalGovernor::new(AcousticGovernorMode::Quiet);
    // Steady state at 75°C over 5 seconds
    let pwm_quiet = quiet_gov.step(MultiZoneTemperatures::new(75, 75, 38, 42), 5000);
    if pwm_quiet > 60 {
        return false; // Acoustic cap respected
    }

    // 3. Verify Turbo Mode aggressive cooling (reaches 90%+ at 75°C)
    let mut turbo_gov = HandheldThermalGovernor::new(AcousticGovernorMode::Turbo);
    let pwm_turbo = turbo_gov.step(MultiZoneTemperatures::new(75, 75, 38, 42), 5000);
    if pwm_turbo < 80 {
        return false; // Turbo profile provides strong cooling
    }

    // 4. Verify Skin Touch Limit protection
    // High chassis skin temperature (52°C > 48°C) triggers accelerated fan response
    let mut balanced_gov = HandheldThermalGovernor::new(AcousticGovernorMode::Balanced);
    let pwm_skin = balanced_gov.step(MultiZoneTemperatures::new(55, 55, 35, 52), 2000);
    if pwm_skin == 0 {
        return false;
    }

    true
}
