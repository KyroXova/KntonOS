//! Hardware Random Number Generation (RDRAND / RDSEED) and Entropy Extractor.
//! Provides x86_64 CPUID feature interrogation, hardware digital random number

use crate::crypto::chacha20::{
    ChaCha20State, CHACHA20_BLOCK_SIZE, CHACHA20_KEY_SIZE, CHACHA20_NONCE_SIZE,
};
use crate::memory::spinlock::Spinlock;

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

pub const ENTROPY_POOL_SIZE: usize = 64;

/// Kernel Cryptographic Entropy Pool.
#[derive(Debug, Clone)]
pub struct KernelEntropyPool {
    pool: [u8; ENTROPY_POOL_SIZE],
    pool_index: usize,
    extractor: HardwareEntropyExtractor,
}

impl KernelEntropyPool {
    pub fn new() -> Self {
        let mut pool = Self {
            pool: [0u8; ENTROPY_POOL_SIZE],
            pool_index: 0,
            extractor: HardwareEntropyExtractor::new(),
        };
        pool.harvest_hardware_entropy();
        pool
    }

    /// Mixes external entropy into the circular entropy pool buffer.
    pub fn add_entropy(&mut self, data: &[u8]) {
        for &byte in data {
            self.pool[self.pool_index] ^= byte.rotate_left(3);
            self.pool_index = (self.pool_index + 1) % ENTROPY_POOL_SIZE;
        }
    }

    /// Harvests 32 bytes from hardware RNG / jitter and mixes into the pool.
    pub fn harvest_hardware_entropy(&mut self) -> EntropySource {
        let mut seed = [0u8; 32];
        let src = self.extractor.extract_seed(&mut seed);
        self.add_entropy(&seed);
        src
    }

    /// Extracts a fresh 32-byte seed derived from the entropy pool and hardware extractor.
    pub fn extract_seed(&mut self, out_seed: &mut [u8; 32]) {
        self.harvest_hardware_entropy();
        for i in 0..32 {
            out_seed[i] = self.pool[i] ^ self.pool[i + 32].rotate_left(1);
        }
    }
}

pub const DEFAULT_RESEED_INTERVAL: usize = 1024 * 1024; // 1 MiB

/// ChaCha20-based Cryptographically Secure Pseudorandom Number Generator (CSPRNG).
#[derive(Debug, Clone)]
pub struct ChaCha20Csprng {
    state: ChaCha20State,
    buffer: [u8; CHACHA20_BLOCK_SIZE],
    buffer_index: usize,
    bytes_generated: usize,
    reseed_interval: usize,
    pool: KernelEntropyPool,
    nonce_counter: u64,
}

impl ChaCha20Csprng {
    /// Initializes CSPRNG with hardware entropy and generates the first keystream block.
    pub fn new() -> Self {
        let mut pool = KernelEntropyPool::new();
        let mut initial_key = [0u8; CHACHA20_KEY_SIZE];
        pool.extract_seed(&mut initial_key);

        let initial_nonce = [0xAA, 0x55, 0x33, 0x11, 0x77, 0x88, 0x99, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF];
        let state = ChaCha20State::new(&initial_key, &initial_nonce, 0);

        let mut csprng = Self {
            state,
            buffer: [0u8; CHACHA20_BLOCK_SIZE],
            buffer_index: CHACHA20_BLOCK_SIZE, // Forces immediate block generation on first read
            bytes_generated: 0,
            reseed_interval: DEFAULT_RESEED_INTERVAL,
            pool,
            nonce_counter: 1,
        };

        csprng.refresh_buffer();
        csprng
    }

    /// Refreshes the internal 64-byte keystream buffer.
    fn refresh_buffer(&mut self) {
        self.buffer = self.state.generate_block();
        self.buffer_index = 0;
    }

    /// Reseeds the ChaCha20 key using fresh entropy from the kernel pool.
    pub fn reseed(&mut self) {
        let mut fresh_entropy = [0u8; CHACHA20_KEY_SIZE];
        self.pool.extract_seed(&mut fresh_entropy);

        // Mix fresh entropy with next 32 bytes of ChaCha20 keystream for forward secrecy
        let keystream = self.state.generate_block();
        let mut new_key = [0u8; CHACHA20_KEY_SIZE];
        for i in 0..CHACHA20_KEY_SIZE {
            new_key[i] = fresh_entropy[i] ^ keystream[i];
        }

        self.nonce_counter = self.nonce_counter.wrapping_add(1);
        let mut nonce = [0u8; CHACHA20_NONCE_SIZE];
        let nc_bytes = self.nonce_counter.to_le_bytes();
        nonce[0..8].copy_from_slice(&nc_bytes);
        nonce[8..12].copy_from_slice(&[0x13, 0x37, 0xC0, 0xDE]);

        self.state = ChaCha20State::new(&new_key, &nonce, 0);
        self.bytes_generated = 0;
        self.refresh_buffer();
    }

