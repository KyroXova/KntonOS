//! Unified Handheld PC Multi-Zone Thermocouple & Peltier Active Cooler Module Driver.

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
/// Safety margin in °C maintained strictly above ambient dew point to avoid condensation.
pub const DEW_POINT_SAFETY_MARGIN_C: f32 = 3.0;

/// Approximates ambient air dew point from ambient temperature and relative humidity.
#[inline]
pub fn calculate_dew_point_c(ambient_temp_c: f32, relative_humidity_pct: f32) -> f32 {
    let rh_clamped = relative_humidity_pct.clamp(1.0, 100.0);
    ambient_temp_c - ((100.0 - rh_clamped) / 5.0)
}

/// Evaluates whether the Peltier cold plate is at risk of condensation.
pub fn evaluate_condensation_risk(telemetry: &MultiZoneThermalTelemetry) -> (bool, f32) {
    let dew_point = calculate_dew_point_c(telemetry.ambient_air_c, telemetry.ambient_humidity_pct);
    let min_safe_temp = dew_point + DEW_POINT_SAFETY_MARGIN_C;
    let is_at_risk = telemetry.tec_cold_plate_c <= min_safe_temp;
    (is_at_risk, min_safe_temp)
}

/// Closed-loop Proportional-Integral-Derivative (PID) thermal controller for the TEC module.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TecPidController {
    /// Proportional gain factor.
    pub kp: f32,
    /// Integral gain factor.
    pub ki: f32,
    /// Derivative gain factor.
    pub kd: f32,
    /// Accumulated integral error.
    pub integral_accum: f32,
    /// Previous loop error for derivative calculation.
    pub previous_error: f32,
    /// Active target temperature setpoint in °C.
    pub target_temp_c: f32,
    /// Anti-windup integral clamp bound (+/-).
    pub integral_windup_limit: f32,
}

impl TecPidController {
    /// Creates a new tuned PID controller instance.
    pub const fn new(kp: f32, ki: f32, kd: f32, target_temp_c: f32) -> Self {
        Self {
            kp,
            ki,
            kd,
            integral_accum: 0.0,
            previous_error: 0.0,
            target_temp_c,
            integral_windup_limit: 40.0,
        }
    }

    /// Resets historical integral and derivative states.
    pub fn reset(&mut self) {
        self.integral_accum = 0.0;
        self.previous_error = 0.0;
    }

    /// Updates the PID loop given current temperature and delta time (dt in seconds).
    pub fn update(
        &mut self,
        current_temp_c: f32,
        dt_seconds: f32,
        min_safe_temp_c: f32,
    ) -> u8 {
        // Enforce condensation avoidance: cold plate target cannot be set below safe dew point limit
        let effective_target = if self.target_temp_c < min_safe_temp_c {
            min_safe_temp_c
        } else {
            self.target_temp_c
        };

        let dt = if dt_seconds <= 0.001 { 0.01 } else { dt_seconds };
        // Error: positive when hotter than target (demands cooling increase)
        let error = current_temp_c - effective_target;

        // Proportional term
        let p = self.kp * error;

        // Integral term with anti-windup clamping
        self.integral_accum = (self.integral_accum + error * dt)
            .clamp(-self.integral_windup_limit, self.integral_windup_limit);
        let i = self.ki * self.integral_accum;

        // Derivative term
        let d = self.kd * ((error - self.previous_error) / dt);
        self.previous_error = error;

        let output = p + i + d;
        output.clamp(0.0, 100.0) as u8
    }
}

