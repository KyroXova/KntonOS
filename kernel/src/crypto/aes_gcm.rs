//! AES-128-GCM Authenticated Encryption with Associated Data (AEAD) (NIST SP 800-38D).
//!
//! Provides a zero-dependency, pure-Rust implementation of:
//! - AES-128 (FIPS 197) block cipher (10 rounds, 128-bit block and key)
//! - GHASH 128-bit Galois Field GF(2^128) authenticator
//! - Galois/Counter Mode (GCM) AEAD encryption and decryption

use alloc::vec::Vec;

pub const AES128_KEY_SIZE: usize = 16;
pub const AES_BLOCK_SIZE: usize = 16;
pub const GCM_NONCE_SIZE: usize = 12;
pub const GCM_TAG_SIZE: usize = 16;

/// Rijndael S-box substitution table.
const SBOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5e, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];

/// Round constant table for AES key expansion.
const RCON: [u8; 11] = [
    0x00, 0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x1b, 0x36,
];

#[inline(always)]
fn xtime(x: u8) -> u8 {
    (x << 1) ^ (if (x & 0x80) != 0 { 0x1b } else { 0x00 })
}

/// AES-128 block cipher state with precomputed round keys.
#[derive(Clone)]
pub struct Aes128 {
    round_keys: [u8; 176], // 11 round keys of 16 bytes each
}

impl Aes128 {
    /// Initializes AES-128 context and derives round keys via standard key expansion.
    pub fn new(key: &[u8; AES128_KEY_SIZE]) -> Self {
        let mut round_keys = [0u8; 176];
        round_keys[..16].copy_from_slice(key);

        let mut rcon_idx = 1;
        let mut i = 16;
        while i < 176 {
            let mut temp = [
                round_keys[i - 4],
                round_keys[i - 3],
                round_keys[i - 2],
                round_keys[i - 1],
            ];

            if i % 16 == 0 {
                // RotWord: [k0, k1, k2, k3] -> [k1, k2, k3, k0]
                let t = temp[0];
                temp[0] = temp[1];
                temp[1] = temp[2];
                temp[2] = temp[3];
                temp[3] = t;

                // SubWord
                temp[0] = SBOX[temp[0] as usize];
                temp[1] = SBOX[temp[1] as usize];
                temp[2] = SBOX[temp[2] as usize];
                temp[3] = SBOX[temp[3] as usize];

                temp[0] ^= RCON[rcon_idx];
                rcon_idx += 1;
            }

            for j in 0..4 {
                round_keys[i + j] = round_keys[i - 16 + j] ^ temp[j];
            }
            i += 4;
        }

        Self { round_keys }
    }

    /// Encrypts a single 16-byte block in-place.
    pub fn encrypt_block(&self, block: &mut [u8; AES_BLOCK_SIZE]) {
        // AddRoundKey for round 0
        for i in 0..16 {
            block[i] ^= self.round_keys[i];
        }

        // Rounds 1 through 9
        for round in 1..10 {
            let rk_offset = round * 16;

            // SubBytes
            for b in block.iter_mut() {
                *b = SBOX[*b as usize];
            }

            // ShiftRows
            let s0 = block[0];
            let s1 = block[5];
            let s2 = block[10];
            let s3 = block[15];

            let s4 = block[4];
            let s5 = block[9];
            let s6 = block[14];
            let s7 = block[3];

            let s8 = block[8];
            let s9 = block[13];
            let s10 = block[2];
            let s11 = block[7];

            let s12 = block[12];
            let s13 = block[1];
            let s14 = block[6];
            let s15 = block[11];

            // MixColumns
            // Col 0
            block[0] = xtime(s0) ^ (s4 ^ xtime(s4)) ^ s8 ^ s12;
            block[4] = s0 ^ xtime(s4) ^ (s8 ^ xtime(s8)) ^ s12;
            block[8] = s0 ^ s4 ^ xtime(s8) ^ (s12 ^ xtime(s12));
            block[12] = (s0 ^ xtime(s0)) ^ s4 ^ s8 ^ xtime(s12);

            // Col 1
            block[1] = xtime(s13) ^ (s1 ^ xtime(s1)) ^ s5 ^ s9;
            block[5] = s13 ^ xtime(s1) ^ (s5 ^ xtime(s5)) ^ s9;
            block[9] = s13 ^ s1 ^ xtime(s5) ^ (s9 ^ xtime(s9));
            block[13] = (s13 ^ xtime(s13)) ^ s1 ^ s5 ^ xtime(s9);

            // Col 2
            block[2] = xtime(s10) ^ (s14 ^ xtime(s14)) ^ s2 ^ s6;
            block[6] = s10 ^ xtime(s14) ^ (s2 ^ xtime(s2)) ^ s6;
            block[10] = s10 ^ s14 ^ xtime(s2) ^ (s6 ^ xtime(s6));
            block[14] = (s10 ^ xtime(s10)) ^ s14 ^ s2 ^ xtime(s6);

            // Col 3
            block[3] = xtime(s7) ^ (s11 ^ xtime(s11)) ^ s15 ^ s3;
            block[7] = s7 ^ xtime(s11) ^ (s15 ^ xtime(s15)) ^ s3;
            block[11] = s7 ^ s11 ^ xtime(s15) ^ (s3 ^ xtime(s3));
            block[15] = (s7 ^ xtime(s7)) ^ s11 ^ s15 ^ xtime(s3);

            // AddRoundKey
            for i in 0..16 {
                block[i] ^= self.round_keys[rk_offset + i];
            }
        }

        // Final round (Round 10, no MixColumns)
        for b in block.iter_mut() {
            *b = SBOX[*b as usize];
        }

        // ShiftRows
        let s0 = block[0];
        let s1 = block[5];
        let s2 = block[10];
        let s3 = block[15];
        let s4 = block[4];
        let s5 = block[9];
        let s6 = block[14];
        let s7 = block[3];
        let s8 = block[8];
        let s9 = block[13];
        let s10 = block[2];
        let s11 = block[7];
        let s12 = block[12];
        let s13 = block[1];
        let s14 = block[6];
        let s15 = block[11];

        block[0] = s0;
        block[1] = s13;
        block[2] = s10;
        block[3] = s7;
        block[4] = s4;
        block[5] = s1;
        block[6] = s14;
        block[7] = s11;
        block[8] = s8;
        block[9] = s5;
        block[10] = s2;
        block[11] = s15;
        block[12] = s12;
        block[13] = s9;
        block[14] = s6;
        block[15] = s3;

        // AddRoundKey for round 10
        for i in 0..16 {
            block[i] ^= self.round_keys[160 + i];
        }
    }
}

