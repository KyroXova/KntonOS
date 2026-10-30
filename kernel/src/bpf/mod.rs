//! In-Kernel eBPF-Style Bytecode Packet & Event Filter Subsystem.
//!
//! Provides a safe in-kernel virtual machine, static bytecode verifier, and helper execution
//! engine for dynamic packet filtering, tracepoint evaluation, and event instrumentation.

pub mod insn;

pub use insn::{
    test_bpf_instruction_encoding, BpfInsn, BPF_ALU_ADD, BPF_ALU_AND, BPF_ALU_ARSH, BPF_ALU_DIV,
    BPF_ALU_END, BPF_ALU_LSH, BPF_ALU_MOD, BPF_ALU_MOV, BPF_ALU_MUL, BPF_ALU_NEG, BPF_ALU_OR,
    BPF_ALU_RSH, BPF_ALU_SUB, BPF_ALU_XOR, BPF_CLASS_ALU, BPF_CLASS_ALU64, BPF_CLASS_JMP,
    BPF_CLASS_JMP32, BPF_CLASS_LD, BPF_CLASS_LDX, BPF_CLASS_ST, BPF_CLASS_STX, BPF_JMP_CALL,
    BPF_JMP_EXIT, BPF_JMP_JA, BPF_JMP_JEQ, BPF_JMP_JGE, BPF_JMP_JGT, BPF_JMP_JLE, BPF_JMP_JLT,
    BPF_JMP_JNE, BPF_JMP_JSET, BPF_JMP_JSGE, BPF_JMP_JSGT, BPF_JMP_JSLE, BPF_JMP_JSLT,
    BPF_MODE_MEM, BPF_REG_COUNT, BPF_REG_FP, BPF_SIZE_B, BPF_SIZE_DW, BPF_SIZE_H, BPF_SIZE_W,
    BPF_SRC_K, BPF_SRC_X,
};

pub mod verifier;

pub use verifier::{
    test_bpf_verifier, BpfVerifier, RegisterState, VerifierError, BPF_MAX_INSNS, BPF_STACK_SIZE,
};
