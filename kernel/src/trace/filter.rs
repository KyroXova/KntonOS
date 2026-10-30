//! Dynamic Trace Event Filtering & Shared-Memory Userspace Reader Export Engine.
//!
//! Provides global and per-core event filtering masks, TSC nanosecond conversion,
//! and shared memory export framing for userspace profiling tools (`knton-trace-dump`).

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use crate::trace::events::{TraceEventMask, TraceEventType, TraceRecord};
use crate::trace::ring::TraceRingBuffer;

/// Maximum number of logical CPU cores supported for trace buffer indexing.
pub const MAX_TRACE_CORES: usize = 4;

/// Capacity of per-core trace buffers in the global profiling engine.
pub const ENGINE_RING_CAPACITY: usize = 256;

/// Shared-memory export magic identifier ("KNTNTRAC").
pub const TRACE_EXPORT_MAGIC: u64 = 0x4B4E_544E_5452_4143;
pub const TRACE_EXPORT_VERSION: u32 = 1;

/// Global dynamic event filtering and configuration engine.
pub struct TraceFilterEngine {
    enabled: AtomicBool,
    filter_mask: AtomicU32,
    tsc_khz: AtomicU32,
}

impl TraceFilterEngine {
    pub const fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            filter_mask: AtomicU32::new(0xFFFF_FFFF), // All events enabled by default
            tsc_khz: AtomicU32::new(2_000_000),      // 2 GHz default until calibrated
        }
    }

    #[inline(always)]
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    #[inline(always)]
    pub fn set_enabled(&self, enable: bool) {
        self.enabled.store(enable, Ordering::Release);
    }

    #[inline(always)]
    pub fn get_mask(&self) -> TraceEventMask {
        TraceEventMask(self.filter_mask.load(Ordering::Relaxed))
    }

    #[inline(always)]
    pub fn set_mask(&self, mask: TraceEventMask) {
        self.filter_mask.store(mask.0, Ordering::Release);
    }

    #[inline(always)]
    pub fn enable_event(&self, event: TraceEventType) {
        let bit = 1 << (event as u16);
        self.filter_mask.fetch_or(bit, Ordering::Release);
    }

    #[inline(always)]
    pub fn disable_event(&self, event: TraceEventType) {
        let bit = 1 << (event as u16);
        self.filter_mask.fetch_and(!bit, Ordering::Release);
    }

    #[inline(always)]
    pub fn is_event_allowed(&self, event: TraceEventType) -> bool {
        if !self.is_enabled() {
            return false;
        }
        let bit = 1 << (event as u16);
        (self.filter_mask.load(Ordering::Relaxed) & bit) != 0
    }

    #[inline(always)]
    pub fn set_tsc_khz(&self, khz: u32) {
        self.tsc_khz.store(khz, Ordering::Relaxed);
    }

    /// Converts raw TSC tick counter to nanoseconds using calibrated TSC frequency.
    #[inline(always)]
    pub fn tsc_to_nanos(&self, tsc: u64) -> u64 {
        let khz = self.tsc_khz.load(Ordering::Relaxed) as u64;
        if khz == 0 {
            return tsc;
        }
        // (tsc * 1_000_000) / khz
        (tsc.saturating_mul(1_000_000)) / khz
    }
}

/// Header for binary trace export buffer in shared memory.
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct TraceExportHeader {
    pub magic: u64,
    pub version: u32,
    pub core_count: u16,
    pub record_count: u32,
    pub dropped_count: u64,
    pub start_timestamp_ns: u64,
    pub end_timestamp_ns: u64,
}

impl TraceExportHeader {
    pub const fn empty() -> Self {
        Self {
            magic: TRACE_EXPORT_MAGIC,
            version: TRACE_EXPORT_VERSION,
            core_count: 0,
            record_count: 0,
            dropped_count: 0,
            start_timestamp_ns: 0,
            end_timestamp_ns: 0,
        }
    }
}

/// Global per-core trace manager and userspace export coordinator.
pub struct GlobalTraceCoordinator {
    pub filter: TraceFilterEngine,
    pub core_buffers: [TraceRingBuffer<ENGINE_RING_CAPACITY>; MAX_TRACE_CORES],
}

impl GlobalTraceCoordinator {
    pub const fn new() -> Self {
        Self {
            filter: TraceFilterEngine::new(),
            core_buffers: [
                TraceRingBuffer::new(0),
                TraceRingBuffer::new(1),
                TraceRingBuffer::new(2),
                TraceRingBuffer::new(3),
            ],
        }
    }

