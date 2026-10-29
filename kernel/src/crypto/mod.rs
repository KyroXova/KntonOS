//! Kernel Cryptographic Primitives Subsystem.
//!
//! Provides zero-dependency cryptographic engines for KntonOS network security:
//! - SHA-256 (FIPS 180-4 / RFC 6234) message digest & HMAC-SHA256 (RFC 2104)
//! - HKDF (RFC 5869) key derivation functions (Extract & Expand)
//! - AES-128-GCM (NIST SP 800-38D) authenticated encryption and GHASH authenticator
//! - Hardware RNG (RDRAND/RDSEED) detection, entropy extractor, and ChaCha20 cipher state structures
//! - ChaCha20 256-bit CSPRNG stream generator and kernel entropy pool

pub mod aes_gcm;
pub mod chacha20;
pub mod hkdf;
pub mod rng;
pub mod sha256;

pub use aes_gcm::{
    ghash, ghash_multiply, verify_aes_gcm, Aes128, Aes128Gcm, AES128_KEY_SIZE, AES_BLOCK_SIZE,
    GCM_NONCE_SIZE, GCM_TAG_SIZE,
};
pub use chacha20::{
    verify_chacha20_structures, ChaCha20State, CHACHA20_BLOCK_SIZE, CHACHA20_KEY_SIZE,
    CHACHA20_NONCE_SIZE, CONST_0, CONST_1, CONST_2, CONST_3,
};
pub use hkdf::{hkdf_expand, hkdf_extract, verify_hkdf};
pub use rng::{
    cpuid, get_random_bytes, get_random_u32, get_random_u64, init_kernel_rng, rdrand64, rdseed64,
    rdtsc, verify_chacha20_csprng, verify_entropy_extractor, verify_hw_rng_detection,
    ChaCha20Csprng, EntropySource, HardwareEntropyExtractor, HardwareRngCapabilities,
    KernelEntropyPool, KERNEL_RNG,
};
pub use sha256::{
    hmac_sha256, verify_sha256, Sha256, SHA256_BLOCK_SIZE, SHA256_DIGEST_SIZE,
};

/// Verifies core cryptographic primitives against standard test vectors.
pub fn verify_crypto_primitives() -> bool {
    verify_sha256()
        && verify_hkdf()
        && verify_aes_gcm()
        && verify_chacha20_structures()
        && verify_hw_rng_detection()
        && verify_entropy_extractor()
        && verify_chacha20_csprng()
}

/// Comprehensive verification of the hardware entropy extractor and ChaCha20 CSPRNG.
///
/// Tests CPUID detection, RDRAND/RDSEED/RDTSC extraction, RFC 7539 test vectors,
/// ChaCha20 20-round keystream generation, monobit statistical balance,
/// byte frequency distribution, and non-repeating sequence uniqueness.
pub fn test_crypto_rng_subsystem() -> bool {
    // 1. Verify underlying cryptographic primitives and RFC vectors
    if !verify_crypto_primitives() {
        return false;
    }

    // 2. Statistical Monobit (Frequency) Test across 1024 bytes (8192 bits)
    let mut random_sample = [0u8; 1024];
    get_random_bytes(&mut random_sample);

    let mut ones_count: u32 = 0;
    for &byte in &random_sample {
        ones_count += byte.count_ones();
    }

    // Expected ones for 8192 random bits: ~4096. Range 3700..=4500 (45.1% - 54.9%)
    if ones_count < 3700 || ones_count > 4500 {
        return false;
    }

    // 3. Byte Value Diversity / Frequency Distribution Test
    let mut seen = [false; 256];
    let mut unique_bytes = 0;
    for &byte in &random_sample {
        if !seen[byte as usize] {
            seen[byte as usize] = true;
            unique_bytes += 1;
        }
    }
    // Across 1024 random samples with replacement from 256 items,
    // expected unique count is ~251. We require >= 200 distinct values.
    if unique_bytes < 200 {
        return false;
    }

    // 4. Non-Repeating Random Sequence Verification
    let mut seq = [0u64; 8];
    for i in 0..8 {
        seq[i] = get_random_u64();
        if seq[i] == 0 {
            return false;
        }
    }
    // Verify all 8 samples are pairwise unique
    for i in 0..8 {
        for j in (i + 1)..8 {
            if seq[i] == seq[j] {
                return false;
            }
        }
    }

    // 5. Output boot sentinel verification
    crate::serial_write_str("[KNT-CRYPTO-RNG-PASS] Hardware entropy and ChaCha20 CSPRNG verified\r\n");

    true
}
