//! Dynamic Voltage & Frequency Scaling (DVFS) & Micro-Voltage Stepping Governor.
//!
//! Provides unified Operating Performance Point (OPP) generation, per-core micro-voltage
//! curve undervolting offsets (-25mV to -100mV), and physical power estimation for handheld
//! x86-64 gaming APUs.

use super::cppc::CppcController;

/// Maximum logical cores managed by the DVFS governor.
pub const MAX_DVFS_CORES: usize = 16;

/// Hardware safety limits for core voltage (in millivolts).
pub const MIN_SAFE_VOLTAGE_MV: u32 = 680;  // 0.680 V minimum stable logic rail
pub const MAX_SAFE_VOLTAGE_MV: u32 = 1250; // 1.250 V maximum safe thermal rail

/// Target 60 FPS frame time in microseconds (16.666 ms).
pub const FRAME_TIME_TARGET_60FPS_US: u32 = 16_666;

/// Number of consecutive smooth frames required before decaying boost frequency (hysteresis window).
pub const BOOST_COOLDOWN_SMOOTH_FRAMES: u32 = 20;

/// Thermal throttle trip temperature (in degrees Celsius).
pub const THERMAL_THROTTLE_TRIP_CELSIUS: u32 = 92;

/// Thermal throttle hysteresis clear temperature (in degrees Celsius).
pub const THERMAL_THROTTLE_CLEAR_CELSIUS: u32 = 82;

/// Clamped maximum frequency while thermally throttled (2.0 GHz).
pub const THERMAL_THROTTLE_MAX_FREQ_MHZ: u32 = 2000;

/// Maximum allowed undervolt offset (-100 mV).
pub const MAX_UNDERVOLT_OFFSET_MV: i32 = -100;
/// Minimum allowed undervolt offset (0 mV = factory nominal, +25 mV max overvolt).
pub const MAX_OVERVOLT_OFFSET_MV: i32 = 25;

/// Real-time operating performance point of a CPU core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DvfsOperatingPoint {
    /// Active core frequency in Megahertz (MHz).
    pub frequency_mhz: u32,
    /// Factory nominal base voltage for this frequency step (mV).
    pub base_voltage_mv: u32,
    /// User/governor undervolting curve offset (mV, negative = undervolt).
    pub undervolt_offset_mv: i32,
    /// Final commanded core rail voltage after applying offset and safety clamping (mV).
    pub effective_voltage_mv: u32,
    /// Estimated core dynamic and static power draw (milliwatts).
    pub estimated_power_mw: u32,
}

/// State and voltage curve tracking for a single logical core.
#[derive(Debug, Clone, Copy)]
pub struct CoreDvfsState {
    pub core_id: usize,
    pub current_opp: DvfsOperatingPoint,
    pub undervolt_offset_mv: i32,
    pub total_opp_transitions: usize,
}

impl CoreDvfsState {
    /// Creates a default DVFS state for a core at nominal base clock (2.8 GHz @ 990 mV, 0 offset).
    pub const fn new(core_id: usize) -> Self {
        let base_freq = 2800;
        let base_volt = 990;
        let opp = DvfsOperatingPoint {
            frequency_mhz: base_freq,
            base_voltage_mv: base_volt,
            undervolt_offset_mv: 0,
            effective_voltage_mv: base_volt,
            estimated_power_mw: 8200, // ~8.2W nominal at 2.8 GHz
        };
        Self {
            core_id,
            current_opp: opp,
            undervolt_offset_mv: 0,
            total_opp_transitions: 0,
        }
    }
}

/// Dynamic Voltage and Frequency Scaling (DVFS) Governor.
pub struct DvfsGovernor {
    cores: [CoreDvfsState; MAX_DVFS_CORES],
    cppc: CppcController,
    total_governor_ticks: usize,
    boost_active: bool,
    cooldown_frames_remaining: u32,
    total_frame_spikes: usize,
    total_boost_events: usize,
    thermal_throttled: bool,
    current_temperature_celsius: u32,
    total_thermal_throttle_events: usize,
}

impl DvfsGovernor {
    /// Creates a new DVFS governor initialized with factory nominal OPP curves.
    pub fn new() -> Self {
        Self {
            cores: [
                CoreDvfsState::new(0),
                CoreDvfsState::new(1),
                CoreDvfsState::new(2),
                CoreDvfsState::new(3),
                CoreDvfsState::new(4),
                CoreDvfsState::new(5),
                CoreDvfsState::new(6),
                CoreDvfsState::new(7),
                CoreDvfsState::new(8),
                CoreDvfsState::new(9),
                CoreDvfsState::new(10),
                CoreDvfsState::new(11),
                CoreDvfsState::new(12),
                CoreDvfsState::new(13),
                CoreDvfsState::new(14),
                CoreDvfsState::new(15),
            ],
            cppc: CppcController::new(),
            total_governor_ticks: 0,
            boost_active: false,
            cooldown_frames_remaining: 0,
            total_frame_spikes: 0,
            total_boost_events: 0,
            thermal_throttled: false,
            current_temperature_celsius: 45,
            total_thermal_throttle_events: 0,
        }
    }

