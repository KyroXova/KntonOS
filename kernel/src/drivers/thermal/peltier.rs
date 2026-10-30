//! Unified Handheld PC Multi-Zone Thermocouple & Peltier Active Cooler Module Driver.
//!
//! Provides bare-metal thermal management for extreme TDP handheld gaming PCs featuring:
//! - Multi-zone thermocouple temperature sensing (APU junction, VRM, Battery, Cold Plate, Hot Plate).
//! - Thermoelectric Cooler (TEC / Peltier module) active cooling control.
//! - Closed-loop PID junction temperature controller.
//! - Dew-point condensation prevention engine to stop moisture buildup on cold plate.
//! - Synchronized Fan-TEC thermal curve with dynamic turbo cooling burst governor.

/// Hardware I2C / SMBus thermocouple ADC and TEC controller register definitions.
pub mod tec_registers {
    /// APU core junction thermocouple ADC register (0.1 °C per LSB).
    pub const REG_THERMO_APU_JUNCTION: u8 = 0x10;
    /// Power delivery VRM MOSFET thermocouple ADC register.
    pub const REG_THERMO_VRM: u8 = 0x12;
    /// Battery pack surface thermocouple ADC register.
    pub const REG_THERMO_BATTERY: u8 = 0x14;
    /// Peltier TEC cold plate thermocouple ADC register.
    pub const REG_THERMO_COLD_PLATE: u8 = 0x16;
    /// Peltier TEC hot plate thermocouple ADC register.
    pub const REG_THERMO_HOT_PLATE: u8 = 0x18;
    /// Ambient chassis air temperature sensor register.
    pub const REG_THERMO_AMBIENT: u8 = 0x1A;
    /// Ambient relative humidity sensor register (0.1% RH per LSB).
    pub const REG_HUMIDITY_AMBIENT: u8 = 0x1C;

    /// Peltier TEC buck-boost PWM duty cycle register (0..100%).
    pub const REG_TEC_PWM_DUTY: u8 = 0x20;
    /// Peltier TEC status and diagnostic fault flags register.
    pub const REG_TEC_STATUS: u8 = 0x22;
    /// High-RPM exhaust heatsink cooling fan PWM duty register (0..100%).
    pub const REG_FAN_PWM_DUTY: u8 = 0x24;

    // Status register bit flags
    pub const TEC_STATUS_ENABLED: u8 = 1 << 0;
    pub const TEC_STATUS_OVERTEMP_HOT_PLATE: u8 = 1 << 1;
    pub const TEC_STATUS_CONDENSATION_RISK: u8 = 1 << 2;
    pub const TEC_STATUS_TURBO_ACTIVE: u8 = 1 << 3;
}

/// Identifies an individual thermal sensing zone on the handheld device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalZone {
    /// APU silicon core junction.
    ApuJunction,
    /// VRM power stages and inductors.
    VrmMosfet,
    /// Dual-battery pack cells.
    BatteryPack,
    /// Thermoelectric cooler cold plate mating with APU heat spreader.
    TecColdPlate,
    /// Thermoelectric cooler hot plate mating with exhaust heatsink.
    TecHotPlate,
    /// Intake ambient air temperature.
    AmbientAir,
}

/// Multi-zone thermocouple telemetry snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MultiZoneThermalTelemetry {
    /// APU junction temperature in degrees Celsius (°C).
    pub apu_junction_c: f32,
    /// VRM MOSFET temperature in °C.
    pub vrm_mosfet_c: f32,
    /// Battery pack temperature in °C.
    pub battery_pack_c: f32,
    /// TEC cold plate temperature in °C.
    pub tec_cold_plate_c: f32,
    /// TEC hot plate temperature in °C.
    pub tec_hot_plate_c: f32,
    /// Ambient air temperature in °C.
    pub ambient_air_c: f32,
    /// Ambient relative humidity percentage (0.0 .. 100.0%).
    pub ambient_humidity_pct: f32,
}

impl MultiZoneThermalTelemetry {
    /// Creates a new telemetry snapshot with specified readings.
    pub const fn new(
        apu_junction_c: f32,
        vrm_mosfet_c: f32,
        battery_pack_c: f32,
        tec_cold_plate_c: f32,
        tec_hot_plate_c: f32,
        ambient_air_c: f32,
        ambient_humidity_pct: f32,
    ) -> Self {
        Self {
            apu_junction_c,
            vrm_mosfet_c,
            battery_pack_c,
            tec_cold_plate_c,
            tec_hot_plate_c,
            ambient_air_c,
            ambient_humidity_pct,
        }
    }

    /// Nominal idle conditions for handheld gaming PC.
    pub const fn nominal() -> Self {
        Self {
            apu_junction_c: 45.0,
            vrm_mosfet_c: 42.0,
            battery_pack_c: 30.0,
            tec_cold_plate_c: 32.0,
            tec_hot_plate_c: 40.0,
            ambient_air_c: 24.0,
            ambient_humidity_pct: 45.0,
        }
    }

