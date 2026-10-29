//! ChaCha20 Stream Cipher State and Quarter-Round Permutations (RFC 7539).
//!
//! Provides the 512-bit (16 x 32-bit word) ChaCha20 state matrix, 20-round ARX
//! (Add-Rotate-Xor) block function, and keystream generator.

pub const CHACHA20_KEY_SIZE: usize = 32;       // 256-bit key
pub const CHACHA20_NONCE_SIZE: usize = 12;     // 96-bit nonce
pub const CHACHA20_BLOCK_SIZE: usize = 64;     // 512-bit block (16 x u32)

/// ChaCha20 initial state constant words: "expand 32-byte k" in little-endian.
pub const CONST_0: u32 = 0x61707865; // "expa"
pub const CONST_1: u32 = 0x3320646e; // "nd 3"
pub const CONST_2: u32 = 0x79622d32; // "2-by"
pub const CONST_3: u32 = 0x6b206574; // "te k"

/// 512-bit ChaCha20 State Matrix (16 x 32-bit unsigned words).
///
/// Layout:
/// - Words 0..3: Constants ("expand 32-byte k")
/// - Words 4..11: 256-bit Key (8 words)
/// - Word 12: 32-bit Block Counter
/// - Words 13..15: 96-bit Nonce (3 words)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChaCha20State {
    pub state: [u32; 16],
}

impl ChaCha20State {
    /// Initializes state with standard constants, 256-bit key, 96-bit nonce, and block counter.
    pub fn new(key: &[u8; CHACHA20_KEY_SIZE], nonce: &[u8; CHACHA20_NONCE_SIZE], counter: u32) -> Self {
        let mut state = [0u32; 16];

        // 1. Constants: "expand 32-byte k"
        state[0] = CONST_0;
        state[1] = CONST_1;
        state[2] = CONST_2;
        state[3] = CONST_3;

        // 2. 256-bit Key (8 x u32 little-endian)
        for i in 0..8 {
            let offset = i * 4;
            state[4 + i] = u32::from_le_bytes([
                key[offset],
                key[offset + 1],
                key[offset + 2],
                key[offset + 3],
            ]);
        }

        // 3. Block Counter
        state[12] = counter;

        // 4. 96-bit Nonce (3 x u32 little-endian)
        for i in 0..3 {
            let offset = i * 4;
            state[13 + i] = u32::from_le_bytes([
                nonce[offset],
                nonce[offset + 1],
                nonce[offset + 2],
                nonce[offset + 3],
            ]);
        }

        Self { state }
    }

    /// Initializes state directly from 32-bit pre-parsed words.
    pub fn from_words(key_words: [u32; 8], nonce_words: [u32; 3], counter: u32) -> Self {
        let mut state = [0u32; 16];
        state[0] = CONST_0;
        state[1] = CONST_1;
        state[2] = CONST_2;
        state[3] = CONST_3;
        state[4..12].copy_from_slice(&key_words);
        state[12] = counter;
        state[13..16].copy_from_slice(&nonce_words);
        Self { state }
    }

    /// Sets the 32-bit block counter at word index 12.
    #[inline(always)]
    pub fn set_counter(&mut self, counter: u32) {
        self.state[12] = counter;
    }

    /// Gets the current 32-bit block counter.
    #[inline(always)]
    pub fn counter(&self) -> u32 {
        self.state[12]
    }

    /// Basic ARX (Add-Rotate-Xor) Quarter-Round function on four words (RFC 7539 Section 2.1).
    #[inline(always)]
    pub fn quarter_round(a: &mut u32, b: &mut u32, c: &mut u32, d: &mut u32) {
        *a = a.wrapping_add(*b);
        *d = (*d ^ *a).rotate_left(16);

        *c = c.wrapping_add(*d);
        *b = (*b ^ *c).rotate_left(12);

        *a = a.wrapping_add(*b);
        *d = (*d ^ *a).rotate_left(8);

        *c = c.wrapping_add(*d);
        *b = (*b ^ *c).rotate_left(7);
    }

    /// Executes the standard RFC 7539 ChaCha20 20-round block function.
    /// Performs 10 double-rounds (column rounds then diagonal rounds), adds the
    /// initial working state, and produces a 64-byte keystream block.
    pub fn block_function(&self) -> [u8; CHACHA20_BLOCK_SIZE] {
        let mut working = self.state;

        for _ in 0..10 {
            // Column Rounds
            Self::quarter_round_indexed(&mut working, 0, 4, 8, 12);
            Self::quarter_round_indexed(&mut working, 1, 5, 9, 13);
            Self::quarter_round_indexed(&mut working, 2, 6, 10, 14);
            Self::quarter_round_indexed(&mut working, 3, 7, 11, 15);

            // Diagonal Rounds
            Self::quarter_round_indexed(&mut working, 0, 5, 10, 15);
            Self::quarter_round_indexed(&mut working, 1, 6, 11, 12);
            Self::quarter_round_indexed(&mut working, 2, 7, 8, 13);
            Self::quarter_round_indexed(&mut working, 3, 4, 9, 14);
        }

        // Add initial state and serialize to 64 bytes
        let mut out = [0u8; CHACHA20_BLOCK_SIZE];
        for i in 0..16 {
            let sum = working[i].wrapping_add(self.state[i]);
            let le = sum.to_le_bytes();
            let offset = i * 4;
            out[offset..offset + 4].copy_from_slice(&le);
        }
        out
    }

