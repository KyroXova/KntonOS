//! Collaborative Processor Performance Control (CPPC v2) & Hardware P-State MSR Interface.
//! Provides ACPI 6.x CPPC v2 performance capability discovery, per-core Desired Performance

/// Maximum number of logical CPU cores supported by CPPC.
pub const MAX_CPPC_CORES: usize = 16;

/// Intel / AMD HWP MSR definitions.
pub const IA32_PM_ENABLE: u32 = 0x0000_0770;
pub const IA32_HWP_CAPABILITIES: u32 = 0x0000_0771;
pub const IA32_HWP_REQUEST_PKG: u32 = 0x0000_0772;
pub const IA32_HWP_INTERRUPT: u32 = 0x0000_0773;
pub const IA32_HWP_REQUEST: u32 = 0x0000_0774;
pub const IA32_HWP_STATUS: u32 = 0x0000_0777;

/// Energy Performance Preference (EPP) hint values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnergyPerfPreference {
    /// Maximum throughput; bias towards highest frequency states with zero downclock latency.
    Performance = 0x00,
    /// Bias towards performance while allowing modest frequency drop during idle.
    BalancedPerformance = 0x40,
    /// Balanced energy and performance for general desktop / mobile usage.
    Balanced = 0x80,
    /// Aggressive power conservation; downclock rapidly on idle.
    PowerSaver = 0xC0,
    /// Lowest power consumption; clamp to most efficient nonlinear performance ratio.
    UltraPowerSaver = 0xFF,
}

impl EnergyPerfPreference {
    /// Raw 8-bit EPP register byte.
    #[inline]
    pub const fn to_raw(self) -> u8 {
        self as u8
    }

    /// Decodes raw 8-bit EPP byte to closest enum variant.
    #[inline]
    pub const fn from_raw(raw: u8) -> Self {
        if raw <= 0x20 {
            Self::Performance
        } else if raw <= 0x60 {
            Self::BalancedPerformance
        } else if raw <= 0xA0 {
            Self::Balanced
        } else if raw <= 0xE0 {
            Self::PowerSaver
        } else {
            Self::UltraPowerSaver
        }
    }
}

/// CPPC v2 performance capability descriptor for a logical core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CppcCapabilities {
    /// Highest performance ratio (e.g. boost clock ratio, 38 = 3.8 GHz).
    pub highest_perf: u8,
    /// Nominal / maximum sustained base clock performance ratio (e.g. 28 = 2.8 GHz).
    pub nominal_perf: u8,
    /// Lowest performance ratio where power scales non-linearly (most power-efficient ratio).
    pub lowest_nonlinear_perf: u8,
    /// Lowest supported performance ratio (e.g. 12 = 1.2 GHz).
    pub lowest_perf: u8,
}

impl CppcCapabilities {
    /// Decodes a 64-bit IA32_HWP_CAPABILITIES MSR value.
    pub const fn from_msr_value(val: u64) -> Self {
        Self {
            highest_perf: (val & 0xFF) as u8,
            nominal_perf: ((val >> 8) & 0xFF) as u8,
            lowest_nonlinear_perf: ((val >> 16) & 0xFF) as u8,
            lowest_perf: ((val >> 24) & 0xFF) as u8,
        }
    }

    /// Encodes into a 64-bit IA32_HWP_CAPABILITIES MSR value.
    pub const fn to_msr_value(&self) -> u64 {
        (self.highest_perf as u64)
            | ((self.nominal_perf as u64) << 8)
            | ((self.lowest_nonlinear_perf as u64) << 16)
            | ((self.lowest_perf as u64) << 24)
    }

    /// Default capability profile representing an 8-core modern handheld APU (e.g. 1.2 GHz to 3.8 GHz).
    pub const fn default_handheld() -> Self {
        Self {
            highest_perf: 38,          // 3.8 GHz Max Boost
            nominal_perf: 28,          // 2.8 GHz Base
            lowest_nonlinear_perf: 16, // 1.6 GHz Efficient
            lowest_perf: 12,           // 1.2 GHz Min
        }
    }
}

/// A single discrete frequency and voltage operating performance point (OPP).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrequencyStep {
    /// Core clock frequency in Megahertz (MHz).
    pub frequency_mhz: u32,
    /// Performance ratio (1 ratio = 100 MHz clock multiplier).
    pub perf_ratio: u8,
    /// Factory nominal core voltage in millivolts (mV).
    pub base_voltage_mv: u32,
}