    /// Sets the micro-voltage undervolting curve offset for a specific core.
    /// Offset is clamped to [-100 mV, +25 mV] for hardware stability.
    pub fn set_core_undervolt(
        &mut self,
        core_id: usize,
        offset_mv: i32,
    ) -> Result<i32, &'static str> {
        let idx = core_id % MAX_DVFS_CORES;
        let clamped = offset_mv.clamp(MAX_UNDERVOLT_OFFSET_MV, MAX_OVERVOLT_OFFSET_MV);
        self.cores[idx].undervolt_offset_mv = clamped;

        // Recalculate current operating point voltage and power with new offset
        let curr_mhz = self.cores[idx].current_opp.frequency_mhz;
        self.set_target_frequency(core_id, curr_mhz)?;

        Ok(clamped)
    }

    /// Sets an identical undervolt offset across all CPU cores.
    pub fn set_all_undervolt(&mut self, offset_mv: i32) {
        for core_id in 0..MAX_DVFS_CORES {
            let _ = self.set_core_undervolt(core_id, offset_mv);
        }
    }

    /// Sets target operating frequency for a core, calculating matching base voltage,
    /// effective voltage, and estimated power draw.
    pub fn set_target_frequency(
        &mut self,
        core_id: usize,
        target_mhz: u32,
    ) -> Result<DvfsOperatingPoint, &'static str> {
        let idx = core_id % MAX_DVFS_CORES;
        let step = CppcController::find_closest_step(target_mhz);

        let offset = self.cores[idx].undervolt_offset_mv;
        let effective_raw = step.base_voltage_mv as i32 + offset;
        let effective_voltage_mv =
            (effective_raw.clamp(MIN_SAFE_VOLTAGE_MV as i32, MAX_SAFE_VOLTAGE_MV as i32)) as u32;

        let estimated_power_mw = Self::estimate_core_power_mw(step.frequency_mhz, effective_voltage_mv);

        let opp = DvfsOperatingPoint {
            frequency_mhz: step.frequency_mhz,
            base_voltage_mv: step.base_voltage_mv,
            undervolt_offset_mv: offset,
            effective_voltage_mv,
            estimated_power_mw,
        };

        self.cores[idx].current_opp = opp;
        self.cores[idx].total_opp_transitions += 1;

        // Synchronize with CPPC hardware request
        let _ = self.cppc.request_performance(
            core_id,
            step.perf_ratio,
            super::cppc::EnergyPerfPreference::Balanced,
        );

        Ok(opp)
    }

    /// Estimates core dynamic and static power draw (in milliwatts) using P ~ C * V^2 * f + V * I_leak.
    pub fn estimate_core_power_mw(freq_mhz: u32, voltage_mv: u32) -> u32 {
        let v_norm = voltage_mv as f32 / 1000.0; // Volts
        let f_norm = freq_mhz as f32 / 1000.0;   // GHz

        // Dynamic capacitance factor (empirical for 7nm/5nm mobile core: ~1.8 W / (V^2 * GHz))
        let dynamic_mw = 1800.0 * (v_norm * v_norm) * f_norm;

        // Static leakage current factor (exponential with voltage: ~0.8 W @ 1.0V)
        let leakage_mw = 800.0 * (v_norm * v_norm * v_norm);

        (dynamic_mw + leakage_mw) as u32
    }

    /// Calculates power savings in milliwatts achieved by undervolting on the current OPP.
    pub fn calculate_power_savings_mw(&self, core_id: usize) -> u32 {
        let idx = core_id % MAX_DVFS_CORES;
        let opp = self.cores[idx].current_opp;
        let nominal_power = Self::estimate_core_power_mw(opp.frequency_mhz, opp.base_voltage_mv);
        let undervolted_power = opp.estimated_power_mw;
        nominal_power.saturating_sub(undervolted_power)
    }

    /// Obtains current operating point for a core.
    #[inline]
    pub fn operating_point(&self, core_id: usize) -> DvfsOperatingPoint {
        let idx = core_id % MAX_DVFS_CORES;
        self.cores[idx].current_opp
    }

    /// Access internal CPPC controller.
    #[inline]
    pub fn cppc(&self) -> &CppcController {
        &self.cppc
    }

    /// Access internal CPPC controller mutably.
    #[inline]
    pub fn cppc_mut(&mut self) -> &mut CppcController {
        &mut self.cppc
    }

    /// Core state inspector.
    pub fn core_state(&self, core_id: usize) -> Option<&CoreDvfsState> {
        let idx = core_id % MAX_DVFS_CORES;
        self.cores.get(idx)
    }

    /// Total governor update ticks.
    #[inline]
    pub fn total_governor_ticks(&self) -> usize {
        self.total_governor_ticks
    }

    /// Evaluates frame presentation timing and triggers reactive boost on frame drops.
    pub fn on_frame_presented(&mut self, frame_time_us: u32) {
        self.total_governor_ticks += 1;

        if self.thermal_throttled {
            // Cannot boost while thermal throttle is active
            return;
        }

        if frame_time_us > FRAME_TIME_TARGET_60FPS_US {
            // Frame drop detected: immediate reactive boost to max clock
            self.total_frame_spikes += 1;
            self.boost_active = true;
            self.cooldown_frames_remaining = BOOST_COOLDOWN_SMOOTH_FRAMES;
            self.total_boost_events += 1;

            // Boost all active cores to max boost clock (3.8 GHz)
            for core_id in 0..MAX_DVFS_CORES {
                let _ = self.set_target_frequency(core_id, 3800);
            }
        } else if self.boost_active {
            // Smooth frame delivered during boost cooldown
            if self.cooldown_frames_remaining > 0 {
                self.cooldown_frames_remaining -= 1;
            }
            if self.cooldown_frames_remaining == 0 {
                // Cooldown expired: decay boost frequency down to balanced nominal (2.8 GHz)
                self.boost_active = false;
                for core_id in 0..MAX_DVFS_CORES {
                    let _ = self.set_target_frequency(core_id, 2800);
                }
            }
        }
    }

    /// Evaluates thermal status and regulates clock limits with a 10°C hysteresis band.
    pub fn on_temperature_update(&mut self, temp_celsius: u32) {
        self.current_temperature_celsius = temp_celsius;

        if !self.thermal_throttled {
            if temp_celsius >= THERMAL_THROTTLE_TRIP_CELSIUS {
                // Trip thermal throttling
                self.thermal_throttled = true;
                self.boost_active = false;
                self.cooldown_frames_remaining = 0;
                self.total_thermal_throttle_events += 1;

                // Clamp all cores to throttled ceiling (2.0 GHz)
                for core_id in 0..MAX_DVFS_CORES {
                    let _ = self.set_target_frequency(core_id, THERMAL_THROTTLE_MAX_FREQ_MHZ);
                }
            }
        } else {
            // Throttled: require temperature to drop below CLEAR threshold (82°C)
            if temp_celsius <= THERMAL_THROTTLE_CLEAR_CELSIUS {
                self.thermal_throttled = false;
                // Restore nominal balanced frequency (2.8 GHz)
                for core_id in 0..MAX_DVFS_CORES {
                    let _ = self.set_target_frequency(core_id, 2800);
                }
            }
        }
    }

    /// True if reactive game frame boost is actively holding elevated clocks.
    #[inline]
    pub fn is_boost_active(&self) -> bool {
        self.boost_active
    }

    /// True if thermal throttling is active.
    #[inline]
    pub fn is_thermal_throttled(&self) -> bool {
        self.thermal_throttled
    }

    /// Total frame drops detected.
    #[inline]
    pub fn total_frame_spikes(&self) -> usize {
        self.total_frame_spikes
    }

    /// Total reactive boost activations.
    #[inline]
    pub fn total_boost_events(&self) -> usize {
        self.total_boost_events
    }

    /// Total thermal throttle trip events.
    #[inline]
    pub fn total_thermal_throttle_events(&self) -> usize {
        self.total_thermal_throttle_events
    }

    /// Current monitored core temperature in Celsius.
    #[inline]
    pub fn current_temperature_celsius(&self) -> u32 {
        self.current_temperature_celsius
    }
}

