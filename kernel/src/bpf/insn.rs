//! eBPF 64-bit Instruction Format, Opcode Encoding, & Argument Decoding.
//!
//! Implements standard 8-byte eBPF instruction representation conforming to Linux eBPF ISA:
//! - 8-bit opcode
//! - 4-bit destination register (dst_reg)
//! - 4-bit source register (src_reg)
//! - 16-bit signed offset
//! - 32-bit signed immediate (imm)

/// Maximum general-purpose registers (R0-R9 general, R10 read-only stack frame pointer).
pub const BPF_REG_COUNT: usize = 11;
pub const BPF_REG_FP: u8 = 10; // Frame pointer register

// Instruction Classes (bits 2:0 of opcode)
pub const BPF_CLASS_LD: u8 = 0x00;
pub const BPF_CLASS_LDX: u8 = 0x01;
pub const BPF_CLASS_ST: u8 = 0x02;
pub const BPF_CLASS_STX: u8 = 0x03;
pub const BPF_CLASS_ALU: u8 = 0x04;  // 32-bit ALU
pub const BPF_CLASS_JMP: u8 = 0x05;  // 64-bit Jumps
pub const BPF_CLASS_JMP32: u8 = 0x06;// 32-bit Jumps
pub const BPF_CLASS_ALU64: u8 = 0x07;// 64-bit ALU

// Source Modifiers (bit 3 of opcode)
pub const BPF_SRC_K: u8 = 0x00; // Immediate source
pub const BPF_SRC_X: u8 = 0x08; // Register source

// ALU Operation Codes (bits 7:4 of opcode)
pub const BPF_ALU_ADD: u8 = 0x00;
pub const BPF_ALU_SUB: u8 = 0x10;
pub const BPF_ALU_MUL: u8 = 0x20;
pub const BPF_ALU_DIV: u8 = 0x30;
pub const BPF_ALU_OR: u8 = 0x40;
pub const BPF_ALU_AND: u8 = 0x50;
pub const BPF_ALU_LSH: u8 = 0x60;
pub const BPF_ALU_RSH: u8 = 0x70;
pub const BPF_ALU_NEG: u8 = 0x80;
pub const BPF_ALU_MOD: u8 = 0x90;
pub const BPF_ALU_XOR: u8 = 0xA0;
pub const BPF_ALU_MOV: u8 = 0xB0;
pub const BPF_ALU_ARSH: u8 = 0xC0;
pub const BPF_ALU_END: u8 = 0xD0;

// Jump Operation Codes (bits 7:4 of opcode)
pub const BPF_JMP_JA: u8 = 0x00;   // Unconditional jump
pub const BPF_JMP_JEQ: u8 = 0x10;  // Jump if equal
pub const BPF_JMP_JGT: u8 = 0x20;  // Jump if greater than (unsigned)
pub const BPF_JMP_JGE: u8 = 0x30;  // Jump if greater than or equal (unsigned)
pub const BPF_JMP_JSET: u8 = 0x40; // Jump if bitwise AND non-zero
pub const BPF_JMP_JNE: u8 = 0x50;  // Jump if not equal
pub const BPF_JMP_JSGT: u8 = 0x60; // Jump if greater than (signed)
pub const BPF_JMP_JSGE: u8 = 0x70; // Jump if greater than or equal (signed)
pub const BPF_JMP_CALL: u8 = 0x80; // Call kernel helper function
pub const BPF_JMP_EXIT: u8 = 0x90; // Exit program returning R0
pub const BPF_JMP_JLT: u8 = 0xA0;  // Jump if less than (unsigned)
pub const BPF_JMP_JLE: u8 = 0xB0;  // Jump if less than or equal (unsigned)
pub const BPF_JMP_JSLT: u8 = 0xC0; // Jump if less than (signed)
pub const BPF_JMP_JSLE: u8 = 0xD0; // Jump if less than or equal (signed)

// Memory Size Modifiers (bits 4:3 of opcode in LD/ST/LDX/STX)
pub const BPF_SIZE_W: u8 = 0x00;  // 32-bit word
pub const BPF_SIZE_H: u8 = 0x08;  // 16-bit half-word
pub const BPF_SIZE_B: u8 = 0x10;  // 8-bit byte
pub const BPF_SIZE_DW: u8 = 0x18; // 64-bit double-word

// Memory Mode Modifiers (bits 7:5 of opcode in LD/ST/LDX/STX)
pub const BPF_MODE_MEM: u8 = 0x60;

/// Standard 8-byte eBPF instruction structure.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BpfInsn {
    pub opcode: u8,
    pub regs: u8, // High 4 bits: src_reg, Low 4 bits: dst_reg
    pub offset: i16,
    pub imm: i32,
}

impl BpfInsn {
    pub const SIZE: usize = 8;

