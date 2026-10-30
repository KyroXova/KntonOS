//! x86-64 Atomic Instruction Detour Splicing & Out-of-Line Execution Trampoline.
//!
//! Provides inline function detour hooking by splicing instructions with either a
//! 5-byte relative jump (`JMP rel32`) or a 14-byte 64-bit absolute indirect jump
//! (`FF 25 00 00 00 00 <64-bit addr>`). Stolen prologue instructions are preserved
//! in an out-of-line execute trampoline buffer to allow seamless continuation.

use core::sync::atomic::{fence, Ordering};

/// Maximum number of bytes stolen from function prologue.
pub const MAX_STOLEN_BYTES: usize = 32;

/// Maximum size of an out-of-line execution trampoline.
pub const TRAMPOLINE_SIZE: usize = 64;

/// Errors that may occur during detour creation or installation.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DetourError {
    BufferTooShort,
    InstructionDecodingFailed,
    DisplacementOutOfRange,
    AlreadyInstalled,
    NotInstalled,
    NullAddress,
}

/// Splicing mechanism type for detour jumps.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DetourType {
    /// 5-byte near relative jump: `0xE9 <disp32>`.
    /// Target must be within +/-2 GB of source.
    Rel32Jmp,
    /// 14-byte 64-bit absolute indirect jump: `FF 25 00 00 00 00 <u64 target>`.
    /// Can reach any address in 64-bit address space.
    Abs64Jmp,
}

impl DetourType {
    #[inline(always)]
    pub fn min_splice_bytes(&self) -> usize {
        match self {
            DetourType::Rel32Jmp => 5,
            DetourType::Abs64Jmp => 14,
        }
    }
}

/// Out-of-line execution trampoline holding stolen prologue instructions and a jump back.
#[derive(Copy, Clone, Debug)]
pub struct DetourTrampoline {
    pub buffer: [u8; TRAMPOLINE_SIZE],
    pub len: usize,
    pub return_addr: usize,
}

impl DetourTrampoline {
    pub const fn empty() -> Self {
        Self {
            buffer: [0x90; TRAMPOLINE_SIZE], // NOP filled
            len: 0,
            return_addr: 0,
        }
    }
}

/// Minimal instruction boundary decoder for x86-64 function prologues.
pub struct InstructionDecoder;

