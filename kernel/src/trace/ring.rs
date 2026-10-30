//! Lockless Circular Kernel Trace Ring Buffer.
//!
//! Provides per-core, bounded circular ring buffers storing 32-byte `TraceRecord` events.
//! Implements atomic write cursor advancement, overwrite-oldest semantics on wrap,
//! and batch reader draining without blocking kernel execution paths.

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use crate::trace::events::{TraceEventType, TraceRecord};

/// Default capacity: 1024 records per core ring buffer (32 KiB memory footprint).
pub const DEFAULT_RING_CAPACITY: usize = 1024;

/// Cacheline-isolated atomic cursor.
#[repr(C, align(64))]
pub struct CachelineCursor {
    pub value: AtomicUsize,
    _pad: [u8; 64 - core::mem::size_of::<AtomicUsize>()],
}

impl CachelineCursor {
    pub const fn new(val: usize) -> Self {
        Self {
            value: AtomicUsize::new(val),
            _pad: [0u8; 64 - core::mem::size_of::<AtomicUsize>()],
        }
    }
}

/// Control header for trace ring buffer inspection.
#[repr(C, align(64))]
pub struct TraceRingHeader {
    pub core_id: u16,
    pub capacity: usize,
    pub total_records_written: AtomicU64,
    pub total_records_dropped: AtomicU64,
    pub head: CachelineCursor, // Read cursor
    pub tail: CachelineCursor, // Write cursor
}

impl TraceRingHeader {
    pub const fn new(core_id: u16, capacity: usize) -> Self {
        Self {
            core_id,
            capacity,
            total_records_written: AtomicU64::new(0),
            total_records_dropped: AtomicU64::new(0),
            head: CachelineCursor::new(0),
            tail: CachelineCursor::new(0),
        }
    }
}

/// Lockless, single-producer multi-consumer circular trace buffer.
#[repr(C, align(64))]
pub struct TraceRingBuffer<const CAP: usize> {
    pub header: TraceRingHeader,
    pub records: [TraceRecord; CAP],
}

impl<const CAP: usize> TraceRingBuffer<CAP> {
    pub const fn new(core_id: u16) -> Self {
        Self {
            header: TraceRingHeader::new(core_id, CAP),
            records: [TraceRecord::empty(); CAP],
        }
    }

    #[inline(always)]
    pub const fn capacity(&self) -> usize {
        CAP
    }

    /// Emits a new trace record into the ring buffer with atomic write cursor advancement.
    /// Overwrites the oldest slot if full (lossless telemetry under normal load, circular wrap on surge).
    pub fn push(&mut self, record: TraceRecord) {
        let current_tail = self.header.tail.value.load(Ordering::Relaxed);
        let slot = current_tail % CAP;

        // Write record into current slot
        self.records[slot] = record;

        // Advance tail cursor
        self.header
            .tail
            .value
            .store(current_tail.wrapping_add(1), Ordering::Release);
        self.header
            .total_records_written
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Number of available records ready for reading.
    #[inline(always)]
    pub fn available_records(&self) -> usize {
        let tail = self.header.tail.value.load(Ordering::Acquire);
        let head = self.header.head.value.load(Ordering::Relaxed);
        let count = tail.wrapping_sub(head);
        if count > CAP {
            CAP
        } else {
            count
        }
    }

    /// Drains a batch of available trace records into `dest`.
    pub fn drain_batch(&mut self, dest: &mut [TraceRecord]) -> usize {
        let tail = self.header.tail.value.load(Ordering::Acquire);
        let head = self.header.head.value.load(Ordering::Relaxed);
        let available = tail.wrapping_sub(head);

        if available == 0 {
            return 0;
        }

        // Handle case where producer has wrapped past reader (head fallen behind capacity)
        let actual_head = if available > CAP {
            let dropped = available - CAP;
            self.header
                .total_records_dropped
                .fetch_add(dropped as u64, Ordering::Relaxed);
            tail.wrapping_sub(CAP)
        } else {
            head
        };

        let to_read = core::cmp::min(dest.len(), tail.wrapping_sub(actual_head));
        for i in 0..to_read {
            let slot = (actual_head.wrapping_add(i)) % CAP;
            dest[i] = self.records[slot];
        }

        self.header
            .head
            .value
            .store(actual_head.wrapping_add(to_read), Ordering::Release);
        to_read
    }

    /// Peeks at the latest emitted record without advancing read cursor.
    pub fn peek_latest(&self) -> Option<TraceRecord> {
        let tail = self.header.tail.value.load(Ordering::Acquire);
        if tail == 0 {
            return None;
        }
        let slot = (tail.wrapping_sub(1)) % CAP;
        let r = self.records[slot];
        if r.is_valid() {
            Some(r)
        } else {
            None
        }
    }
}

/// Unit test verifying trace ring buffer insertion, circular wrapping, and batch reading.
pub fn test_trace_ring_buffer() -> bool {
    static mut RING: TraceRingBuffer<16> = TraceRingBuffer::new(0);
    let ring = unsafe { &mut *core::ptr::addr_of_mut!(RING) };

    if ring.available_records() != 0 || ring.capacity() != 16 {
        return false;
    }

    // 1. Push 4 records
    for i in 0..4 {
        let rec = TraceRecord::new(
            TraceEventType::SyscallEntry,
            0,
            0,
            1_000 + i as u64,
            100 + i as u64,
            0,
        );
        ring.push(rec);
    }

    if ring.available_records() != 4 {
        return false;
    }

    // 2. Drain 2 records
    let mut drain_buf = [TraceRecord::empty(); 4];
    let drained = ring.drain_batch(&mut drain_buf[..2]);
    if drained != 2 || ring.available_records() != 2 {
        return false;
    }
    if drain_buf[0].timestamp_ns != 1_000 || drain_buf[1].timestamp_ns != 1_001 {
        return false;
    }

    // 3. Force circular wrap past capacity (push 20 records)
    for i in 0..20 {
        let rec = TraceRecord::new(
            TraceEventType::PageFault,
            0,
            0,
            2_000 + i as u64,
            0x0000_7FFF_0000_0000 + (i as u64) * 4096,
            2,
        );
        ring.push(rec);
    }

    // Available records should be clamped to CAP (16)
    if ring.available_records() != 16 {
        return false;
    }

    let latest = match ring.peek_latest() {
        Some(l) => l,
        None => return false,
    };
    if latest.timestamp_ns != 2_019 {
        return false;
    }

    // Drain all 16 records
    let mut full_drain = [TraceRecord::empty(); 16];
    let read_count = ring.drain_batch(&mut full_drain);
    if read_count != 16 || ring.available_records() != 0 {
        return false;
    }
    if full_drain[15].timestamp_ns != 2_019 {
        return false;
    }

    true
}