    pub const fn new(opcode: u8, dst_reg: u8, src_reg: u8, offset: i16, imm: i32) -> Self {
        Self {
            opcode,
            regs: ((src_reg & 0x0F) << 4) | (dst_reg & 0x0F),
            offset,
            imm,
        }
    }

    #[inline(always)]
    pub fn dst_reg(&self) -> u8 {
        self.regs & 0x0F
    }

    #[inline(always)]
    pub fn src_reg(&self) -> u8 {
        (self.regs >> 4) & 0x0F
    }

    #[inline(always)]
    pub fn class(&self) -> u8 {
        self.opcode & 0x07
    }

    #[inline(always)]
    pub fn is_alu64(&self) -> bool {
        self.class() == BPF_CLASS_ALU64
    }

    #[inline(always)]
    pub fn is_alu32(&self) -> bool {
        self.class() == BPF_CLASS_ALU
    }

    #[inline(always)]
    pub fn is_jmp(&self) -> bool {
        self.class() == BPF_CLASS_JMP || self.class() == BPF_CLASS_JMP32
    }

    #[inline(always)]
    pub fn is_mem_load(&self) -> bool {
        self.class() == BPF_CLASS_LDX
    }

    #[inline(always)]
    pub fn is_mem_store(&self) -> bool {
        self.class() == BPF_CLASS_ST || self.class() == BPF_CLASS_STX
    }

    #[inline(always)]
    pub fn is_exit(&self) -> bool {
        self.opcode == (BPF_CLASS_JMP | BPF_JMP_EXIT)
    }

    #[inline(always)]
    pub fn is_call(&self) -> bool {
        self.opcode == (BPF_CLASS_JMP | BPF_JMP_CALL)
    }

    // Helper builders for common instructions
    pub const fn mov64_imm(dst: u8, imm: i32) -> Self {
        Self::new(BPF_CLASS_ALU64 | BPF_SRC_K | BPF_ALU_MOV, dst, 0, 0, imm)
    }

    pub const fn mov64_reg(dst: u8, src: u8) -> Self {
        Self::new(BPF_CLASS_ALU64 | BPF_SRC_X | BPF_ALU_MOV, dst, src, 0, 0)
    }

    pub const fn add64_imm(dst: u8, imm: i32) -> Self {
        Self::new(BPF_CLASS_ALU64 | BPF_SRC_K | BPF_ALU_ADD, dst, 0, 0, imm)
    }

    pub const fn add64_reg(dst: u8, src: u8) -> Self {
        Self::new(BPF_CLASS_ALU64 | BPF_SRC_X | BPF_ALU_ADD, dst, src, 0, 0)
    }

    pub const fn sub64_imm(dst: u8, imm: i32) -> Self {
        Self::new(BPF_CLASS_ALU64 | BPF_SRC_K | BPF_ALU_SUB, dst, 0, 0, imm)
    }

    pub const fn jmp_eq_imm(dst: u8, imm: i32, off: i16) -> Self {
        Self::new(BPF_CLASS_JMP | BPF_SRC_K | BPF_JMP_JEQ, dst, 0, off, imm)
    }

    pub const fn jmp_exit() -> Self {
        Self::new(BPF_CLASS_JMP | BPF_JMP_EXIT, 0, 0, 0, 0)
    }

    pub const fn call_helper(func_id: i32) -> Self {
        Self::new(BPF_CLASS_JMP | BPF_JMP_CALL, 0, 0, 0, func_id)
    }

    pub const fn ldx_mem(size: u8, dst: u8, src: u8, off: i16) -> Self {
        Self::new(BPF_CLASS_LDX | BPF_MODE_MEM | size, dst, src, off, 0)
    }

    pub const fn stx_mem(size: u8, dst: u8, src: u8, off: i16) -> Self {
        Self::new(BPF_CLASS_STX | BPF_MODE_MEM | size, dst, src, off, 0)
    }
}

/// Unit test verifying eBPF instruction encoding, decoding, and builder helpers.
pub fn test_bpf_instruction_encoding() -> bool {
    let insn1 = BpfInsn::mov64_imm(0, 42);
    if insn1.dst_reg() != 0 || insn1.src_reg() != 0 || insn1.imm != 42 {
        return false;
    }
    if !insn1.is_alu64() || insn1.class() != BPF_CLASS_ALU64 {
        return false;
    }

    let insn2 = BpfInsn::ldx_mem(BPF_SIZE_DW, 1, 10, -8);
    if insn2.dst_reg() != 1 || insn2.src_reg() != 10 || insn2.offset != -8 {
        return false;
    }
    if !insn2.is_mem_load() {
        return false;
    }

    let insn3 = BpfInsn::jmp_exit();
    if !insn3.is_exit() || !insn3.is_jmp() {
        return false;
    }

    let insn4 = BpfInsn::call_helper(1); // ktime_get_ns
    if !insn4.is_call() || insn4.imm != 1 {
        return false;
    }

    true
}
