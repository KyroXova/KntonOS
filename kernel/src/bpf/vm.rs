//! High-Performance Register-Based eBPF Virtual Machine Interpreter & Helper Dispatch.
//! Executes verified eBPF bytecode in the kernel with 64-bit virtual registers (R0-R10),

use crate::bpf::insn::*;
use crate::bpf::verifier::{BpfVerifier, VerifierError, BPF_STACK_SIZE};

/// Well-known eBPF in-kernel helper function IDs.
pub const BPF_FUNC_KTIME_GET_NS: i32 = 1;
pub const BPF_FUNC_TRACE_PRINTK: i32 = 2;
pub const BPF_FUNC_PACKET_LOAD_U8: i32 = 3;
pub const BPF_FUNC_PACKET_LOAD_U16: i32 = 4;
pub const BPF_FUNC_PACKET_LOAD_U32: i32 = 5;
pub const BPF_FUNC_GET_SMP_PROCESSOR_ID: i32 = 6;

/// Context passed to eBPF packet/event filter program.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BpfContext {
    pub data_ptr: usize,
    pub data_len: usize,
    pub protocol: u16,
    pub vlan_id: u16,
    pub ifindex: u32,
}

impl BpfContext {
    pub const fn new(data_ptr: usize, data_len: usize, protocol: u16) -> Self {
        Self {
            data_ptr,
            data_len,
            protocol,
            vlan_id: 0,
            ifindex: 0,
        }
    }
}

/// Execution outcome of an eBPF program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmExitStatus {
    Success(u64), // Program terminated normally returning R0 value
    DivisionByZero,
    UnknownHelper(i32),
    MemoryFault,
    InvalidOpcode(u8),
    VerificationFailed(VerifierError),
}

/// High-performance register-based virtual machine interpreter.
pub struct BpfVirtualMachine {
    regs: [u64; BPF_REG_COUNT],
    stack: [u8; BPF_STACK_SIZE as usize],
}

impl BpfVirtualMachine {
    pub const fn new() -> Self {
        Self {
            regs: [0; BPF_REG_COUNT],
            stack: [0; BPF_STACK_SIZE as usize],
        }
    }

    /// Dispatches a kernel helper function call.
    fn dispatch_helper(&mut self, func_id: i32, ctx: &BpfContext) -> Result<u64, VmExitStatus> {
        match func_id {
            BPF_FUNC_KTIME_GET_NS => {
                // Return monotonic time in nanoseconds
                Ok(crate::arch::x86_64::monotonic_time_ns())
            }
            BPF_FUNC_TRACE_PRINTK => {
                // R1 = message code, return 0
                Ok(0)
            }
            BPF_FUNC_PACKET_LOAD_U8 => {
                // R1 = offset
                let offset = self.regs[1] as usize;
                if offset < ctx.data_len {
                    let val = unsafe { *((ctx.data_ptr + offset) as *const u8) };
                    Ok(val as u64)
                } else {
                    Err(VmExitStatus::MemoryFault)
                }
            }
            BPF_FUNC_PACKET_LOAD_U16 => {
                // R1 = offset (load network byte order u16)
                let offset = self.regs[1] as usize;
                if offset + 2 <= ctx.data_len {
                    let b0 = unsafe { *((ctx.data_ptr + offset) as *const u8) } as u16;
                    let b1 = unsafe { *((ctx.data_ptr + offset + 1) as *const u8) } as u16;
                    Ok(((b0 << 8) | b1) as u64)
                } else {
                    Err(VmExitStatus::MemoryFault)
                }
            }
            BPF_FUNC_PACKET_LOAD_U32 => {
                // R1 = offset (load network byte order u32)
                let offset = self.regs[1] as usize;
                if offset + 4 <= ctx.data_len {
                    let b0 = unsafe { *((ctx.data_ptr + offset) as *const u8) } as u32;
                    let b1 = unsafe { *((ctx.data_ptr + offset + 1) as *const u8) } as u32;
                    let b2 = unsafe { *((ctx.data_ptr + offset + 2) as *const u8) } as u32;
                    let b3 = unsafe { *((ctx.data_ptr + offset + 3) as *const u8) } as u32;
                    Ok(((b0 << 24) | (b1 << 16) | (b2 << 8) | b3) as u64)
                } else {
                    Err(VmExitStatus::MemoryFault)
                }
            }
            BPF_FUNC_GET_SMP_PROCESSOR_ID => {
                // Return current core index (0)
                Ok(0)
            }
            _ => Err(VmExitStatus::UnknownHelper(func_id)),
        }
    }