/// Standard 8-step frequency curve for handheld Zen/Core mobile processors.
pub const STANDARD_FREQUENCY_TABLE: [FrequencyStep; 8] = [
    FrequencyStep { frequency_mhz: 1200, perf_ratio: 12, base_voltage_mv: 750 },
    FrequencyStep { frequency_mhz: 1600, perf_ratio: 16, base_voltage_mv: 800 },
    FrequencyStep { frequency_mhz: 2000, perf_ratio: 20, base_voltage_mv: 860 },
    FrequencyStep { frequency_mhz: 2400, perf_ratio: 24, base_voltage_mv: 920 },
    FrequencyStep { frequency_mhz: 2800, perf_ratio: 28, base_voltage_mv: 990 },
    FrequencyStep { frequency_mhz: 3200, perf_ratio: 32, base_voltage_mv: 1060 },
    FrequencyStep { frequency_mhz: 3600, perf_ratio: 36, base_voltage_mv: 1120 },
    FrequencyStep { frequency_mhz: 3800, perf_ratio: 38, base_voltage_mv: 1160 },
];

/// Encoded IA32_HWP_REQUEST structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HwpRequest {
    pub min_perf: u8,
    pub max_perf: u8,
    pub desired_perf: u8,
    pub epp: u8,
    pub activity_window: u8,
}

impl HwpRequest {
    /// Packs into 64-bit MSR payload.
    pub const fn to_raw(self) -> u64 {
        (self.min_perf as u64)
            | ((self.max_perf as u64) << 8)
            | ((self.desired_perf as u64) << 16)
            | ((self.epp as u64) << 24)
            | ((self.activity_window as u64) << 32)
    }

    /// Unpacks from 64-bit MSR payload.
    pub const fn from_raw(val: u64) -> Self {
        Self {
            min_perf: (val & 0xFF) as u8,
            max_perf: ((val >> 8) & 0xFF) as u8,
            desired_perf: ((val >> 16) & 0xFF) as u8,
            epp: ((val >> 24) & 0xFF) as u8,
            activity_window: ((val >> 32) & 0xFF) as u8,
        }
    }
}

/// Per-core CPPC v2 governor state.
#[derive(Debug, Clone, Copy)]
pub struct CoreCppcState {
    pub core_id: usize,
    pub capabilities: CppcCapabilities,
    pub current_request: HwpRequest,
    pub active_frequency_mhz: u32,
    pub total_transitions: usize,
}

impl CoreCppcState {
    pub const fn new(core_id: usize) -> Self {
        let caps = CppcCapabilities::default_handheld();
        let initial_req = HwpRequest {
            min_perf: caps.lowest_perf,
            max_perf: caps.highest_perf,
            desired_perf: caps.nominal_perf,
            epp: EnergyPerfPreference::Balanced.to_raw(),
            activity_window: 0,
        };
        Self {
            core_id,
            capabilities: caps,
            current_request: initial_req,
            active_frequency_mhz: caps.nominal_perf as u32 * 100,
            total_transitions: 0,
        }
    }
}

/// Central controller managing CPPC v2 performance states across all CPU cores.
pub struct CppcController {
    cores: [CoreCppcState; MAX_CPPC_CORES],
    total_perf_requests: usize,
    hardware_enabled: bool,
}

impl CppcController {
    /// Creates a new CPPC controller initialized for all cores.
    pub fn new() -> Self {
        Self {
            cores: [
                CoreCppcState::new(0),
                CoreCppcState::new(1),
                CoreCppcState::new(2),
                CoreCppcState::new(3),
                CoreCppcState::new(4),
                CoreCppcState::new(5),
                CoreCppcState::new(6),
                CoreCppcState::new(7),
                CoreCppcState::new(8),
                CoreCppcState::new(9),
                CoreCppcState::new(10),
                CoreCppcState::new(11),
                CoreCppcState::new(12),
                CoreCppcState::new(13),
                CoreCppcState::new(14),
                CoreCppcState::new(15),
            ],
            total_perf_requests: 0,
            hardware_enabled: true,
        }
    }

    /// Reads CPPC capabilities for the given core ID.
    pub fn capabilities(&self, core_id: usize) -> Option<CppcCapabilities> {
        let idx = core_id % MAX_CPPC_CORES;
        Some(self.cores[idx].capabilities)
    }

    /// Sets custom CPPC capabilities for a core (e.g. read from hardware MSR).
    pub fn set_capabilities(&mut self, core_id: usize, caps: CppcCapabilities) {
        let idx = core_id % MAX_CPPC_CORES;
        self.cores[idx].capabilities = caps;
    }

    /// Requests an explicit performance ratio and EPP hint for a specific core.
    pub fn request_performance(
        &mut self,
        core_id: usize,
        desired_ratio: u8,
        epp: EnergyPerfPreference,
    ) -> Result<u32, &'static str> {
        let idx = core_id % MAX_CPPC_CORES;
        let caps = self.cores[idx].capabilities;

