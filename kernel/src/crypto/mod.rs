//! Kernel Cryptographic Primitives Subsystem.
//!
//! Provides zero-dependency cryptographic engines for KntonOS network security:
//! - SHA-256 (FIPS 180-4 / RFC 6234) message digest & HMAC-SHA256 (RFC 2104)
//! - HKDF (RFC 5869) key derivation functions (Extract & Expand)

pub mod hkdf;
pub mod sha256;

pub use hkdf::{hkdf_expand, hkdf_extract, verify_hkdf};
pub use sha256::{
    hmac_sha256, verify_sha256, Sha256, SHA256_BLOCK_SIZE, SHA256_DIGEST_SIZE,
};

/// Verifies core cryptographic primitives against standard test vectors.
pub fn verify_crypto_primitives() -> bool {
    verify_sha256() && verify_hkdf()
}