    /// Executes an eBPF program against a context, returning `VmExitStatus`.
    pub fn execute(&mut self, insns: &[BpfInsn], ctx: &BpfContext) -> VmExitStatus {
        // First verify program safely
        if let Err(e) = BpfVerifier::verify(insns) {
            return VmExitStatus::VerificationFailed(e);
        }

        // Initialize registers
        self.regs = [0; BPF_REG_COUNT];
        self.stack = [0; BPF_STACK_SIZE as usize];

        // R1 = context pointer
        self.regs[1] = ctx as *const BpfContext as usize as u64;

        // R10 = frame pointer (points to the top of the 512-byte stack frame)
        let stack_top = self.stack.as_mut_ptr() as usize + (BPF_STACK_SIZE as usize);
        self.regs[BPF_REG_FP as usize] = stack_top as u64;

        let len = insns.len();
        let mut pc = 0;

        while pc < len {
            let insn = insns[pc];
            let opcode = insn.opcode;
            let dst = insn.dst_reg() as usize;
            let src = insn.src_reg() as usize;
            let imm = insn.imm as u64;

            let class = opcode & 0x07;
            let is_reg_src = (opcode & 0x08) == BPF_SRC_X;
            let src_val = if is_reg_src { self.regs[src] } else { imm };

            match class {
                BPF_CLASS_ALU64 => {
                    let op = opcode & 0xF0;
                    match op {
                        BPF_ALU_ADD => self.regs[dst] = self.regs[dst].wrapping_add(src_val),
                        BPF_ALU_SUB => self.regs[dst] = self.regs[dst].wrapping_sub(src_val),
                        BPF_ALU_MUL => self.regs[dst] = self.regs[dst].wrapping_mul(src_val),
                        BPF_ALU_DIV => {
                            if src_val == 0 {
                                return VmExitStatus::DivisionByZero;
                            }
                            self.regs[dst] = self.regs[dst] / src_val;
                        }
                        BPF_ALU_OR => self.regs[dst] = self.regs[dst] | src_val,
                        BPF_ALU_AND => self.regs[dst] = self.regs[dst] & src_val,
                        BPF_ALU_LSH => self.regs[dst] = self.regs[dst] << (src_val & 63),
                        BPF_ALU_RSH => self.regs[dst] = self.regs[dst] >> (src_val & 63),
                        BPF_ALU_NEG => self.regs[dst] = (!self.regs[dst]).wrapping_add(1),
                        BPF_ALU_MOD => {
                            if src_val == 0 {
                                return VmExitStatus::DivisionByZero;
                            }
                            self.regs[dst] = self.regs[dst] % src_val;
                        }
                        BPF_ALU_XOR => self.regs[dst] = self.regs[dst] ^ src_val,
                        BPF_ALU_MOV => self.regs[dst] = src_val,
                        BPF_ALU_ARSH => self.regs[dst] = ((self.regs[dst] as i64) >> (src_val & 63)) as u64,
                        _ => return VmExitStatus::InvalidOpcode(opcode),
                    }
                }

                BPF_CLASS_ALU => {
                    let op = opcode & 0xF0;
                    let dst32 = self.regs[dst] as u32;
                    let src32 = src_val as u32;
                    let res32 = match op {
                        BPF_ALU_ADD => dst32.wrapping_add(src32),
                        BPF_ALU_SUB => dst32.wrapping_sub(src32),
                        BPF_ALU_MUL => dst32.wrapping_mul(src32),
                        BPF_ALU_DIV => {
                            if src32 == 0 {
                                return VmExitStatus::DivisionByZero;
                            }
                            dst32 / src32
                        }
                        BPF_ALU_OR => dst32 | src32,
                        BPF_ALU_AND => dst32 & src32,
                        BPF_ALU_LSH => dst32 << (src32 & 31),
                        BPF_ALU_RSH => dst32 >> (src32 & 31),
                        BPF_ALU_NEG => (!dst32).wrapping_add(1),
                        BPF_ALU_MOD => {
                            if src32 == 0 {
                                return VmExitStatus::DivisionByZero;
                            }
                            dst32 % src32
                        }
                        BPF_ALU_XOR => dst32 ^ src32,
                        BPF_ALU_MOV => src32,
                        _ => return VmExitStatus::InvalidOpcode(opcode),
                    };
                    self.regs[dst] = res32 as u64; // Zero-extended
                }

                BPF_CLASS_JMP => {
                    let op = opcode & 0xF0;
                    match op {
                        BPF_JMP_JA => {
                            pc = (pc as isize + insn.offset as isize + 1) as usize;
                            continue;
                        }
                        BPF_JMP_CALL => {
                            match self.dispatch_helper(insn.imm, ctx) {
                                Ok(ret) => self.regs[0] = ret,
                                Err(err) => return err,
                            }
                        }
                        BPF_JMP_EXIT => {
                            return VmExitStatus::Success(self.regs[0]);
                        }
                        _ => {
                            let cond = match op {
                                BPF_JMP_JEQ => self.regs[dst] == src_val,
                                BPF_JMP_JGT => self.regs[dst] > src_val,
                                BPF_JMP_JGE => self.regs[dst] >= src_val,
                                BPF_JMP_JSET => (self.regs[dst] & src_val) != 0,
                                BPF_JMP_JNE => self.regs[dst] != src_val,
                                BPF_JMP_JSGT => (self.regs[dst] as i64) > (src_val as i64),
                                BPF_JMP_JSGE => (self.regs[dst] as i64) >= (src_val as i64),
                                BPF_JMP_JLT => self.regs[dst] < src_val,
                                BPF_JMP_JLE => self.regs[dst] <= src_val,
                                BPF_JMP_JSLT => (self.regs[dst] as i64) < (src_val as i64),
                                BPF_JMP_JSLE => (self.regs[dst] as i64) <= (src_val as i64),
                                _ => return VmExitStatus::InvalidOpcode(opcode),
                            };

                            if cond {
                                pc = (pc as isize + insn.offset as isize + 1) as usize;
                                continue;
                            }
                        }
                    }
                }

                BPF_CLASS_LDX => {
                    let addr = (self.regs[src] as i64 + insn.offset as i64) as usize;
                    let size = opcode & 0x18;
                    unsafe {
                        match size {
                            BPF_SIZE_B => self.regs[dst] = *(addr as *const u8) as u64,
                            BPF_SIZE_H => self.regs[dst] = *(addr as *const u16) as u64,
                            BPF_SIZE_W => self.regs[dst] = *(addr as *const u32) as u64,
                            BPF_SIZE_DW => self.regs[dst] = *(addr as *const u64),
                            _ => return VmExitStatus::InvalidOpcode(opcode),
                        }
                    }
                }

                BPF_CLASS_STX => {
                    let addr = (self.regs[dst] as i64 + insn.offset as i64) as usize;
                    let size = opcode & 0x18;
                    unsafe {
                        match size {
                            BPF_SIZE_B => *(addr as *mut u8) = self.regs[src] as u8,
                            BPF_SIZE_H => *(addr as *mut u16) = self.regs[src] as u16,
                            BPF_SIZE_W => *(addr as *mut u32) = self.regs[src] as u32,
                            BPF_SIZE_DW => *(addr as *mut u64) = self.regs[src],
                            _ => return VmExitStatus::InvalidOpcode(opcode),
                        }
                    }
                }

                _ => return VmExitStatus::InvalidOpcode(opcode),
            }

            pc += 1;
        }

        VmExitStatus::Success(self.regs[0])
    }
}