    /// Retrieves temperature reading for a specific zone.
    #[inline(always)]
    pub fn reading_for_zone(&self, zone: ThermalZone) -> f32 {
        match zone {
            ThermalZone::ApuJunction => self.apu_junction_c,
            ThermalZone::VrmMosfet => self.vrm_mosfet_c,
            ThermalZone::BatteryPack => self.battery_pack_c,
            ThermalZone::TecColdPlate => self.tec_cold_plate_c,
            ThermalZone::TecHotPlate => self.tec_hot_plate_c,
            ThermalZone::AmbientAir => self.ambient_air_c,
        }
    }
}

/// Operational mode of the Thermoelectric Peltier Cooler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TecMode {
    /// Peltier module completely powered off (0% PWM duty).
    Disabled,
    /// Low-power eco cooling mode (e.g. 15% duty for desktop idle/light tasks).
    Eco,
    /// Closed-loop PID control holding cold plate / APU at target temperature setpoint.
    TargetTempPID,
    /// High-current aggressive cooling burst for transient heavy game loads.
    DynamicTurbo,
    /// Emergency failsafe: hot plate exceeded critical threshold, TEC shut off immediately.
    ThermalFailsafe,
}

/// Constants defining thermal and electrical operating boundaries.
pub const MAX_TEC_DUTY_PCT: u8 = 100;
pub const MIN_TEC_DUTY_PCT: u8 = 0;
pub const HOT_PLATE_CRITICAL_TEMP_C: f32 = 85.0;
pub const HOT_PLATE_WARNING_TEMP_C: f32 = 75.0;

/// Peltier Thermoelectric Cooler (TEC) active operational state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TecState {
    /// Active operating mode.
    pub mode: TecMode,
    /// Current PWM duty cycle percentage applied to TEC H-bridge / buck-boost (0..100%).
    pub duty_pct: u8,
    /// Target cold plate temperature setpoint in °C.
    pub cold_plate_target_c: f32,
    /// Estimated instantaneous power consumed by the Peltier module in Watts.
    pub power_watts: f32,
    /// Maximum allowed PWM duty limit (e.g. 100% on AC wall power, 60% on battery).
    pub max_duty_limit: u8,
    /// Set to true if thermal failsafe emergency shutdown was tripped.
    pub failsafe_tripped: bool,
}

impl TecState {
    /// Creates a new TEC state in default idle/disabled mode.
    pub const fn new() -> Self {
        Self {
            mode: TecMode::Disabled,
            duty_pct: 0,
            cold_plate_target_c: 25.0,
            power_watts: 0.0,
            max_duty_limit: 100,
            failsafe_tripped: false,
        }
    }

    /// Sets PWM duty cycle clamped within safe bounds [0, max_duty_limit].
    pub fn set_duty(&mut self, duty: u8) {
        if self.failsafe_tripped {
            self.duty_pct = 0;
            self.power_watts = 0.0;
            return;
        }
        let clamped = duty.min(self.max_duty_limit);
        self.duty_pct = clamped;
        // Nominal 25W Peltier at 12V 100% duty: power is roughly proportional to duty^2
        let ratio = clamped as f32 / 100.0;
        self.power_watts = ratio * ratio * 25.0;
    }

    /// Triggers immediate emergency shutdown due to hot plate over-temperature.
    pub fn emergency_shutdown(&mut self) {
        self.mode = TecMode::ThermalFailsafe;
        self.duty_pct = 0;
        self.power_watts = 0.0;
        self.failsafe_tripped = true;
    }

    /// Clears emergency shutdown condition once hot plate drops to safe levels.
    pub fn reset_failsafe(&mut self) {
        self.failsafe_tripped = false;
        self.mode = TecMode::Disabled;
    }
}

/// Verification helper for Commit 1 structures, registers, and TEC state transitions.
pub fn verify_thermal_registers_and_tec_state() -> bool {
    // 1. Verify Telemetry Zone Readings
    let telemetry = MultiZoneThermalTelemetry::new(65.5, 58.0, 34.2, 22.0, 52.3, 23.5, 50.0);
    if (telemetry.reading_for_zone(ThermalZone::ApuJunction) - 65.5).abs() > 0.01 {
        return false;
    }
    if (telemetry.reading_for_zone(ThermalZone::TecColdPlate) - 22.0).abs() > 0.01 {
        return false;
    }
    if (telemetry.reading_for_zone(ThermalZone::TecHotPlate) - 52.3).abs() > 0.01 {
        return false;
    }

    // 2. Verify TEC State Initialization & PWM Clamping
    let mut tec = TecState::new();
    if tec.mode != TecMode::Disabled || tec.duty_pct != 0 || tec.power_watts != 0.0 {
        return false;
    }

    // Set duty under normal limit
    tec.set_duty(60);
    if tec.duty_pct != 60 || tec.power_watts <= 0.0 {
        return false;
    }

    // Limit clamping (e.g. battery mode limit 40%)
    tec.max_duty_limit = 40;
    tec.set_duty(80);
    if tec.duty_pct != 40 {
        return false;
    }

    // Emergency failsafe shutdown
    tec.emergency_shutdown();
    if tec.mode != TecMode::ThermalFailsafe || tec.duty_pct != 0 || !tec.failsafe_tripped {
        return false;
    }

    // Setting duty while tripped must have no effect
    tec.set_duty(50);
    if tec.duty_pct != 0 {
        return false;
    }

    // Reset failsafe
    tec.reset_failsafe();
    if tec.failsafe_tripped || tec.mode != TecMode::Disabled {
        return false;
    }

    true
}