/// Verification helper for Commit 2 PID control and dew point condensation prevention.
pub fn verify_pid_and_dew_point_prevention() -> bool {
    // 1. Verify Dew Point Calculation
    // Ambient 25.0°C, 50.0% RH -> Dew point = 25 - (50/5) = 15.0°C
    let dew = calculate_dew_point_c(25.0, 50.0);
    if (dew - 15.0).abs() > 0.01 {
        return false;
    }

    // 2. Verify Condensation Risk Evaluation
    // Telemetry with cold plate at 16.0°C (below min safe 15 + 3 = 18.0°C) -> Risk true
    let risk_telem = MultiZoneThermalTelemetry::new(50.0, 45.0, 30.0, 16.0, 48.0, 25.0, 50.0);
    let (is_risk, min_safe) = evaluate_condensation_risk(&risk_telem);
    if !is_risk || (min_safe - 18.0).abs() > 0.01 {
        return false;
    }

    // Safe cold plate at 22.0°C (> 18.0°C) -> Risk false
    let safe_telem = MultiZoneThermalTelemetry::new(50.0, 45.0, 30.0, 22.0, 48.0, 25.0, 50.0);
    let (is_safe_risk, _) = evaluate_condensation_risk(&safe_telem);
    if is_safe_risk {
        return false;
    }

    // 3. Verify PID Controller Response
    let mut pid = TecPidController::new(2.0, 0.1, 0.5, 30.0);

    // Initial state: current temp 60.0°C > target 30.0°C -> heavy cooling demand
    let duty_hot = pid.update(60.0, 0.1, 15.0);
    if duty_hot < 60 {
        return false;
    }

    // Stabilized state: temp at 30.0°C -> duty drops
    for _ in 0..10 {
        pid.update(30.0, 0.1, 15.0);
    }
    let duty_stable = pid.update(30.0, 0.1, 15.0);
    if duty_stable > 10 {
        return false;
    }

    // Target below condensation safety limit (e.g. target requested 10.0°C, safe is 18.0°C)
    let mut pid_cond = TecPidController::new(2.0, 0.1, 0.5, 10.0);
    let _ = pid_cond.update(18.0, 0.1, 18.0);
    // Since current temp is at min_safe (18.0°C), error against clamped effective target should be ~0
    let duty_cond_clamped = pid_cond.update(18.0, 0.1, 18.0);
    if duty_cond_clamped > 10 {
        return false;
    }

    true
}
/// Dynamic Turbo Cooling Burst Governor configuration.
pub const TURBO_BURST_MAX_TICKS: u32 = 100; // 10 seconds at 10 Hz
pub const TURBO_BURST_COOLDOWN_TICKS: u32 = 50; // 5 seconds cooldown
pub const TURBO_GRADIENT_TRIGGER_C_PER_SEC: f32 = 2.0;
pub const TURBO_TEMP_THRESHOLD_C: f32 = 75.0;

/// Computes the synchronized heatsink exhaust fan duty cycle.
pub fn calculate_synchronized_fan_duty(
    apu_temp_c: f32,
    tec_duty_pct: u8,
    hot_plate_temp_c: f32,
) -> u8 {
    // 1. Base fan curve from APU junction temperature
    let base_fan = if apu_temp_c < 45.0 {
        20
    } else if apu_temp_c < 60.0 {
        40
    } else if apu_temp_c < 70.0 {
        65
    } else if apu_temp_c < 80.0 {
        85
    } else {
        100
    };

    // 2. TEC lead synchronization: Fan must lead or track TEC duty + 10%
    let tec_lead_fan = if tec_duty_pct > 0 {
        (tec_duty_pct as u16 + 10).min(100) as u8
    } else {
        0
    };

    let mut final_fan = base_fan.max(tec_lead_fan);

    // 3. Hot plate safety override: If hot plate exceeds warning threshold (75°C), force 100% fan
    if hot_plate_temp_c >= HOT_PLATE_WARNING_TEMP_C {
        final_fan = 100;
    }

    final_fan
}

/// Unified Peltier Active Cooler Manager coordinating telemetry, PID, burst governor, and fan curve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PeltierCoolerManager {
    /// Active TEC operational state.
    pub tec: TecState,
    /// Closed-loop PID controller.
    pub pid: TecPidController,
    /// Dynamic turbo burst active flag.
    pub turbo_active: bool,
    /// Remaining ticks for active burst.
    pub burst_ticks: u32,
    /// Remaining cooldown ticks before next burst allowed.
    pub cooldown_ticks: u32,
    /// Previous APU junction temperature for gradient calculation.
    pub prev_apu_temp_c: f32,
    /// Current synchronized exhaust fan PWM duty (0..100%).
    pub fan_duty_pct: u8,
}

impl PeltierCoolerManager {
    /// Creates a new cooler manager instance with default tuning.
    pub const fn new() -> Self {
        Self {
            tec: TecState::new(),
            pid: TecPidController::new(2.5, 0.2, 1.0, 30.0),
            turbo_active: false,
            burst_ticks: 0,
            cooldown_ticks: 0,
            prev_apu_temp_c: 45.0,
            fan_duty_pct: 20,
        }
    }

