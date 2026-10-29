//! SHA-256 Cryptographic Hash Engine (FIPS 180-4 / RFC 6234) & HMAC-SHA256.
//!
//! Provides a zero-dependency, pure-Rust implementation of the SHA-256 message
//! digest algorithm and Keyed-Hash Message Authentication Code (HMAC-SHA256).

pub const SHA256_BLOCK_SIZE: usize = 64;
pub const SHA256_DIGEST_SIZE: usize = 32;

/// Round constants for SHA-256 (first 32 bits of fractional parts of cube roots of first 64 primes).
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Initial hash values (first 32 bits of fractional parts of square roots of first 8 primes).
const H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
    0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

#[inline(always)]
fn rotr(x: u32, n: u32) -> u32 {
    (x >> n) | (x << (32 - n))
}

#[inline(always)]
fn ch(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (!x & z)
}

#[inline(always)]
fn maj(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (x & z) ^ (y & z)
}

#[inline(always)]
fn sigma0(x: u32) -> u32 {
    rotr(x, 2) ^ rotr(x, 13) ^ rotr(x, 22)
}

#[inline(always)]
fn sigma1(x: u32) -> u32 {
    rotr(x, 6) ^ rotr(x, 11) ^ rotr(x, 25)
}

#[inline(always)]
fn gamma0(x: u32) -> u32 {
    rotr(x, 7) ^ rotr(x, 18) ^ (x >> 3)
}

#[inline(always)]
fn gamma1(x: u32) -> u32 {
    rotr(x, 17) ^ rotr(x, 19) ^ (x >> 10)
}

/// Incremental SHA-256 Digest State Machine.
#[derive(Clone)]
pub struct Sha256 {
    state: [u32; 8],
    buffer: [u8; SHA256_BLOCK_SIZE],
    buffer_len: usize,
    total_len: u64,
}

impl Sha256 {
    pub const fn new() -> Self {
        Self {
            state: H0,
            buffer: [0u8; SHA256_BLOCK_SIZE],
            buffer_len: 0,
            total_len: 0,
        }
    }

    /// Appends data bytes to the active hash context.
    pub fn update(&mut self, data: &[u8]) {
        let mut offset = 0;
        self.total_len += data.len() as u64;

        // If bytes already in buffer, try to complete a 64-byte block
        if self.buffer_len > 0 {
            let to_copy = (SHA256_BLOCK_SIZE - self.buffer_len).min(data.len());
            self.buffer[self.buffer_len..self.buffer_len + to_copy]
                .copy_from_slice(&data[..to_copy]);
            self.buffer_len += to_copy;
            offset += to_copy;

            if self.buffer_len == SHA256_BLOCK_SIZE {
                let block = self.buffer;
                self.process_block(&block);
                self.buffer_len = 0;
            }
        }

        // Process full 64-byte blocks directly
        while offset + SHA256_BLOCK_SIZE <= data.len() {
            let mut block = [0u8; SHA256_BLOCK_SIZE];
            block.copy_from_slice(&data[offset..offset + SHA256_BLOCK_SIZE]);
            self.process_block(&block);
            offset += SHA256_BLOCK_SIZE;
        }

        // Store remaining bytes in buffer
        if offset < data.len() {
            let rem = data.len() - offset;
            self.buffer[..rem].copy_from_slice(&data[offset..]);
            self.buffer_len = rem;
        }
    }

    /// Finalizes hashing with standard SHA-256 padding and outputs the 32-byte digest.
    pub fn finalize(mut self) -> [u8; SHA256_DIGEST_SIZE] {
        let bit_len = self.total_len * 8;

        // 1. Append bit '1' (0x80)
        self.buffer[self.buffer_len] = 0x80;
        self.buffer_len += 1;

        // 2. If not enough room for 8-byte length at end, pad with zeros and process block
        if self.buffer_len > 56 {
            for b in &mut self.buffer[self.buffer_len..SHA256_BLOCK_SIZE] {
                *b = 0;
            }
            let block = self.buffer;
            self.process_block(&block);
            self.buffer_len = 0;
        }

        // 3. Pad zeros up to byte 56
        for b in &mut self.buffer[self.buffer_len..56] {
            *b = 0;
        }

        // 4. Append 64-bit big-endian length
        let len_bytes = bit_len.to_be_bytes();
        self.buffer[56..64].copy_from_slice(&len_bytes);
        let block = self.buffer;
        self.process_block(&block);

        // 5. Serialize state to big-endian bytes
        let mut digest = [0u8; SHA256_DIGEST_SIZE];
        for i in 0..8 {
            let bytes = self.state[i].to_be_bytes();
            digest[i * 4..(i + 1) * 4].copy_from_slice(&bytes);
        }

        digest
    }

    /// One-shot convenience function returning the SHA-256 digest of `data`.
    pub fn digest(data: &[u8]) -> [u8; SHA256_DIGEST_SIZE] {
        let mut hasher = Self::new();
        hasher.update(data);
        hasher.finalize()
    }

