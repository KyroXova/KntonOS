//! Kernel Cryptographic Primitives Subsystem.
//!
//! Provides zero-dependency cryptographic engines for KntonOS network security:
//! - SHA-256 (FIPS 180-4 / RFC 6234) message digest & HMAC-SHA256 (RFC 2104)
//! - HKDF (RFC 5869) key derivation functions (Extract & Expand)
//! - AES-128-GCM (NIST SP 800-38D) authenticated encryption and GHASH authenticator

pub mod aes_gcm;
pub mod hkdf;
pub mod sha256;

pub use aes_gcm::{
    ghash, ghash_multiply, verify_aes_gcm, Aes128, Aes128Gcm, AES128_KEY_SIZE, AES_BLOCK_SIZE,
    GCM_NONCE_SIZE, GCM_TAG_SIZE,
};
pub use hkdf::{hkdf_expand, hkdf_extract, verify_hkdf};
pub use sha256::{
    hmac_sha256, verify_sha256, Sha256, SHA256_BLOCK_SIZE, SHA256_DIGEST_SIZE,
};

/// Verifies core cryptographic primitives against standard test vectors.
pub fn verify_crypto_primitives() -> bool {
    verify_sha256() && verify_hkdf() && verify_aes_gcm()
}