/// Unit test verifying OPP curves, micro-voltage undervolting offsets, safety clamping, and power savings.
pub fn test_micro_voltage_stepping_and_undervolt() -> bool {
    let mut gov = DvfsGovernor::new();

    // Verify initial OPP on Core 0 (default: 2.8 GHz @ 990 mV)
    let opp0 = gov.operating_point(0);
    if opp0.frequency_mhz != 2800 || opp0.base_voltage_mv != 990 || opp0.undervolt_offset_mv != 0 {
        return false;
    }
    if opp0.effective_voltage_mv != 990 {
        return false;
    }

    // 1. Apply -50 mV undervolt offset on Core 0
    let applied_offset = gov.set_core_undervolt(0, -50).unwrap();
    if applied_offset != -50 {
        return false;
    }

    let opp0_uv = gov.operating_point(0);
    if opp0_uv.undervolt_offset_mv != -50 {
        return false;
    }
    // Base 990 mV - 50 mV = 940 mV effective
    if opp0_uv.effective_voltage_mv != 940 {
        return false;
    }

    // Verify power savings > 0
    let savings_mw = gov.calculate_power_savings_mw(0);
    if savings_mw == 0 {
        return false;
    }

    // 2. Transition Core 0 to 3.8 GHz boost clock
    let opp0_boost = gov.set_target_frequency(0, 3800).unwrap();
    if opp0_boost.frequency_mhz != 3800 || opp0_boost.base_voltage_mv != 1160 {
        return false;
    }
    // Base 1160 mV - 50 mV = 1110 mV effective
    if opp0_boost.effective_voltage_mv != 1110 {
        return false;
    }

    // 3. Test Safety Clamping
    // Request extreme undervolt (-150 mV): must be clamped to MAX_UNDERVOLT_OFFSET_MV (-100 mV)
    let clamped_offset = gov.set_core_undervolt(1, -150).unwrap();
    if clamped_offset != -100 {
        return false;
    }

    // Set Core 1 to minimum frequency (1.2 GHz, base 750 mV)
    // 750 mV - 100 mV = 650 mV, which is below MIN_SAFE_VOLTAGE_MV (680 mV)!
    // Must be clamped to 680 mV!
    let opp1_min = gov.set_target_frequency(1, 1200).unwrap();
    if opp1_min.effective_voltage_mv != MIN_SAFE_VOLTAGE_MV {
        return false;
    }

    true
}

