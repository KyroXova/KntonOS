//! Hardware Random Number Generation (RDRAND / RDSEED) and CPUID Detection.
//!
//! Provides x86_64 CPUID instruction leaf interrogation to detect on-chip
//! hardware digital random number generators (Intel Ivy Bridge / AMD Zen onwards).

/// Hardware RNG capability flags detected via CPUID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HardwareRngCapabilities {
    pub has_rdrand: bool,
    pub has_rdseed: bool,
}

impl HardwareRngCapabilities {
    pub const RDRAND_FLAG_ECX_BIT: u32 = 1 << 30; // Leaf 1, ECX bit 30
    pub const RDSEED_FLAG_EBX_BIT: u32 = 1 << 18; // Leaf 7, Subleaf 0, EBX bit 18

    /// Interrogates CPUID to detect presence of RDRAND and RDSEED instructions.
    pub fn detect() -> Self {
        // 1. Check maximum supported basic leaf
        let (max_leaf, _, _, _) = cpuid(0, 0);

        // 2. Leaf 1: Features & Capabilities (ECX bit 30 = RDRAND)
        let has_rdrand = if max_leaf >= 1 {
            let (_, _, ecx_1, _) = cpuid(1, 0);
            (ecx_1 & Self::RDRAND_FLAG_ECX_BIT) != 0
        } else {
            false
        };

        // 3. Leaf 7: Structured Extended Feature Flags (EBX bit 18 = RDSEED)
        let has_rdseed = if max_leaf >= 7 {
            let (_, ebx_7, _, _) = cpuid(7, 0);
            (ebx_7 & Self::RDSEED_FLAG_EBX_BIT) != 0
        } else {
            false
        };

        Self {
            has_rdrand,
            has_rdseed,
        }
    }
}

/// Executes x86_64 CPUID instruction with specified leaf and subleaf (ECX count).
#[inline(always)]
pub fn cpuid(leaf: u32, subleaf: u32) -> (u32, u32, u32, u32) {
    #[cfg(target_arch = "x86_64")]
    {
        let res = core::arch::x86_64::__cpuid_count(leaf, subleaf);
        (res.eax, res.ebx, res.ecx, res.edx)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (leaf, subleaf);
        (0, 0, 0, 0)
    }
}

/// Self-test verifying CPUID interrogation and hardware capability parsing.
pub fn verify_hw_rng_detection() -> bool {
    let caps = HardwareRngCapabilities::detect();
    // Capabilities must be boolean flags without panic
    let _ = caps.has_rdrand;
    let _ = caps.has_rdseed;

    // Verify basic leaf 0 returns non-zero max leaf
    let (max_leaf, _, _, _) = cpuid(0, 0);
    if max_leaf == 0 {
        return false;
    }

    true
}