    /// Fills the destination buffer with cryptographically secure random bytes.
    pub fn fill_bytes(&mut self, dest: &mut [u8]) {
        let mut written = 0;
        while written < dest.len() {
            // Check reseed boundary
            if self.bytes_generated >= self.reseed_interval {
                self.reseed();
            }

            if self.buffer_index >= CHACHA20_BLOCK_SIZE {
                self.refresh_buffer();
            }

            let available = CHACHA20_BLOCK_SIZE - self.buffer_index;
            let needed = dest.len() - written;
            let to_copy = core::cmp::min(available, needed);

            dest[written..written + to_copy]
                .copy_from_slice(&self.buffer[self.buffer_index..self.buffer_index + to_copy]);

            self.buffer_index += to_copy;
            written += to_copy;
            self.bytes_generated += to_copy;
        }
    }

    /// Generates next random 32-bit unsigned integer.
    pub fn next_u32(&mut self) -> u32 {
        let mut buf = [0u8; 4];
        self.fill_bytes(&mut buf);
        u32::from_le_bytes(buf)
    }

    /// Generates next random 64-bit unsigned integer.
    pub fn next_u64(&mut self) -> u64 {
        let mut buf = [0u8; 8];
        self.fill_bytes(&mut buf);
        u64::from_le_bytes(buf)
    }
}

/// Global thread-safe kernel CSPRNG instance.
pub static KERNEL_RNG: Spinlock<Option<ChaCha20Csprng>> = Spinlock::new(None);

/// Initializes the global kernel random number generator.
pub fn init_kernel_rng() {
    let csprng = ChaCha20Csprng::new();
    *KERNEL_RNG.lock() = Some(csprng);
}

/// Fills destination slice with cryptographically secure pseudorandom bytes.
pub fn get_random_bytes(dest: &mut [u8]) {
    let mut lock = KERNEL_RNG.lock();
    if let Some(rng) = lock.as_mut() {
        rng.fill_bytes(dest);
    } else {
        let mut rng = ChaCha20Csprng::new();
        rng.fill_bytes(dest);
        *lock = Some(rng);
    }
}

/// Generates a random 64-bit unsigned integer from the global CSPRNG.
pub fn get_random_u64() -> u64 {
    let mut buf = [0u8; 8];
    get_random_bytes(&mut buf);
    u64::from_le_bytes(buf)
}

/// Generates a random 32-bit unsigned integer from the global CSPRNG.
pub fn get_random_u32() -> u32 {
    let mut buf = [0u8; 4];
    get_random_bytes(&mut buf);
    u32::from_le_bytes(buf)
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

    let (w1, src1) = extractor.extract_u64();
    let (w2, _) = extractor.extract_u64();

    if w1 == w2 || (w1 == 0 && w2 == 0) {
        return false;
    }

    let mut seed = [0u8; 32];
    let seed_src = extractor.extract_seed(&mut seed);

    let all_zero = seed.iter().all(|&b| b == 0);
    let all_same = seed.iter().all(|&b| b == seed[0]);
    if all_zero || all_same {
        return false;
    }

    let _ = (src1, seed_src);
    true
}

/// Self-test verifying ChaCha20 CSPRNG stream generation, uniqueness, and reseeding.
pub fn verify_chacha20_csprng() -> bool {
    let mut rng = ChaCha20Csprng::new();

    // 1. Verify two consecutive 32-byte chunks are distinct
    let mut buf1 = [0u8; 32];
    let mut buf2 = [0u8; 32];
    rng.fill_bytes(&mut buf1);
    rng.fill_bytes(&mut buf2);

    if buf1 == buf2 {
        return false;
    }
    if buf1.iter().all(|&b| b == 0) || buf2.iter().all(|&b| b == 0) {
        return false;
    }

    // 2. Verify reseed logic
    rng.reseed();
    let mut buf3 = [0u8; 32];
    rng.fill_bytes(&mut buf3);
    if buf3 == buf1 || buf3 == buf2 {
        return false;
    }

    // 3. Verify scalar generators
    let u1 = rng.next_u64();
    let u2 = rng.next_u64();
    if u1 == u2 || (u1 == 0 && u2 == 0) {
        return false;
    }

    true
}