/// Unit test verifying game frame-time spike reactive boost and thermal throttle hysteresis regulator.
pub fn test_frame_boost_and_thermal_hysteresis() -> bool {
    let mut gov = DvfsGovernor::new();

    // Set initial baseline frequency across all cores to 2.4 GHz
    for core_id in 0..MAX_DVFS_CORES {
        let _ = gov.set_target_frequency(core_id, 2400);
    }

    // 1. Present 5 smooth frames (14.0 ms = 14,000 us < 16.666 ms)
    for _ in 0..5 {
        gov.on_frame_presented(14_000);
    }
    if gov.is_boost_active() || gov.total_frame_spikes() != 0 {
        return false;
    }
    if gov.operating_point(0).frequency_mhz != 2400 {
        return false;
    }

    // 2. Present frame spike: 22.0 ms (22,000 us > 16.666 ms)
    gov.on_frame_presented(22_000);
    if !gov.is_boost_active() || gov.total_frame_spikes() != 1 || gov.total_boost_events() != 1 {
        return false;
    }
    // All cores must be boosted to 3.8 GHz
    if gov.operating_point(0).frequency_mhz != 3800 {
        return false;
    }

    // 3. Present 10 smooth frames: boost must stay active due to hysteresis cooldown
    for _ in 0..10 {
        gov.on_frame_presented(14_000);
    }
    if !gov.is_boost_active() || gov.operating_point(0).frequency_mhz != 3800 {
        return false;
    }

    // Present remaining 10 smooth frames (total 20 smooth frames): cooldown expires!
    for _ in 0..10 {
        gov.on_frame_presented(14_000);
    }
    if gov.is_boost_active() || gov.operating_point(0).frequency_mhz != 2800 {
        return false;
    }

    // 4. Test Thermal Hysteresis (Trip at 92°C, Clear at 82°C)
    // Rise to 88°C -> not throttled
    gov.on_temperature_update(88);
    if gov.is_thermal_throttled() {
        return false;
    }

    // Rise to 93°C (>= 92°C) -> trips thermal throttling
    gov.on_temperature_update(93);
    if !gov.is_thermal_throttled() || gov.total_thermal_throttle_events() != 1 {
        return false;
    }
    // Clamped to 2.0 GHz
    if gov.operating_point(0).frequency_mhz != THERMAL_THROTTLE_MAX_FREQ_MHZ {
        return false;
    }

    // Present frame spike while throttled: boost must be suppressed by thermal throttle!
    gov.on_frame_presented(25_000);
    if gov.is_boost_active() || gov.operating_point(0).frequency_mhz != THERMAL_THROTTLE_MAX_FREQ_MHZ {
        return false;
    }

    // Temperature cools to 86°C: hysteresis band holds throttle active!
    gov.on_temperature_update(86);
    if !gov.is_thermal_throttled() {
        return false;
    }

    // Temperature cools to 81°C (<= 82°C clear threshold): throttle cleared!
    gov.on_temperature_update(81);
    if gov.is_thermal_throttled() {
        return false;
    }
    // Cores restored to 2.8 GHz
    if gov.operating_point(0).frequency_mhz != 2800 {
        return false;
    }

    true
}
