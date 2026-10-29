//! Hardware Random Number Generation (RDRAND / RDSEED) and Entropy Extractor.
//!
//! Provides x86_64 CPUID feature interrogation, hardware digital random number
//! extraction (RDRAND and RDSEED instructions), and high-resolution RDTSC jitter
//! fallback mixing for entropy-constrained execution environments.

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
        let (max_leaf, _, _, _) = cpuid(0, 0);

        let has_rdrand = if max_leaf >= 1 {
            let (_, _, ecx_1, _) = cpuid(1, 0);
            (ecx_1 & Self::RDRAND_FLAG_ECX_BIT) != 0
        } else {
            false
        };

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

pub const RDRAND_MAX_RETRIES: usize = 32;
pub const RDSEED_MAX_RETRIES: usize = 64;

/// Executes x86_64 RDRAND instruction returning 64-bit random word if carry flag is set.
#[inline]
pub fn rdrand64() -> Option<u64> {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let val: u64;
        let ok: u8;
        core::arch::asm!(
            "rdrand {val}",
            "setc {ok}",
            val = out(reg) val,
            ok = out(reg_byte) ok,
            options(nomem, nostack)
        );
        if ok != 0 {
            Some(val)
        } else {
            None
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    None
}

/// Executes x86_64 RDSEED instruction returning 64-bit true entropy word if carry flag is set.
#[inline]
pub fn rdseed64() -> Option<u64> {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let val: u64;
        let ok: u8;
        core::arch::asm!(
            "rdseed {val}",
            "setc {ok}",
            val = out(reg) val,
            ok = out(reg_byte) ok,
            options(nomem, nostack)
        );
        if ok != 0 {
            Some(val)
        } else {
            None
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    None
}

/// Reads x86_64 Time-Stamp Counter (RDTSC).
#[inline(always)]
pub fn rdtsc() -> u64 {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let eax: u32;
        let edx: u32;
        core::arch::asm!(
            "rdtsc",
            out("eax") eax,
            out("edx") edx,
            options(nomem, nostack, preserves_flags)
        );
        ((edx as u64) << 32) | (eax as u64)
    }
    #[cfg(not(target_arch = "x86_64"))]
    0
}

/// Entropy source classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntropySource {
    RdSeed,
    RdRand,
    RdtscJitter,
}

/// Hardware Entropy Extractor and Fallback Seed Mixer.
#[derive(Debug, Clone)]
pub struct HardwareEntropyExtractor {
    pub caps: HardwareRngCapabilities,
    pub fallback_state: u64,
}

impl HardwareEntropyExtractor {
    pub fn new() -> Self {
        let caps = HardwareRngCapabilities::detect();
        let initial_seed = rdtsc() ^ 0x517cc1b727220a95;
        Self {
            caps,
            fallback_state: initial_seed,
        }
    }

    /// Extracts a single 64-bit entropy word using hardware instruction or jitter fallback.
    pub fn extract_u64(&mut self) -> (u64, EntropySource) {
        // 1. Try RDSEED if supported
        if self.caps.has_rdseed {
            for _ in 0..RDSEED_MAX_RETRIES {
                if let Some(val) = rdseed64() {
                    return (val, EntropySource::RdSeed);
                }
                core::hint::spin_loop();
            }
        }

        // 2. Try RDRAND if supported
        if self.caps.has_rdrand {
            for _ in 0..RDRAND_MAX_RETRIES {
                if let Some(val) = rdrand64() {
                    return (val, EntropySource::RdRand);
                }
                core::hint::spin_loop();
            }
        }

        // 3. Fallback: SplitMix64 mixing RDTSC cycle jitter and memory state
        let jitter = self.sample_jitter();
        self.fallback_state = self.fallback_state.wrapping_add(0x9E3779B97F4A7C15) ^ jitter;
        let mut z = self.fallback_state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        let val = z ^ (z >> 31);

        (val, EntropySource::RdtscJitter)
    }

    /// Samples timing jitter across tight ALU and memory operations.
    fn sample_jitter(&mut self) -> u64 {
        let mut acc = rdtsc() ^ 0xCBF29CE484222325;
        for i in 0..16 {
            let t1 = rdtsc();
            core::hint::black_box(i * 37 + 13);
            let t2 = rdtsc();
            let delta = t2.wrapping_sub(t1);
            acc = (acc ^ delta).wrapping_mul(0x100000001B3);
            acc = acc.rotate_left(3);
        }
        acc
    }

    /// Populates a 32-byte (256-bit) seed array from entropy sources.
    pub fn extract_seed(&mut self, out_seed: &mut [u8; 32]) -> EntropySource {
        let mut primary_source = EntropySource::RdSeed;

        for i in 0..4 {
            let (word, src) = self.extract_u64();
            if src == EntropySource::RdtscJitter {
                primary_source = EntropySource::RdtscJitter;
            } else if src == EntropySource::RdRand && primary_source != EntropySource::RdtscJitter {
                primary_source = EntropySource::RdRand;
            }
            let le = word.to_le_bytes();
            let offset = i * 8;
            out_seed[offset..offset + 8].copy_from_slice(&le);
        }

        primary_source
    }
}

/// Self-test verifying CPUID interrogation and hardware capability parsing.
pub fn verify_hw_rng_detection() -> bool {
    let caps = HardwareRngCapabilities::detect();
    let _ = caps.has_rdrand;
    let _ = caps.has_rdseed;

    let (max_leaf, _, _, _) = cpuid(0, 0);
    if max_leaf == 0 {
        return false;
    }

    true
}

/// Self-test verifying hardware entropy extraction and fallback seed generation.
pub fn verify_entropy_extractor() -> bool {
    let mut extractor = HardwareEntropyExtractor::new();

    // 1. Verify 64-bit entropy extraction
    let (w1, src1) = extractor.extract_u64();
    let (w2, _) = extractor.extract_u64();

    // Entropy words must not be equal
    if w1 == w2 || (w1 == 0 && w2 == 0) {
        return false;
    }

    // 2. Verify 32-byte seed extraction
    let mut seed = [0u8; 32];
    let seed_src = extractor.extract_seed(&mut seed);

    // Verify seed has non-zero bytes and variance
    let all_zero = seed.iter().all(|&b| b == 0);
    let all_same = seed.iter().all(|&b| b == seed[0]);
    if all_zero || all_same {
        return false;
    }

    let _ = (src1, seed_src);
    true
}
