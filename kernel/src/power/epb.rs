//! Energy-Performance Bias (EPB) & Hardware P-State EPP Governor Profiles.
//!
//! Controls Intel IA32_ENERGY_PERF_BIAS (MSR 0x1B0) and Intel/AMD IA32_HWP_REQUEST
//! Energy-Performance Preference (EPP bits 31:24 of MSR 0x774).
//! Provides high-level profiles: Performance, Balanced, and PowerSaver.

use crate::arch::x86_64::syscall::{rdmsr, wrmsr};

// -----------------------------------------------------------------------------
// Hardware MSR Addresses & Architectural Masks
// -----------------------------------------------------------------------------

/// Intel IA32_ENERGY_PERF_BIAS MSR (introduced in Nehalem/Sandy Bridge).
/// Bits 3:0 specify EPB policy:
/// - 0: Maximum Performance
/// - 4: Balanced Performance
/// - 6: Normal Balanced
/// - 8: Balanced Power
/// - 15: Maximum Power Saving
pub const IA32_ENERGY_PERF_BIAS_MSR: u32 = 0x0000_01B0;

/// Intel & AMD Hardware-Controlled P-States (HWP) Request MSR.
/// Bits 31:24 define Energy_Performance_Preference (EPP):
/// - 0x00: Performance
/// - 0x40: Balance Performance
/// - 0x80: Balance Power
/// - 0xFF: Power Saver
pub const IA32_HWP_REQUEST_MSR: u32 = 0x0000_0774;

/// Intel IA32_MISC_ENABLE MSR. Bit 16 indicates whether Turbo Mode is available.
pub const IA32_MISC_ENABLE_MSR: u32 = 0x0000_01A0;

/// Bitmask for EPP byte in IA32_HWP_REQUEST (bits 31:24).
pub const HWP_REQUEST_EPP_MASK: u64 = 0xFF00_0000;
pub const HWP_REQUEST_EPP_SHIFT: u32 = 24;

// -----------------------------------------------------------------------------
// EPB / EPP Governor Profiles
// -----------------------------------------------------------------------------

/// High-level Energy-Performance governor profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpbProfile {
    /// Maximum responsiveness and compute performance (gaming, heavy compile).
    Performance,
    /// Dynamic trade-off between performance and battery/thermal consumption.
    Balanced,
    /// Aggressive power conservation and extended battery life.
    PowerSaver,
}

impl EpbProfile {
    /// Returns architectural Intel IA32_ENERGY_PERF_BIAS value (0..15).
    pub const fn to_epb_value(self) -> u8 {
        match self {
            Self::Performance => 0,  // Max Performance
            Self::Balanced => 6,     // Normal Balanced
            Self::PowerSaver => 15,  // Max Power Saving
        }
    }

    /// Returns architectural Intel/AMD HWP EPP byte value (0x00..0xFF).
    pub const fn to_epp_value(self) -> u8 {
        match self {
            Self::Performance => 0x00, // 0 = Performance
            Self::Balanced => 0x80,    // 128 = Balance Power/Perf
            Self::PowerSaver => 0xFF,  // 255 = Power Saver
        }
    }
}

// -----------------------------------------------------------------------------
// Hardware MSR Controller & Emulated Register Store
// -----------------------------------------------------------------------------

/// Hardware EPB & EPP MSR Controller.
pub struct HardwareEpbManager {
    pub current_profile: EpbProfile,
    pub epb_supported: bool,
    pub hwp_supported: bool,
    pub simulated_epb_msr: u64,
    pub simulated_hwp_msr: u64,
}

impl HardwareEpbManager {
    pub fn new(epb_supported: bool, hwp_supported: bool) -> Self {
        Self {
            current_profile: EpbProfile::Balanced,
            epb_supported,
            hwp_supported,
            simulated_epb_msr: EpbProfile::Balanced.to_epb_value() as u64,
            simulated_hwp_msr: (EpbProfile::Balanced.to_epp_value() as u64) << HWP_REQUEST_EPP_SHIFT,
        }
    }

    /// Applies profile to hardware MSRs (or simulated registers in virtualized/test environments).
    pub fn apply_profile(&mut self, profile: EpbProfile, write_hardware_msrs: bool) {
        self.current_profile = profile;

        let epb_val = profile.to_epb_value() as u64;
        let epp_val = profile.to_epp_value() as u64;

        self.simulated_epb_msr = epb_val;
        self.simulated_hwp_msr = (self.simulated_hwp_msr & !HWP_REQUEST_EPP_MASK) | (epp_val << HWP_REQUEST_EPP_SHIFT);

        if write_hardware_msrs {
            unsafe {
                if self.epb_supported {
                    wrmsr(IA32_ENERGY_PERF_BIAS_MSR, epb_val);
                }
                if self.hwp_supported {
                    let cur_hwp = rdmsr(IA32_HWP_REQUEST_MSR);
                    let new_hwp = (cur_hwp & !HWP_REQUEST_EPP_MASK) | (epp_val << HWP_REQUEST_EPP_SHIFT);
                    wrmsr(IA32_HWP_REQUEST_MSR, new_hwp);
                }
            }
        }
    }

    /// Reads back effective EPB byte (0..15).
    pub fn read_effective_epb(&self) -> u8 {
        (self.simulated_epb_msr & 0x0F) as u8
    }

    /// Reads back effective HWP EPP byte (0x00..0xFF).
    pub fn read_effective_epp(&self) -> u8 {
        ((self.simulated_hwp_msr & HWP_REQUEST_EPP_MASK) >> HWP_REQUEST_EPP_SHIFT) as u8
    }
}

// -----------------------------------------------------------------------------
// Unit Tests & Verification
// -----------------------------------------------------------------------------

/// Verification self-test for EPB and EPP hardware MSR governor profiles.
pub fn verify_epb_and_epp_profiles() -> bool {
    let mut mgr = HardwareEpbManager::new(true, true);

    // Initial default is Balanced
    if mgr.current_profile != EpbProfile::Balanced {
        return false;
    }
    if mgr.read_effective_epb() != 6 || mgr.read_effective_epp() != 0x80 {
        return false;
    }

    // Transition to Performance
    mgr.apply_profile(EpbProfile::Performance, false);
    if mgr.current_profile != EpbProfile::Performance {
        return false;
    }
    if mgr.read_effective_epb() != 0 || mgr.read_effective_epp() != 0x00 {
        return false;
    }

    // Transition to PowerSaver
    mgr.apply_profile(EpbProfile::PowerSaver, false);
    if mgr.current_profile != EpbProfile::PowerSaver {
        return false;
    }
    if mgr.read_effective_epb() != 15 || mgr.read_effective_epp() != 0xFF {
        return false;
    }

    true
}
