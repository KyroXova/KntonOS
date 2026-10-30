//! Zero-Overhead Kernel Trace Ring Buffer & Performance Profiling Event Engine.
//!
//! Provides lockless per-core circular event ring buffers, nanosecond timestamping,
//! dynamic event filtering, and shared-memory userspace exporter.

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
