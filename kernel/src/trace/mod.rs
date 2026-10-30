//! Zero-Overhead Kernel Trace Ring Buffer & Performance Profiling Event Engine.
//! Provides lockless per-core circular event ring buffers, nanosecond timestamping,

pub mod events;

pub use events::{
    test_trace_record_serialization, TraceEventMask, TraceEventType, TraceRecord,
    TRACE_RECORD_MAGIC,
};

pub mod ring;

pub use ring::{
    test_trace_ring_buffer, CachelineCursor, TraceRingBuffer, TraceRingHeader,
    DEFAULT_RING_CAPACITY,
};

pub mod filter;
pub mod detour;
pub mod kprobes;

pub use detour::*;
pub use kprobes::*;
pub use filter::{
    test_filter_and_export_engine, GlobalTraceCoordinator, TraceExportHeader, TraceFilterEngine,
    ENGINE_RING_CAPACITY, MAX_TRACE_CORES, TRACE_EXPORT_MAGIC, TRACE_EXPORT_VERSION,
};

/// Comprehensive verification self-test suite for Zero-Overhead Kernel Trace Ring Buffer
/// & Performance Profiling Event Engine.
pub fn test_trace_subsystem() -> bool {
    crate::serial_write_str("[KNT-TRACE-TEST] Running Kernel Trace Ring Buffer & Performance Event verification suite...\r\n");

    // 1. Verify trace record binary serialization and size invariants
    if !test_trace_record_serialization() {
        crate::serial_write_str("[KNT-PANIC] Trace record serialization verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * Trace record 32-byte layout, event enumeration, and binary serialization verified\r\n");

    // 2. Verify lockless circular trace ring buffer and wrap advancement
    if !test_trace_ring_buffer() {
        crate::serial_write_str("[KNT-PANIC] Trace ring buffer verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * Lockless circular ring buffer, atomic cursor advancement, and wrap handling verified\r\n");

    // 3. Verify dynamic event filtering, TSC nanosecond conversion, and shared-memory export
    if !test_filter_and_export_engine() {
        crate::serial_write_str("[KNT-PANIC] Event filter and export engine verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * Dynamic event filtering mask, TSC calibration, and userspace export verified\r\n");

    // 4. End-to-end integration: profile synthetic system activity (syscalls, context switches, page faults)
    let mut coordinator = GlobalTraceCoordinator::new();
    coordinator.filter.set_enabled(true);
    coordinator.filter.set_tsc_khz(2_000_000); // 2.0 GHz

    // Core 0: Syscall entry & exit
    coordinator.trace_event(0, TraceEventType::SyscallEntry, 0, 10_000, 1, 0);
    coordinator.trace_event(0, TraceEventType::SyscallExit, 0, 12_000, 1, 1000);

    // Core 1: Context switch
    coordinator.trace_event(1, TraceEventType::SchedContextSwitch, 0, 15_000, 3, 5);

    // Core 2: Page fault
    coordinator.trace_event(2, TraceEventType::PageFault, 0, 20_000, 0x0000_7FFF_1234_0000, 4);

    let mut export_hdr = TraceExportHeader::empty();
    let mut dest = [TraceRecord::empty(); 32];
    let exported = coordinator.export_shared_memory(&mut export_hdr, &mut dest);
    if exported != 4 || export_hdr.record_count != 4 {
        crate::serial_write_str("[KNT-PANIC] End-to-end trace event export count mismatch!\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-TRACE-PERF-PASS] Zero-overhead kernel trace ring buffer and event engine operational\r\n");
    true
}

/// Comprehensive verification self-test suite for Kernel Dynamic Instrumentation Probes
/// (kprobes / ftrace Style) & Instruction Detour Engine.
pub fn test_kprobes_subsystem() -> bool {
    crate::serial_write_str("[KNT-TRACE-TEST] Running Kernel Dynamic Instrumentation Probes & Detour verification suite...\r\n");

    // 1. Verify x86-64 instruction detour splicing and out-of-line execute trampoline
    if !detour::test_instruction_detour_splicing() {
        crate::serial_write_str("[KNT-PANIC] Instruction detour splicing verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * x86-64 instruction detour splicing and out-of-line execute trampoline verified\r\n");

    // 2. Verify INT3 software breakpoint handler and dynamic kprobe registry
    if !kprobes::test_int3_and_kprobe_registry() {
        crate::serial_write_str("[KNT-PANIC] INT3 breakpoint handler and registry verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * INT3 breakpoint handler and dynamic kprobe registry verified\r\n");

    // 3. Verify pre/post probe callback invocation and register context capture
    if !kprobes::test_kprobe_callbacks_and_context() {
        crate::serial_write_str("[KNT-PANIC] Pre/post probe callback invocation verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * Pre/post probe callback invocation and register context capture verified\r\n");

    // 4. End-to-end integration: dynamic kprobe lifecycle with detour & context capture
    let mut registry = KprobeRegistry::new();
    let mut code_bytes = [0x55, 0x48, 0x89, 0xE5, 0x90]; // push rbp; mov rbp, rsp; nop
    let func_addr = 0x5000_1000usize;

    let probe_id = match registry.register_with_handlers(
        func_addr,
        "knt_sample_worker",
        Some(|_kp, regs| {
            regs.rsi = 0x5555_6666;
            true
        }),
        Some(|_kp, regs, _flags| {
            regs.rdi = regs.rsi + 1;
        }),
    ) {
        Ok(id) => id,
        Err(_) => {
            crate::serial_write_str("[KNT-PANIC] Failed to register sample kprobe!\r\n");
            return false;
        }
    };

    if registry.arm_probe(probe_id, &mut code_bytes).is_err() {
        crate::serial_write_str("[KNT-PANIC] Failed to arm sample kprobe!\r\n");
        return false;
    }

    if code_bytes[0] != INT3_OPCODE {
        crate::serial_write_str("[KNT-PANIC] INT3 opcode not present in armed code!\r\n");
        return false;
    }

    let mut regs = PtRegs::empty();
    regs.rip = (func_addr + 1) as u64;
    regs.rsp = 0x7FFF_FFFF_0000;

    // Handle INT3 trap
    if registry.handle_int3(&mut regs, 1) != KprobeBreakStatus::Handled {
        crate::serial_write_str("[KNT-PANIC] INT3 handler failed to process sample kprobe!\r\n");
        return false;
    }

    if regs.rsi != 0x5555_6666 || (regs.rflags & EFLAGS_TF) == 0 {
        crate::serial_write_str("[KNT-PANIC] Pre-handler context update or TF flag assertion failed!\r\n");
        return false;
    }

    // Handle single-step #DB trap
    if registry.handle_debug_trap(&mut regs) != KprobeBreakStatus::Handled {
        crate::serial_write_str("[KNT-PANIC] Debug trap handler failed to process single-step!\r\n");
        return false;
    }

    if regs.rdi != 0x5555_6667 || (regs.rflags & EFLAGS_TF) != 0 || regs.rip != (func_addr + 1) as u64 {
        crate::serial_write_str("[KNT-PANIC] Post-handler context update or RIP restoration failed!\r\n");
        return false;
    }

    if registry.disarm_probe(probe_id, &mut code_bytes).is_err() {
        crate::serial_write_str("[KNT-PANIC] Failed to disarm sample kprobe!\r\n");
        return false;
    }

    if code_bytes[0] != 0x55 {
        crate::serial_write_str("[KNT-PANIC] Original opcode not restored after disarm!\r\n");
        return false;
    }

    crate::serial_write_str("  * End-to-end dynamic kprobe lifecycle, INT3 splicing, and context mutation verified\r\n");
    crate::serial_write_str("[KNT-TRACE-KPROBES-PASS] Dynamic kprobes instrumentation and detour engine operational\r\n");
    true
}

