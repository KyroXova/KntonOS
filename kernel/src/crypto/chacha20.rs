//! ChaCha20 Stream Cipher State and Quarter-Round Permutations (RFC 7539).
//!
//! Provides the 512-bit (16 x 32-bit word) ChaCha20 state matrix, quarter-round
//! ARX (Add-Rotate-Xor) operations, and block formatting.

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

    /// Serializes 16 words into a 64-byte little-endian block.
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

    true
}