        // Clamp desired ratio between lowest and highest capability
        let clamped_ratio = desired_ratio.clamp(caps.lowest_perf, caps.highest_perf);

        let req = HwpRequest {
            min_perf: caps.lowest_perf,
            max_perf: caps.highest_perf,
            desired_perf: clamped_ratio,
            epp: epp.to_raw(),
            activity_window: 0,
        };

        self.cores[idx].current_request = req;
        self.cores[idx].active_frequency_mhz = clamped_ratio as u32 * 100;
        self.cores[idx].total_transitions += 1;
        self.total_perf_requests += 1;

        Ok(self.cores[idx].active_frequency_mhz)
    }

    /// Finds the closest matching frequency step from the standard table.
    pub fn find_closest_step(target_mhz: u32) -> FrequencyStep {
        let mut closest = STANDARD_FREQUENCY_TABLE[0];
        let mut min_diff = target_mhz.abs_diff(closest.frequency_mhz);

        for &step in &STANDARD_FREQUENCY_TABLE {
            let diff = target_mhz.abs_diff(step.frequency_mhz);
            if diff < min_diff {
                min_diff = diff;
                closest = step;
            }
        }
        closest
    }

    /// Current frequency for a core in MHz.
    #[inline]
    pub fn current_frequency_mhz(&self, core_id: usize) -> u32 {
        let idx = core_id % MAX_CPPC_CORES;
        self.cores[idx].active_frequency_mhz
    }

    /// Total transitions across all cores.
    #[inline]
    pub fn total_perf_requests(&self) -> usize {
        self.total_perf_requests
    }

    /// Core state inspector.
    pub fn core_state(&self, core_id: usize) -> Option<&CoreCppcState> {
        let idx = core_id % MAX_CPPC_CORES;
        self.cores.get(idx)
    }

    /// Returns true if hardware CPPC/Speed Shift is active.
    #[inline]
    pub fn is_hardware_enabled(&self) -> bool {
        self.hardware_enabled
    }
}

/// Unit test verifying CPPC capabilities decoding, HWP request packing, and frequency step table lookup.
pub fn test_cppc_interface_and_stepping_tables() -> bool {
    // 1. Test CppcCapabilities MSR packing/unpacking
    let caps = CppcCapabilities {
        highest_perf: 38,
        nominal_perf: 28,
        lowest_nonlinear_perf: 16,
        lowest_perf: 12,
    };
    let msr_raw = caps.to_msr_value();
    let decoded = CppcCapabilities::from_msr_value(msr_raw);
    if decoded != caps {
        return false;
    }

    // 2. Test HwpRequest encoding
    let req = HwpRequest {
        min_perf: 12,
        max_perf: 38,
        desired_perf: 28,
        epp: EnergyPerfPreference::Balanced.to_raw(),
        activity_window: 0,
    };
    let req_raw = req.to_raw();
    let req_decoded = HwpRequest::from_raw(req_raw);
    if req_decoded != req {
        return false;
    }

    // 3. Test closest frequency step lookups
    let s1 = CppcController::find_closest_step(1400); // Between 1200 and 1600 (diff 200 vs 200, snaps to 1200 or 1600)
    if s1.frequency_mhz != 1200 && s1.frequency_mhz != 1600 {
        return false;
    }

    let s2 = CppcController::find_closest_step(3150); // Snaps to 3200
    if s2.frequency_mhz != 3200 || s2.perf_ratio != 32 || s2.base_voltage_mv != 1060 {
        return false;
    }

    let s3 = CppcController::find_closest_step(4500); // Above max boost, clamps to 3800
    if s3.frequency_mhz != 3800 || s3.perf_ratio != 38 {
        return false;
    }

    // 4. Test CppcController multi-core performance requests
    let mut ctrl = CppcController::new();

    // Core 0: Request performance boost to 3.8 GHz
    let freq0 = ctrl
        .request_performance(0, 38, EnergyPerfPreference::Performance)
        .unwrap();
    if freq0 != 3800 || ctrl.current_frequency_mhz(0) != 3800 {
        return false;
    }

    // Core 1: Request power saver downclock to 1.2 GHz
    let freq1 = ctrl
        .request_performance(1, 10, EnergyPerfPreference::PowerSaver) // Requested 10, clamped to lowest 12
        .unwrap();
    if freq1 != 1200 || ctrl.current_frequency_mhz(1) != 1200 {
        return false;
    }

    if ctrl.total_perf_requests() != 2 {
        return false;
    }

    true
}
