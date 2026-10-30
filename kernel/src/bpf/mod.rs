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

pub mod vm;

pub use vm::{
    test_bpf_vm_execution, BpfContext, BpfVirtualMachine, VmExitStatus, BPF_FUNC_GET_SMP_PROCESSOR_ID,
    BPF_FUNC_KTIME_GET_NS, BPF_FUNC_PACKET_LOAD_U16, BPF_FUNC_PACKET_LOAD_U32,
    BPF_FUNC_PACKET_LOAD_U8, BPF_FUNC_TRACE_PRINTK,
};


/// Comprehensive verification self-test suite for In-Kernel eBPF Packet & Event Filter Subsystem.
pub fn test_bpf_filter_subsystem() -> bool {
    crate::serial_write_str("[KNT-BPF-TEST] Running in-kernel eBPF bytecode packet & event filter verification...\r\n");

    // 1. Verify 64-bit eBPF instruction format and opcode decoding
    if !test_bpf_instruction_encoding() {
        crate::serial_write_str("[KNT-PANIC] eBPF instruction encoding self-test failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * eBPF 64-bit instruction format, ALU/jump opcodes, and decoding verified\r\n");

    // 2. Verify static bytecode verifier, DAG validation, and stack bounds
    if !test_bpf_verifier() {
        crate::serial_write_str("[KNT-PANIC] eBPF bytecode verifier self-test failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * Static bytecode verifier, CFG bounds, and DAG termination checker verified\r\n");

    // 3. Verify VM execution, ALU math, stack memory, and helper dispatch
    if !test_bpf_vm_execution() {
        crate::serial_write_str("[KNT-PANIC] eBPF VM execution self-test failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * High-performance register VM interpreter and helper calls verified\r\n");

    // 4. End-to-end integration: firewall packet filter drops non-IPv4 packets
    let mut vm = BpfVirtualMachine::new();

    // Bytecode program:
    // R1 = load_u16(offset = 0)  [EtherType]
    // if R1 != 0x0800 goto drop
    // R0 = 1 (ACCEPT)
    // exit
    // drop:
    // R0 = 0 (DROP)
    // exit
    let firewall_prog = [
        BpfInsn::mov64_imm(1, 0),                       // R1 = 0
        BpfInsn::call_helper(BPF_FUNC_PACKET_LOAD_U16), // R0 = load_u16(0)
        BpfInsn::jmp_eq_imm(0, 0x0800, 2),              // If EtherType == 0x0800 jump +2 (to accept)
        BpfInsn::mov64_imm(0, 0),                       // DROP: R0 = 0
        BpfInsn::jmp_exit(),
        BpfInsn::mov64_imm(0, 1),                       // ACCEPT: R0 = 1
        BpfInsn::jmp_exit(),
    ];

    // Packet A: IPv4 packet (EtherType 0x0800) -> should ACCEPT (1)
    static IPV4_PACKET: [u8; 6] = [0x08, 0x00, 0x45, 0x00, 0x00, 0x28];
    let ctx_ipv4 = BpfContext::new(IPV4_PACKET.as_ptr() as usize, IPV4_PACKET.len(), 0x0800);
    if vm.execute(&firewall_prog, &ctx_ipv4) != VmExitStatus::Success(1) {
        crate::serial_write_str("[KNT-PANIC] eBPF firewall failed to accept valid IPv4 packet!\r\n");
        return false;
    }

    // Packet B: ARP packet (EtherType 0x0806) -> should DROP (0)
    static ARP_PACKET: [u8; 6] = [0x08, 0x06, 0x00, 0x01, 0x08, 0x00];
    let ctx_arp = BpfContext::new(ARP_PACKET.as_ptr() as usize, ARP_PACKET.len(), 0x0806);
    if vm.execute(&firewall_prog, &ctx_arp) != VmExitStatus::Success(0) {
        crate::serial_write_str("[KNT-PANIC] eBPF firewall failed to drop non-IPv4 packet!\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-BPF-FILTER-PASS] In-kernel eBPF packet filter and bytecode engine operational\r\n");
    true
}
