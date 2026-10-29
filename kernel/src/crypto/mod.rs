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