/// GHASH 128-bit Galois Field GF(2^128) multiplication: x = x * h.
/// Reduction polynomial: f(x) = x^128 + x^7 + x^2 + x + 1 (0xE1 constant).
pub fn ghash_multiply(x: &mut [u8; 16], h: &[u8; 16]) {
    let mut z = [0u8; 16];
    let mut v = *h;

    for i in 0..128 {
        let byte_idx = i / 8;
        let bit_idx = 7 - (i % 8);
        if (x[byte_idx] >> bit_idx) & 1 == 1 {
            for j in 0..16 {
                z[j] ^= v[j];
            }
        }

        let lsb = v[15] & 1;
        for j in (1..16).rev() {
            v[j] = (v[j] >> 1) | ((v[j - 1] & 1) << 7);
        }
        v[0] >>= 1;
        if lsb == 1 {
            v[0] ^= 0xe1;
        }
    }

    *x = z;
}

/// Computes GHASH over authenticated data (A) and ciphertext (C) using hash subkey H.
pub fn ghash(h: &[u8; 16], aad: &[u8], ciphertext: &[u8]) -> [u8; 16] {
    let mut y = [0u8; 16];

    // Process AAD blocks (padded to 16 bytes)
    let mut offset = 0;
    while offset < aad.len() {
        let chunk_len = (aad.len() - offset).min(16);
        for i in 0..chunk_len {
            y[i] ^= aad[offset + i];
        }
        ghash_multiply(&mut y, h);
        offset += chunk_len;
    }

    // Process Ciphertext blocks (padded to 16 bytes)
    offset = 0;
    while offset < ciphertext.len() {
        let chunk_len = (ciphertext.len() - offset).min(16);
        for i in 0..chunk_len {
            y[i] ^= ciphertext[offset + i];
        }
        ghash_multiply(&mut y, h);
        offset += chunk_len;
    }

    // Process length block: len(A) in bits || len(C) in bits (64-bit big endian each)
    let aad_bits = (aad.len() as u64) * 8;
    let ct_bits = (ciphertext.len() as u64) * 8;
    let mut len_block = [0u8; 16];
    len_block[..8].copy_from_slice(&aad_bits.to_be_bytes());
    len_block[8..].copy_from_slice(&ct_bits.to_be_bytes());

    for i in 0..16 {
        y[i] ^= len_block[i];
    }
    ghash_multiply(&mut y, h);

    y
}

/// AES-128-GCM Authenticated Encryption with Associated Data engine.
#[derive(Clone)]
pub struct Aes128Gcm {
    cipher: Aes128,
    h: [u8; 16], // Hash subkey H = AES_K(0^16)
}

impl Aes128Gcm {
    /// Creates a new AES-128-GCM instance with the specified 128-bit key.
    pub fn new(key: &[u8; AES128_KEY_SIZE]) -> Self {
        let cipher = Aes128::new(key);
        let mut h = [0u8; 16];
        cipher.encrypt_block(&mut h);
        Self { cipher, h }
    }