/// Unit test verifying VM execution, ALU calculations, stack reads/writes, and helper dispatches.
pub fn test_bpf_vm_execution() -> bool {
    let mut vm = BpfVirtualMachine::new();
    static PKT_DATA: [u8; 6] = [0x08, 0x00, 0x45, 0x00, 0x00, 0x3C]; // EtherType 0x0800 (IPv4)
    let ctx = BpfContext::new(PKT_DATA.as_ptr() as usize, PKT_DATA.len(), 0x0800);

    // 1. Simple ALU test: R0 = (10 + 5) * 2 = 30
    let prog_alu = [
        BpfInsn::mov64_imm(0, 10),
        BpfInsn::add64_imm(0, 5),
        BpfInsn::new(BPF_CLASS_ALU64 | BPF_SRC_K | BPF_ALU_MUL, 0, 0, 0, 2),
        BpfInsn::jmp_exit(),
    ];

    if vm.execute(&prog_alu, &ctx) != VmExitStatus::Success(30) {
        return false;
    }

    // 2. Stack write/read test: store 0xCAFE on stack and load it into R0
    let prog_stack = [
        BpfInsn::mov64_imm(1, 0xCAFE),
        BpfInsn::stx_mem(BPF_SIZE_DW, BPF_REG_FP, 1, -8),
        BpfInsn::ldx_mem(BPF_SIZE_DW, 0, BPF_REG_FP, -8),
        BpfInsn::jmp_exit(),
    ];

    if vm.execute(&prog_stack, &ctx) != VmExitStatus::Success(0xCAFE) {
        return false;
    }

    // 3. Packet inspection helper test: load EtherType (offset 0, size 2)
    // If EtherType == 0x0800 return 1 (PASS), else 0 (DROP)
    let prog_packet_filter = [
        BpfInsn::mov64_imm(1, 0),                       // R1 = offset 0
        BpfInsn::call_helper(BPF_FUNC_PACKET_LOAD_U16), // R0 = load_u16(offset)
        BpfInsn::jmp_eq_imm(0, 0x0800, 2),              // If R0 == 0x0800 jump +2
        BpfInsn::mov64_imm(0, 0),                       // DROP: R0 = 0
        BpfInsn::jmp_exit(),
        BpfInsn::mov64_imm(0, 1),                       // PASS: R0 = 1
        BpfInsn::jmp_exit(),
    ];

    if vm.execute(&prog_packet_filter, &ctx) != VmExitStatus::Success(1) {
        return false;
    }

    true
}