impl InstructionDecoder {
    /// Decodes the length in bytes of the single x86-64 instruction starting at `code[0]`.
    pub fn decode_insn_len(code: &[u8]) -> Result<usize, DetourError> {
        if code.is_empty() {
            return Err(DetourError::BufferTooShort);
        }

        let mut idx = 0;
        let len = code.len();

        // 1. Skip standard prefixes (operand size, address size, segment, REX, lock/rep)
        while idx < len {
            match code[idx] {
                // Segment overrides: CS, SS, DS, ES, FS, GS
                0x2E | 0x36 | 0x3E | 0x26 | 0x64 | 0x65 => idx += 1,
                // Operand size override (0x66), address size override (0x67)
                0x66 | 0x67 => idx += 1,
                // LOCK (0xF0), REPNE/REPNZ (0xF2), REP/REPE/REPZ (0xF3)
                0xF0 | 0xF2 | 0xF3 => idx += 1,
                // REX prefixes: 0x40 ..= 0x4F (x86-64)
                0x40..=0x4F => idx += 1,
                _ => break,
            }
        }

        if idx >= len {
            return Err(DetourError::BufferTooShort);
        }

        let op = code[idx];
        idx += 1;

        match op {
            // 1-byte opcodes without ModR/M
            0x90 // NOP
            | 0xCC // INT3
            | 0xC3 // RET
            | 0xCB // RET far
            | 0xF8 // CLC
            | 0xF9 // STC
            | 0xFA // CLI
            | 0xFB // STI
            | 0x50..=0x5F // PUSH/POP r64
            | 0x06 | 0x07 | 0x0E | 0x16 | 0x17 | 0x1E | 0x1F => Ok(idx),

            // 2-byte opcodes without ModR/M (opcode + 1-byte imm)
            0xEB // JMP rel8
            | 0x70..=0x7F // Jcc rel8
            | 0xB0..=0xB7 // MOV r8, imm8
            | 0xE4 | 0xE5 | 0xE6 | 0xE7 // IN/OUT imm8
            | 0xCD // INT imm8
            | 0xC2 // RET imm16 (needs 2 bytes imm)
            => {
                let imm_len = if op == 0xC2 { 2 } else { 1 };
                if idx + imm_len <= len {
                    Ok(idx + imm_len)
                } else {
                    Err(DetourError::BufferTooShort)
                }
            }

            // 5-byte opcodes without ModR/M (opcode + 4-byte imm/disp)
            0xE8 // CALL rel32
            | 0xE9 // JMP rel32
            | 0xB8..=0xBF // MOV r32, imm32 (or r64, imm32 without REX.W)
            => {
                // If preceded by REX.W (0x48..0x4F) and opcode is B8..BF, it takes an 8-byte imm64
                let is_rex_w = idx >= 2 && (code[idx - 2] & 0xF8) == 0x48;
                let imm_len = if is_rex_w && (0xB8..=0xBF).contains(&op) { 8 } else { 4 };
                if idx + imm_len <= len {
                    Ok(idx + imm_len)
                } else {
                    Err(DetourError::BufferTooShort)
                }
            }

            // Opcodes with ModR/M byte
            0x88 | 0x89 | 0x8A | 0x8B // MOV reg/mem
            | 0x8C | 0x8E // MOV seg
            | 0x8D // LEA
            | 0x84 | 0x85 // TEST
            | 0x00..=0x03 // ADD
            | 0x08..=0x0B // OR
            | 0x10..=0x13 // ADC
            | 0x18..=0x1B // SBB
            | 0x20..=0x23 // AND
            | 0x28..=0x2B // SUB
            | 0x30..=0x33 // XOR
            | 0x38..=0x3B // CMP
            | 0x80 | 0x81 | 0x82 | 0x83 // Group 1 ALU imm
            | 0xC0 | 0xC1 | 0xD0 | 0xD1 | 0xD2 | 0xD3 // Shift/Rotate
            | 0xC6 | 0xC7 // MOV reg/mem, imm
            | 0xF6 | 0xF7 // TEST/NOT/NEG/MUL/DIV
            | 0xFE | 0xFF // INC/DEC/CALL/JMP indirect/PUSH
            => {
                if idx >= len {
                    return Err(DetourError::BufferTooShort);
                }
                let modrm = code[idx];
                idx += 1;
                let mod_bits = (modrm >> 6) & 0x03;
                let rm_bits = modrm & 0x07;

                // SIB byte check in 64-bit mode (rm == 4 and mod != 3)
                if mod_bits != 3 && rm_bits == 4 {
                    if idx >= len {
                        return Err(DetourError::BufferTooShort);
                    }
                    idx += 1; // skip SIB
                }

                // Displacement
                match mod_bits {
                    0 => {
                        // RIP-relative addressing in x86-64 (rm == 5)
                        if rm_bits == 5 {
                            idx += 4; // disp32
                        }
                    }
                    1 => idx += 1, // disp8
                    2 => idx += 4, // disp32
                    _ => {}        // register direct (mod == 3): no disp
                }

                // Immediate byte for certain opcodes
                match op {
                    0x80 | 0x82 | 0xC6 => idx += 1, // imm8
                    0x83 => idx += 1,               // imm8 sign-extended
                    0x81 => idx += 4,               // imm32
                    0xC7 => idx += 4,               // imm32
                    0xF6 => {
                        // Group 3 TEST imm8
                        let reg_bits = (modrm >> 3) & 0x07;
                        if reg_bits == 0 || reg_bits == 1 {
                            idx += 1;
                        }
                    }
                    0xF7 => {
                        // Group 3 TEST imm32
                        let reg_bits = (modrm >> 3) & 0x07;
                        if reg_bits == 0 || reg_bits == 1 {
                            idx += 4;
                        }
                    }
                    _ => {}
                }

                if idx <= len {
                    Ok(idx)
                } else {
                    Err(DetourError::BufferTooShort)
                }
            }

            // 2-byte escape opcodes (0x0F ...)
            0x0F => {
                if idx >= len {
                    return Err(DetourError::BufferTooShort);
                }
                let op2 = code[idx];
                idx += 1;
                match op2 {
                    // Jcc rel32: 0x0F 0x80..=0x8F
                    0x80..=0x8F => {
                        if idx + 4 <= len {
                            Ok(idx + 4)
                        } else {
                            Err(DetourError::BufferTooShort)
                        }
                    }
                    // MOVZX / MOVSX / IMUL / NOP (ModR/M)
                    0xB6 | 0xB7 | 0xBE | 0xBF | 0xAF | 0x1F => {
                        if idx >= len {
                            return Err(DetourError::BufferTooShort);
                        }
                        let modrm = code[idx];
                        idx += 1;
                        let mod_bits = (modrm >> 6) & 0x03;
                        let rm_bits = modrm & 0x07;
                        if mod_bits != 3 && rm_bits == 4 {
                            idx += 1; // SIB
                        }
                        match mod_bits {
                            0 if rm_bits == 5 => idx += 4,
                            1 => idx += 1,
                            2 => idx += 4,
                            _ => {}
                        }
                        if idx <= len {
                            Ok(idx)
                        } else {
                            Err(DetourError::BufferTooShort)
                        }
                    }
                    _ => Ok(idx),
                }
            }

            _ => Ok(idx),
        }
    }

