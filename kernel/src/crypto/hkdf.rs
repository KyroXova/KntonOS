//! HMAC-based Extract-and-Expand Key Derivation Function (HKDF) (RFC 5869).
//!
//! Implements HKDF-Extract and HKDF-Expand primitives utilizing SHA-256 for
//! secure key derivation in TLS 1.3 cryptographic handshakes.

use super::sha256::{hmac_sha256, SHA256_DIGEST_SIZE};
use alloc::vec::Vec;

/// HKDF-Extract(salt, IKM) -> PRK (Pseudorandom Key).
///
/// If `salt` is empty, a string of 32 zero bytes is used per RFC 5869 Section 2.2.
pub fn hkdf_extract(salt: &[u8], ikm: &[u8]) -> [u8; SHA256_DIGEST_SIZE] {
    let default_salt = [0u8; SHA256_DIGEST_SIZE];
    let effective_salt = if salt.is_empty() {
        &default_salt[..]
    } else {
        salt
    };

    hmac_sha256(effective_salt, ikm)
}

/// HKDF-Expand(PRK, info, L) -> OKM (Output Keying Material).
///
/// Fills `okm` with cryptographically derived bytes of length `L = okm.len()`.
pub fn hkdf_expand(prk: &[u8; SHA256_DIGEST_SIZE], info: &[u8], okm: &mut [u8]) -> Result<(), &'static str> {
    let l = okm.len();
    if l == 0 {
        return Ok(());
    }

    let n = (l + SHA256_DIGEST_SIZE - 1) / SHA256_DIGEST_SIZE;
    if n > 255 {
        return Err("Requested output keying material length exceeds HKDF limit (255 * HashLen)");
    }

    let mut t = [0u8; SHA256_DIGEST_SIZE];
    let mut t_len = 0;
    let mut out_offset = 0;

    for i in 1..=n {
        let mut step_input = Vec::with_capacity(t_len + info.len() + 1);
        if t_len > 0 {
            step_input.extend_from_slice(&t[..t_len]);
        }
        step_input.extend_from_slice(info);
        step_input.push(i as u8);

        t = hmac_sha256(prk, &step_input);
        t_len = SHA256_DIGEST_SIZE;

        let to_copy = (l - out_offset).min(SHA256_DIGEST_SIZE);
        okm[out_offset..out_offset + to_copy].copy_from_slice(&t[..to_copy]);
        out_offset += to_copy;
    }

    Ok(())
}

/// Self-test verifying HKDF implementation against RFC 5869 Test Case 1.
pub fn verify_hkdf() -> bool {
    // RFC 5869 Test Case 1:
    // IKM  = 0x0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b (22 octets)
    let ikm = [0x0bu8; 22];

    // salt = 0x000102030405060708090a0b0c (13 octets)
    let salt: [u8; 13] = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
        0x08, 0x09, 0x0a, 0x0b, 0x0c,
    ];

    // info = 0xf0f1f2f3f4f5f6f7f8f9 (10 octets)
    let info: [u8; 10] = [
        0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9,
    ];

    // Expected PRK (32 octets)
    let expected_prk: [u8; 32] = [
        0x07, 0x77, 0x09, 0x36, 0x2c, 0x2e, 0x32, 0xdf,
        0x0d, 0xdc, 0x3f, 0x0d, 0xc4, 0x7b, 0xba, 0x63,
        0x90, 0xb6, 0xc7, 0x3b, 0xb5, 0x0f, 0x9c, 0x31,
        0x22, 0xec, 0x84, 0x4a, 0xd7, 0xc2, 0xb3, 0xe0,
    ];

    let prk = hkdf_extract(&salt, &ikm);
    if prk != expected_prk {
        return false;
    }

    // Expected OKM (42 octets)
    let expected_okm: [u8; 42] = [
        0x3c, 0xb2, 0x5f, 0x25, 0xfa, 0xac, 0xd5, 0x7a,
        0x90, 0x43, 0x4f, 0x64, 0xd0, 0x36, 0x2f, 0x2a,
        0x2d, 0x2d, 0x0a, 0x90, 0xcf, 0x1a, 0x5a, 0x4c,
        0x5d, 0xb0, 0x2d, 0x56, 0xec, 0xc4, 0xc5, 0xbf,
        0x34, 0x00, 0x72, 0x08, 0xd5, 0xb8, 0x87, 0x18,
        0x58, 0x65,
    ];

    let mut okm = [0u8; 42];
    if hkdf_expand(&prk, &info, &mut okm).is_err() {
        return false;
    }

    if okm != expected_okm {
        return false;
    }

    true
}