    /// Increments 32-bit big-endian counter at the end of a 16-byte counter block.
    fn inc32(counter: &mut [u8; 16]) {
        let mut val = u32::from_be_bytes([counter[12], counter[13], counter[14], counter[15]]);
        val = val.wrapping_add(1);
        counter[12..16].copy_from_slice(&val.to_be_bytes());
    }

    /// Prepares initial counter J0 for a 96-bit (12-byte) nonce: J0 = IV || 0x00000001.
    fn init_j0(nonce: &[u8; GCM_NONCE_SIZE]) -> [u8; 16] {
        let mut j0 = [0u8; 16];
        j0[..12].copy_from_slice(nonce);
        j0[15] = 1;
        j0
    }

    /// Encrypts `plaintext` with authenticated additional data (`aad`), returning `(ciphertext, tag)`.
    pub fn encrypt(&self, nonce: &[u8; GCM_NONCE_SIZE], aad: &[u8], plaintext: &[u8]) -> (Vec<u8>, [u8; GCM_TAG_SIZE]) {
        let j0 = Self::init_j0(nonce);

        // Compute tag mask S_0 = AES_K(J0)
        let mut s0 = j0;
        self.cipher.encrypt_block(&mut s0);

        // Counter CTR encryption
        let mut counter = j0;
        let mut ciphertext = Vec::with_capacity(plaintext.len());
        let mut offset = 0;

        while offset < plaintext.len() {
            Self::inc32(&mut counter);
            let mut keystream = counter;
            self.cipher.encrypt_block(&mut keystream);

            let chunk_len = (plaintext.len() - offset).min(16);
            for i in 0..chunk_len {
                ciphertext.push(plaintext[offset + i] ^ keystream[i]);
            }
            offset += chunk_len;
        }

        // Compute GHASH and authentication tag T = GHASH_H(A, C) ^ S_0
        let ghash_out = ghash(&self.h, aad, &ciphertext);
        let mut tag = [0u8; 16];
        for i in 0..16 {
            tag[i] = ghash_out[i] ^ s0[i];
        }

        (ciphertext, tag)
    }

    /// Decrypts `ciphertext` and validates 128-bit authentication `tag`.
    pub fn decrypt(
        &self,
        nonce: &[u8; GCM_NONCE_SIZE],
        aad: &[u8],
        ciphertext: &[u8],
        tag: &[u8; GCM_TAG_SIZE],
    ) -> Result<Vec<u8>, &'static str> {
        let j0 = Self::init_j0(nonce);

        // Compute tag mask S_0 = AES_K(J0)
        let mut s0 = j0;
        self.cipher.encrypt_block(&mut s0);

        // Verify authentication tag before or after decryption
        let ghash_out = ghash(&self.h, aad, ciphertext);
        let mut expected_tag = [0u8; 16];
        for i in 0..16 {
            expected_tag[i] = ghash_out[i] ^ s0[i];
        }

        // Constant-time tag comparison
        let mut diff = 0u8;
        for i in 0..16 {
            diff |= expected_tag[i] ^ tag[i];
        }
        if diff != 0 {
            return Err("AES-128-GCM authentication tag mismatch");
        }

        // Counter CTR decryption
        let mut counter = j0;
        let mut plaintext = Vec::with_capacity(ciphertext.len());
        let mut offset = 0;

        while offset < ciphertext.len() {
            Self::inc32(&mut counter);
            let mut keystream = counter;
            self.cipher.encrypt_block(&mut keystream);

            let chunk_len = (ciphertext.len() - offset).min(16);
            for i in 0..chunk_len {
                plaintext.push(ciphertext[offset + i] ^ keystream[i]);
            }
            offset += chunk_len;
        }

        Ok(plaintext)
    }
}