    /// Computes the exact number of whole instruction bytes required to cover at least `min_bytes`.
    pub fn calculate_splice_len(code: &[u8], min_bytes: usize) -> Result<usize, DetourError> {
        let mut total = 0;
        let mut cursor = 0;
        while total < min_bytes && cursor < code.len() {
            let insn_len = Self::decode_insn_len(&code[cursor..])?;
            if insn_len == 0 {
                return Err(DetourError::InstructionDecodingFailed);
            }
            total += insn_len;
            cursor += insn_len;
            if total >= MAX_STOLEN_BYTES {
                break;
            }
        }

        if total < min_bytes {
            Err(DetourError::BufferTooShort)
        } else {
            Ok(total)
        }
    }
}

/// High-level x86-64 instruction detour hook controller.
#[derive(Clone, Debug)]
pub struct DetourHook {
    pub target_addr: usize,
    pub detour_addr: usize,
    pub stolen_len: usize,
    pub original_bytes: [u8; MAX_STOLEN_BYTES],
    pub trampoline: DetourTrampoline,
    pub is_active: bool,
}

impl DetourHook {
    pub const fn new(target_addr: usize, detour_addr: usize) -> Self {
        Self {
            target_addr,
            detour_addr,
            stolen_len: 0,
            original_bytes: [0; MAX_STOLEN_BYTES],
            trampoline: DetourTrampoline::empty(),
            is_active: false,
        }
    }

    /// Prepares trampoline and installs the detour splice onto `target_mem`.
    pub fn install(
        &mut self,
        target_mem: &mut [u8],
        detour_type: DetourType,
    ) -> Result<(), DetourError> {
        if self.is_active {
            return Err(DetourError::AlreadyInstalled);
        }
        if self.target_addr == 0 || self.detour_addr == 0 {
            return Err(DetourError::NullAddress);
        }

        let min_bytes = detour_type.min_splice_bytes();
        let stolen_len = InstructionDecoder::calculate_splice_len(target_mem, min_bytes)?;
        if stolen_len > MAX_STOLEN_BYTES || target_mem.len() < stolen_len {
            return Err(DetourError::BufferTooShort);
        }

        self.stolen_len = stolen_len;
        self.original_bytes[..stolen_len].copy_from_slice(&target_mem[..stolen_len]);

        // Construct out-of-line execute trampoline:
        // [stolen instructions] + [JMP back to target_addr + stolen_len]
        let mut tramp = DetourTrampoline::empty();
        tramp.buffer[..stolen_len].copy_from_slice(&target_mem[..stolen_len]);

        // Append 14-byte 64-bit absolute indirect jump back to original code
        let ret_addr = self.target_addr + stolen_len;
        tramp.return_addr = ret_addr;
        let tramp_jmp_offset = stolen_len;
        tramp.buffer[tramp_jmp_offset..tramp_jmp_offset + 6]
            .copy_from_slice(&[0xFF, 0x25, 0x00, 0x00, 0x00, 0x00]);
        tramp.buffer[tramp_jmp_offset + 6..tramp_jmp_offset + 14]
            .copy_from_slice(&(ret_addr as u64).to_le_bytes());
        tramp.len = stolen_len + 14;
        self.trampoline = tramp;

        // Construct splice patch to detour
        match detour_type {
            DetourType::Rel32Jmp => {
                let disp = (self.detour_addr as i64) - ((self.target_addr + 5) as i64);
                if disp < (i32::MIN as i64) || disp > (i32::MAX as i64) {
                    return Err(DetourError::DisplacementOutOfRange);
                }

                // 1. Write opcode 0xE9 + disp32
                target_mem[0] = 0xE9;
                target_mem[1..5].copy_from_slice(&(disp as i32).to_le_bytes());

                // 2. Pad remaining stolen bytes with NOPs
                for b in &mut target_mem[5..stolen_len] {
                    *b = 0x90;
                }
            }
            DetourType::Abs64Jmp => {
                // 14-byte jump: FF 25 00 00 00 00 <u64 detour>
                target_mem[0..6].copy_from_slice(&[0xFF, 0x25, 0x00, 0x00, 0x00, 0x00]);
                target_mem[6..14].copy_from_slice(&(self.detour_addr as u64).to_le_bytes());

                // Pad remaining stolen bytes with NOPs
                for b in &mut target_mem[14..stolen_len] {
                    *b = 0x90;
                }
            }
        }

        fence(Ordering::SeqCst);
        self.is_active = true;
        Ok(())
    }