    /// Helper for quarter-round indexing on a mutable state array.
    #[inline(always)]
    fn quarter_round_indexed(state: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
        state[a] = state[a].wrapping_add(state[b]);
        state[d] = (state[d] ^ state[a]).rotate_left(16);

        state[c] = state[c].wrapping_add(state[d]);
        state[b] = (state[b] ^ state[c]).rotate_left(12);

        state[a] = state[a].wrapping_add(state[b]);
        state[d] = (state[d] ^ state[a]).rotate_left(8);

        state[c] = state[c].wrapping_add(state[d]);
        state[b] = (state[b] ^ state[c]).rotate_left(7);
    }

    /// Generates next 64-byte keystream block and increments block counter.
    pub fn generate_block(&mut self) -> [u8; CHACHA20_BLOCK_SIZE] {
        let block = self.block_function();
        self.state[12] = self.state[12].wrapping_add(1);
        block
    }

    /// Encrypts or decrypts a buffer in-place by XORing with the generated keystream.
    pub fn apply_keystream(&mut self, data: &mut [u8]) {
        let mut offset = 0;
        while offset < data.len() {
            let block = self.generate_block();
            let chunk_len = core::cmp::min(CHACHA20_BLOCK_SIZE, data.len() - offset);
            for i in 0..chunk_len {
                data[offset + i] ^= block[i];
            }
            offset += chunk_len;
        }
    }

    /// Serializes current raw 16 words into a 64-byte little-endian block.
    pub fn to_bytes(&self) -> [u8; CHACHA20_BLOCK_SIZE] {
        let mut bytes = [0u8; CHACHA20_BLOCK_SIZE];
        for (i, &word) in self.state.iter().enumerate() {
            let le = word.to_le_bytes();
            let offset = i * 4;
            bytes[offset..offset + 4].copy_from_slice(&le);
        }
        bytes
    }
}

/// Self-test verifying ChaCha20 constants, state formatting, and RFC 7539 quarter-round vector.
pub fn verify_chacha20_structures() -> bool {
    // 1. Verify constant words
    if CONST_0 != 0x61707865 || CONST_1 != 0x3320646e || CONST_2 != 0x79622d32 || CONST_3 != 0x6b206574 {
        return false;
    }

    // 2. Verify RFC 7539 Section 2.1.1 Quarter Round Test Vector
    let mut a = 0x11111111u32;
    let mut b = 0x01020304u32;
    let mut c = 0x9b8d6f43u32;
    let mut d = 0x01234567u32;

    ChaCha20State::quarter_round(&mut a, &mut b, &mut c, &mut d);

    if a != 0xea2a92f4 || b != 0xcb1cf8ce || c != 0x4581472e || d != 0x5881c4bb {
        return false;
    }

    // 3. Verify state construction and counter manipulation
    let key = [0x42u8; CHACHA20_KEY_SIZE];
    let nonce = [0x24u8; CHACHA20_NONCE_SIZE];
    let mut chacha = ChaCha20State::new(&key, &nonce, 1);

    if chacha.state[0] != CONST_0 || chacha.counter() != 1 {
        return false;
    }

    chacha.set_counter(42);
    if chacha.counter() != 42 {
        return false;
    }

    let bytes = chacha.to_bytes();
    if bytes.len() != CHACHA20_BLOCK_SIZE {
        return false;
    }

    // 4. Verify RFC 7539 Section 2.3.2 ChaCha20 Block Function Test Vector
    let mut rfc_key = [0u8; CHACHA20_KEY_SIZE];
    for i in 0..32 {
        rfc_key[i] = i as u8;
    }
    let rfc_nonce = [0x00, 0x00, 0x00, 0x09, 0x00, 0x00, 0x00, 0x4a, 0x00, 0x00, 0x00, 0x00];
    let rfc_state = ChaCha20State::new(&rfc_key, &rfc_nonce, 1);
    let block = rfc_state.block_function();

    // First 4 bytes expected: 0x10, 0xf1, 0xe7, 0xe4
    if block[0] != 0x10 || block[1] != 0xf1 || block[2] != 0xe7 || block[3] != 0xe4 {
        return false;
    }
    // Last 4 bytes expected: 0xa2, 0x50, 0x3c, 0x4e
    if block[60] != 0xa2 || block[61] != 0x50 || block[62] != 0x3c || block[63] != 0x4e {
        return false;
    }

    true
}