/// Self-test verifying AES-128 block cipher and AES-128-GCM AEAD against NIST SP 800-38D test vectors.
pub fn verify_aes_gcm() -> bool {
    // 1. AES-128 Block Cipher Test (FIPS 197 Appendix B):
    // Key: 2b7e151628aed2a6abf7158809cf4f3c
    // Plaintext: 3243f6a8885a308d313198a2e0370734
    // Expected Ciphertext: 3925841d02dc09fbdc118597196a0b32
    let aes_key: [u8; 16] = [
        0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6,
        0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf, 0x4f, 0x3c,
    ];
    let mut aes_block: [u8; 16] = [
        0x32, 0x43, 0xf6, 0xa8, 0x88, 0x5a, 0x30, 0x8d,
        0x31, 0x31, 0x98, 0xa2, 0xe0, 0x37, 0x07, 0x34,
    ];
    let expected_aes_ct: [u8; 16] = [
        0x39, 0x25, 0x84, 0x1d, 0x02, 0xdc, 0x09, 0xfb,
        0xdc, 0x11, 0x85, 0x97, 0x19, 0x6a, 0x0b, 0x32,
    ];
    let cipher = Aes128::new(&aes_key);
    cipher.encrypt_block(&mut aes_block);
    if aes_block != expected_aes_ct {
        return false;
    }

    // 2. NIST SP 800-38D Test Case 2 (16 zero bytes pt, 16 zero bytes key, 12 zero bytes IV):
    // CT: 0388dace60b6a392f328c2b971b2fe78
    // Tag: ab6e47d42cec13bdf53a67b21257bddf
    let zero_key = [0u8; 16];
    let zero_nonce = [0u8; 12];
    let zero_pt = [0u8; 16];
    let gcm = Aes128Gcm::new(&zero_key);

    let (ct, tag) = gcm.encrypt(&zero_nonce, &[], &zero_pt);
    let expected_ct: [u8; 16] = [
        0x03, 0x88, 0xda, 0xce, 0x60, 0xb6, 0xa3, 0x92,
        0xf3, 0x28, 0xc2, 0xb9, 0x71, 0xb2, 0xfe, 0x78,
    ];
    let expected_tag: [u8; 16] = [
        0xab, 0x6e, 0x47, 0xd4, 0x2c, 0xec, 0x13, 0xbd,
        0xf5, 0x3a, 0x67, 0xb2, 0x12, 0x57, 0xbd, 0xdf,
    ];
    if ct != expected_ct || tag != expected_tag {
        return false;
    }

    // Decrypt and ensure plaintext roundtrip
    let decrypted = match gcm.decrypt(&zero_nonce, &[], &ct, &tag) {
        Ok(pt) => pt,
        Err(_) => return false,
    };
    if decrypted != zero_pt {
        return false;
    }

    // 3. NIST SP 800-38D Test Case with Nonce, AAD, and Plaintext:
    let key3: [u8; 16] = [
        0xfe, 0xff, 0xe9, 0x92, 0x86, 0x65, 0x73, 0x1c,
        0x6d, 0x6a, 0x8f, 0x94, 0x67, 0x30, 0x83, 0x08,
    ];
    let nonce3: [u8; 12] = [
        0xca, 0xfe, 0xba, 0xbe, 0xfa, 0xce, 0xdb, 0xad,
        0xde, 0xca, 0xf8, 0x88,
    ];
    let aad3: [u8; 20] = [
        0xfe, 0xed, 0xfa, 0xce, 0xde, 0xad, 0xbe, 0xef,
        0xfe, 0xed, 0xfa, 0xce, 0xde, 0xad, 0xbe, 0xef,
        0xab, 0xad, 0xda, 0xd2,
    ];
    let pt3: [u8; 64] = [
        0xd9, 0x31, 0x32, 0x25, 0xf8, 0x84, 0x06, 0xe5,
        0xa5, 0x59, 0x09, 0xc5, 0xaf, 0xf5, 0x26, 0x9a,
        0x86, 0xa7, 0xa9, 0x53, 0x15, 0x34, 0xf7, 0xda,
        0x2e, 0x4c, 0x30, 0x3d, 0x8a, 0x31, 0x8a, 0x72,
        0x1c, 0x3c, 0x0c, 0x95, 0x95, 0x68, 0x09, 0x53,
        0x2f, 0xcf, 0x0e, 0x24, 0x49, 0xa6, 0xb5, 0x25,
        0xb1, 0x6a, 0xed, 0xf5, 0xaa, 0x0d, 0xe6, 0x57,
        0xba, 0x63, 0x7b, 0x39, 0x1a, 0xaf, 0xd2, 0x55,
    ];
    let expected_tag3: [u8; 16] = [
        0xda, 0x80, 0xce, 0x83, 0x0c, 0xfd, 0xa0, 0x2d,
        0xa2, 0xa2, 0x18, 0xa1, 0x74, 0x4f, 0x4c, 0x76,
    ];

    let gcm3 = Aes128Gcm::new(&key3);
    let (ct3, tag3) = gcm3.encrypt(&nonce3, &aad3, &pt3);
    if tag3 != expected_tag3 {
        return false;
    }

    let dec3 = match gcm3.decrypt(&nonce3, &aad3, &ct3, &tag3) {
        Ok(p) => p,
        Err(_) => return false,
    };
    if dec3 != pt3 {
        return false;
    }

    // Corrupted tag must fail
    let mut bad_tag = tag3;
    bad_tag[0] ^= 0x01;
    if gcm3.decrypt(&nonce3, &aad3, &ct3, &bad_tag).is_ok() {
        return false;
    }

    true
}