    /// Restores the original function prologue bytes, removing the detour splice.
    pub fn uninstall(&mut self, target_mem: &mut [u8]) -> Result<(), DetourError> {
        if !self.is_active {
            return Err(DetourError::NotInstalled);
        }
        if target_mem.len() < self.stolen_len {
            return Err(DetourError::BufferTooShort);
        }

        target_mem[..self.stolen_len].copy_from_slice(&self.original_bytes[..self.stolen_len]);
        fence(Ordering::SeqCst);
        self.is_active = false;
        Ok(())
    }
}

/// Unit test verifying x86-64 instruction detour splicing and out-of-line execute trampoline.
pub fn test_instruction_detour_splicing() -> bool {
    // Simulated function in memory:
    // 0x00: 55                      push rbp
    // 0x01: 48 89 E5                mov rbp, rsp
    // 0x04: 48 83 EC 20             sub rsp, 0x20
    // 0x08: B8 2A 00 00 00          mov eax, 42
    // 0x0D: C3                      ret
    let mut func_mem = [
        0x55, 0x48, 0x89, 0xE5, 0x48, 0x83, 0xEC, 0x20, 0xB8, 0x2A, 0x00, 0x00, 0x00, 0xC3,
    ];
    let original_copy = func_mem;

    let target_addr = 0x1000_0000;
    let detour_addr = 0x1000_0500;

    let mut hook = DetourHook::new(target_addr, detour_addr);
    if hook.install(&mut func_mem, DetourType::Rel32Jmp).is_err() {
        return false;
    }

    // Verify hook is active
    if !hook.is_active {
        return false;
    }
    // Verify stolen length is 8 (1 + 3 + 4)
    if hook.stolen_len != 8 {
        return false;
    }
    // Verify first byte of func_mem is 0xE9
    if func_mem[0] != 0xE9 {
        return false;
    }
    // Verify bytes 5..8 are NOPs (0x90)
    if func_mem[5] != 0x90 || func_mem[6] != 0x90 || func_mem[7] != 0x90 {
        return false;
    }
    // Verify relative displacement: detour_addr - (target_addr + 5) = 0x500 - 5 = 0x4FB
    let disp = i32::from_le_bytes([func_mem[1], func_mem[2], func_mem[3], func_mem[4]]);
    if disp != 0x4FB {
        return false;
    }

    // Verify trampoline starts with the 8 original bytes
    if &hook.trampoline.buffer[0..8] != &original_copy[0..8] {
        return false;
    }
    // Verify trampoline ends with jump back to target_addr + 8
    let tramp_jmp = &hook.trampoline.buffer[8..14];
    if tramp_jmp != [0xFF, 0x25, 0x00, 0x00, 0x00, 0x00] {
        return false;
    }
    let ret_target = u64::from_le_bytes([
        hook.trampoline.buffer[14],
        hook.trampoline.buffer[15],
        hook.trampoline.buffer[16],
        hook.trampoline.buffer[17],
        hook.trampoline.buffer[18],
        hook.trampoline.buffer[19],
        hook.trampoline.buffer[20],
        hook.trampoline.buffer[21],
    ]);
    if ret_target != (target_addr + 8) as u64 {
        return false;
    }

    // Test uninstall: restores original function bytes completely
    if hook.uninstall(&mut func_mem).is_err() {
        return false;
    }
    if func_mem != original_copy {
        return false;
    }
    if hook.is_active {
        return false;
    }

    // Test 14-byte 64-bit absolute jump detour
    let abs_detour = 0xFFFF_8000_1234_5678usize;
    let mut abs_hook = DetourHook::new(target_addr, abs_detour);
    // func_mem is 14 bytes: 1 + 3 + 4 + 5 + 1 = 14 bytes
    if abs_hook.install(&mut func_mem, DetourType::Abs64Jmp).is_err() {
        return false;
    }
    if abs_hook.stolen_len != 14 {
        return false;
    }
    if &func_mem[0..6] != &[0xFF, 0x25, 0x00, 0x00, 0x00, 0x00] {
        return false;
    }
    let written_addr = u64::from_le_bytes([
        func_mem[6],
        func_mem[7],
        func_mem[8],
        func_mem[9],
        func_mem[10],
        func_mem[11],
        func_mem[12],
        func_mem[13],
    ]);
    if written_addr != abs_detour as u64 {
        return false;
    }
    if abs_hook.uninstall(&mut func_mem).is_err() {
        return false;
    }
    if func_mem != original_copy {
        return false;
    }

    true
}
