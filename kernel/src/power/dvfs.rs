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