    /// Internal 64-round SHA-256 compression function.
    fn process_block(&mut self, block: &[u8; SHA256_BLOCK_SIZE]) {
        let mut w = [0u32; 64];

        // Prepare message schedule W[0..15]
        for t in 0..16 {
            let offset = t * 4;
            w[t] = u32::from_be_bytes([
                block[offset],
                block[offset + 1],
                block[offset + 2],
                block[offset + 3],
            ]);
        }

        // Extend message schedule W[16..63]
        for t in 16..64 {
            let s1 = gamma1(w[t - 2]);
            let s0 = gamma0(w[t - 15]);
            w[t] = w[t - 16].wrapping_add(s0).wrapping_add(w[t - 7]).wrapping_add(s1);
        }

        // Initialize working variables
        let mut a = self.state[0];
        let mut b = self.state[1];
        let mut c = self.state[2];
        let mut d = self.state[3];
        let mut e = self.state[4];
        let mut f = self.state[5];
        let mut g = self.state[6];
        let mut h = self.state[7];

        // 64 compression rounds
        for t in 0..64 {
            let t1 = h
                .wrapping_add(sigma1(e))
                .wrapping_add(ch(e, f, g))
                .wrapping_add(K[t])
                .wrapping_add(w[t]);
            let t2 = sigma0(a).wrapping_add(maj(a, b, c));

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }

        // Compute intermediate hash value
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }
}

/// Computes HMAC-SHA256 (RFC 2104) over `data` using secret `key`.
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; SHA256_DIGEST_SIZE] {
    let mut k_pad = [0u8; SHA256_BLOCK_SIZE];

    if key.len() > SHA256_BLOCK_SIZE {
        let digest = Sha256::digest(key);
        k_pad[..SHA256_DIGEST_SIZE].copy_from_slice(&digest);
    } else {
        k_pad[..key.len()].copy_from_slice(key);
    }

    let mut i_key_pad = [0u8; SHA256_BLOCK_SIZE];
    let mut o_key_pad = [0u8; SHA256_BLOCK_SIZE];

    for i in 0..SHA256_BLOCK_SIZE {
        i_key_pad[i] = k_pad[i] ^ 0x36;
        o_key_pad[i] = k_pad[i] ^ 0x5c;
    }

    // Inner hash: H(K_i || data)
    let mut inner = Sha256::new();
    inner.update(&i_key_pad);
    inner.update(data);
    let inner_hash = inner.finalize();

    // Outer hash: H(K_o || inner_hash)
    let mut outer = Sha256::new();
    outer.update(&o_key_pad);
    outer.update(&inner_hash);
    outer.finalize()
}

/// Self-test verifying SHA-256 against NIST FIPS 180-4 test vectors.
pub fn verify_sha256() -> bool {
    // 1. Vector: "" (empty string)
    let expected_empty: [u8; 32] = [
        0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14,
        0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f, 0xb9, 0x24,
        0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c,
        0xa4, 0x95, 0x99, 0x1b, 0x78, 0x52, 0xb8, 0x55,
    ];
    if Sha256::digest(b"") != expected_empty {
        return false;
    }

    // 2. Vector: "abc"
    let expected_abc: [u8; 32] = [
        0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea,
        0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23,
        0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c,
        0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
    ];
    if Sha256::digest(b"abc") != expected_abc {
        return false;
    }

    // 3. Multi-block vector: "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq" (56 bytes)
    let msg56 = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
    let expected_56: [u8; 32] = [
        0x24, 0x8d, 0x6a, 0x61, 0xd2, 0x06, 0x38, 0xb8,
        0xe5, 0xc0, 0x26, 0x93, 0x0c, 0x3e, 0x60, 0x39,
        0xa3, 0x3c, 0xe4, 0x59, 0x64, 0xff, 0x21, 0x67,
        0xf6, 0xec, 0xed, 0xd4, 0x19, 0xdb, 0x06, 0xc1,
    ];
    if Sha256::digest(msg56) != expected_56 {
        return false;
    }

    // 4. HMAC-SHA256 test: Key = "key", Data = "The quick brown fox jumps over the lazy dog"
    // HMAC = f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8
    let expected_hmac: [u8; 32] = [
        0xf7, 0xbc, 0x83, 0xf4, 0x30, 0x53, 0x84, 0x24,
        0xb1, 0x32, 0x98, 0xe6, 0xaa, 0x6f, 0xb1, 0x43,
        0xef, 0x4d, 0x59, 0xa1, 0x49, 0x46, 0x17, 0x59,
        0x97, 0x47, 0x9d, 0xbc, 0x2d, 0x1a, 0x3c, 0xd8,
    ];
    if hmac_sha256(b"key", b"The quick brown fox jumps over the lazy dog") != expected_hmac {
        return false;
    }

    true
}