    /// Records a kernel event on `core_id` if tracing is enabled and event type passes filter mask.
    pub fn trace_event(
        &mut self,
        core_id: u16,
        event_type: TraceEventType,
        flags: u16,
        tsc: u64,
        payload_a: u64,
        payload_b: u64,
    ) -> bool {
        if !self.filter.is_event_allowed(event_type) {
            return false;
        }

        let cid = (core_id as usize) % MAX_TRACE_CORES;
        let nanos = self.filter.tsc_to_nanos(tsc);
        let record = TraceRecord::new(event_type, core_id, flags, nanos, payload_a, payload_b);

        self.core_buffers[cid].push(record);
        true
    }

    /// Exports merged records across all CPU core buffers into a shared-memory user destination slice.
    pub fn export_shared_memory(
        &mut self,
        header_out: &mut TraceExportHeader,
        dest_records: &mut [TraceRecord],
    ) -> usize {
        let mut total_exported = 0;
        let mut total_dropped = 0;
        let mut min_ts = u64::MAX;
        let mut max_ts = 0;

        for core_id in 0..MAX_TRACE_CORES {
            let space_left = dest_records.len().saturating_sub(total_exported);
            if space_left == 0 {
                break;
            }

            let drained = self.core_buffers[core_id].drain_batch(&mut dest_records[total_exported..]);
            for i in 0..drained {
                let ts = dest_records[total_exported + i].timestamp_ns;
                if ts < min_ts {
                    min_ts = ts;
                }
                if ts > max_ts {
                    max_ts = ts;
                }
            }
            total_exported += drained;
            total_dropped += self.core_buffers[core_id].header.total_records_dropped.load(Ordering::Relaxed);
        }

        header_out.magic = TRACE_EXPORT_MAGIC;
        header_out.version = TRACE_EXPORT_VERSION;
        header_out.core_count = MAX_TRACE_CORES as u16;
        header_out.record_count = total_exported as u32;
        header_out.dropped_count = total_dropped;
        header_out.start_timestamp_ns = if min_ts == u64::MAX { 0 } else { min_ts };
        header_out.end_timestamp_ns = max_ts;

        total_exported
    }
}

/// Unit test verifying dynamic event filtering, TSC nanosecond conversion, and shared memory export.
pub fn test_filter_and_export_engine() -> bool {
    let mut coordinator = GlobalTraceCoordinator::new();

    // 1. Initially disabled: events must be rejected
    if coordinator.trace_event(0, TraceEventType::SyscallEntry, 0, 1000, 1, 2) {
        return false;
    }

    // 2. Enable tracing and configure filter mask
    coordinator.filter.set_enabled(true);
    coordinator.filter.set_tsc_khz(2_500_000); // 2.5 GHz -> 1000 ticks = 400 ns

    let nanos = coordinator.filter.tsc_to_nanos(1000);
    if nanos != 400 {
        return false;
    }

    // Disable PageFault in mask
    coordinator.filter.disable_event(TraceEventType::PageFault);
    if coordinator.filter.is_event_allowed(TraceEventType::PageFault) {
        return false;
    }

    // Attempt PageFault event -> must be filtered
    if coordinator.trace_event(0, TraceEventType::PageFault, 0, 2000, 0x1000, 0) {
        return false;
    }

    // 3. Emit allowed events across multiple cores
    // Core 0: Context switch
    if !coordinator.trace_event(0, TraceEventType::SchedContextSwitch, 0, 5000, 10, 20) {
        return false;
    }
    // Core 1: IPC transfer
    if !coordinator.trace_event(1, TraceEventType::IpcTransfer, 0, 6000, 1, 128) {
        return false;
    }

    // 4. Export to shared memory buffer
    let mut export_hdr = TraceExportHeader::empty();
    let mut export_buf = [TraceRecord::empty(); 16];

    let count = coordinator.export_shared_memory(&mut export_hdr, &mut export_buf);
    if count != 2 {
        return false;
    }
    if export_hdr.magic != TRACE_EXPORT_MAGIC || export_hdr.record_count != 2 {
        return false;
    }
    if export_hdr.core_count != 4 {
        return false;
    }
    if export_hdr.start_timestamp_ns > export_hdr.end_timestamp_ns {
        return false;
    }

    true
}