    /// Primary thermal management loop iteration evaluating telemetry, condensation, and PID governor.
    pub fn update(
        &mut self,
        telemetry: &MultiZoneThermalTelemetry,
        dt_seconds: f32,
    ) -> (u8, u8) {
        let dt = if dt_seconds <= 0.001 { 0.1 } else { dt_seconds };

        // 1. Critical safety check: Hot plate over-temperature shutdown
        if telemetry.tec_hot_plate_c >= HOT_PLATE_CRITICAL_TEMP_C {
            self.tec.emergency_shutdown();
            self.turbo_active = false;
            self.fan_duty_pct = 100;
            return (0, 100);
        } else if self.tec.failsafe_tripped && telemetry.tec_hot_plate_c < HOT_PLATE_WARNING_TEMP_C {
            self.tec.reset_failsafe();
        }

        // 2. Dew point condensation prevention
        let (condensation_risk, min_safe_temp_c) = evaluate_condensation_risk(telemetry);

        // 3. APU temperature gradient
        let apu_gradient = (telemetry.apu_junction_c - self.prev_apu_temp_c) / dt;
        self.prev_apu_temp_c = telemetry.apu_junction_c;

        // Decrement cooldown if running
        if self.cooldown_ticks > 0 {
            self.cooldown_ticks -= 1;
        }

        // 4. Turbo burst state machine
        if self.turbo_active {
            self.burst_ticks += 1;
            // Terminate burst on timeout or hot plate approaching warning
            if self.burst_ticks >= TURBO_BURST_MAX_TICKS || telemetry.tec_hot_plate_c >= HOT_PLATE_WARNING_TEMP_C {
                self.turbo_active = false;
                self.burst_ticks = 0;
                self.cooldown_ticks = TURBO_BURST_COOLDOWN_TICKS;
            }
        } else if self.cooldown_ticks == 0 && telemetry.tec_hot_plate_c < HOT_PLATE_WARNING_TEMP_C {
            // Check trigger conditions
            if (apu_gradient >= TURBO_GRADIENT_TRIGGER_C_PER_SEC || telemetry.apu_junction_c >= TURBO_TEMP_THRESHOLD_C)
                && !condensation_risk
            {
                self.turbo_active = true;
                self.burst_ticks = 0;
            }
        }

        // 5. Compute TEC duty
        let tec_duty = if self.turbo_active {
            self.tec.mode = TecMode::DynamicTurbo;
            MAX_TEC_DUTY_PCT
        } else if condensation_risk {
            self.tec.mode = TecMode::Eco;
            // Back off duty to allow cold plate to warm up above dew point
            (self.tec.duty_pct / 2).min(20)
        } else {
            self.tec.mode = TecMode::TargetTempPID;
            self.pid.update(telemetry.tec_cold_plate_c, dt, min_safe_temp_c)
        };

        self.tec.set_duty(tec_duty);

        // 6. Compute synchronized fan duty
        self.fan_duty_pct = calculate_synchronized_fan_duty(
            telemetry.apu_junction_c,
            self.tec.duty_pct,
            telemetry.tec_hot_plate_c,
        );

        (self.tec.duty_pct, self.fan_duty_pct)
    }
}

/// Verification helper for Commit 3 turbo burst governor and synchronized fan curve.
pub fn verify_turbo_burst_and_synchronized_fan_curve() -> bool {
    // 1. Verify Synchronized Fan Curve
    // APU at 50°C (base fan 40%), but TEC at 70% -> Fan must lead at 70 + 10 = 80%
    let fan_lead = calculate_synchronized_fan_duty(50.0, 70, 55.0);
    if fan_lead != 80 {
        return false;
    }

    // Hot plate at 78°C (>= warning 75°C) -> Fan must force 100%
    let fan_hot = calculate_synchronized_fan_duty(50.0, 30, 78.0);
    if fan_hot != 100 {
        return false;
    }

    // 2. Verify Critical Overtemp Shutdown
    let mut manager = PeltierCoolerManager::new();
    let crit_telem = MultiZoneThermalTelemetry::new(70.0, 60.0, 35.0, 25.0, 86.5, 25.0, 40.0);
    let (tec_d, fan_d) = manager.update(&crit_telem, 0.1);
    if tec_d != 0 || fan_d != 100 || !manager.tec.failsafe_tripped {
        return false;
    }

    // 3. Verify Turbo Cooling Burst Activation
    let mut burst_manager = PeltierCoolerManager::new();
    // Simulate sudden APU spike from 45°C to 55°C in 0.1s (gradient = 100°C/s >= 2.0°C/s)
    burst_manager.prev_apu_temp_c = 45.0;
    let spike_telem = MultiZoneThermalTelemetry::new(55.0, 48.0, 30.0, 28.0, 50.0, 24.0, 45.0);
    let (tec_burst, fan_burst) = burst_manager.update(&spike_telem, 0.1);
    if !burst_manager.turbo_active || tec_burst != 100 || fan_burst != 100 {
        return false;
    }

    true
}