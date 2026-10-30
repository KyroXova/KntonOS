//! Zero-Latency Kernel-Bypassed Input Polling Ring Buffer.
//!
//! Provides a lock-free Single-Producer Single-Consumer (SPSC) circular ring
//! designed for direct userspace memory mapping. High-performance gaming runtimes
//! and esports engines poll input state directly from userspace shared memory
//! with sub-microsecond latency and 0 system calls per sample.

extern crate alloc;

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Magic token identifying KntonOS Fast Input Shared Memory Ring: "KNTIFSTR"
pub const FAST_RING_MAGIC: u64 = 0x4B4E5449_46535452;

/// Current ABI version for the shared input ring buffer format.
pub const FAST_RING_VERSION: u32 = 1;

/// Default ring capacity (512 samples = ~512ms buffer at 1000Hz polling).
pub const DEFAULT_FAST_RING_CAPACITY: usize = 512;

/// Sample status flags.
pub const SAMPLE_FLAG_VALID: u32 = 1 << 0;
pub const SAMPLE_FLAG_FILTERED: u32 = 1 << 1;
pub const SAMPLE_FLAG_ANTI_REBOUND: u32 = 1 << 2;
pub const SAMPLE_FLAG_HALL_EFFECT: u32 = 1 << 3;

/// 64-byte aligned input sample frame compatible with direct userspace reads.
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FastInputSample {
    /// Monotonic timestamp in nanoseconds when this sample was captured.
    pub timestamp_ns: u64,
    /// Incrementing sequence number for torn-read and overrun detection.
    pub sequence: u64,
    /// Physical button bitmask (compatible with GamepadButtons).
    pub buttons: u32,
    /// Normalized & filtered Left Stick X in [-1.0, 1.0].
    pub left_stick_x: f32,
    /// Normalized & filtered Left Stick Y in [-1.0, 1.0].
    pub left_stick_y: f32,
    /// Normalized & filtered Right Stick X in [-1.0, 1.0].
    pub right_stick_x: f32,
    /// Normalized & filtered Right Stick Y in [-1.0, 1.0].
    pub right_stick_y: f32,
    /// Normalized Left Trigger in [0.0, 1.0].
    pub left_trigger: f32,
    /// Normalized Right Trigger in [0.0, 1.0].
    pub right_trigger: f32,
    /// Unfiltered 16-bit raw Left Stick X.
    pub raw_left_stick_x: i16,
    /// Unfiltered 16-bit raw Left Stick Y.
    pub raw_left_stick_y: i16,
    /// Unfiltered 16-bit raw Right Stick X.
    pub raw_right_stick_x: i16,
    /// Unfiltered 16-bit raw Right Stick Y.
    pub raw_right_stick_y: i16,
    /// Unfiltered 16-bit raw Left Trigger.
    pub raw_left_trigger: u16,
    /// Unfiltered 16-bit raw Right Trigger.
    pub raw_right_trigger: u16,
    /// Status flags (SAMPLE_FLAG_*).
    pub flags: u32,
    /// Reserved for 64-byte cache line alignment.
    pub _reserved: u32,
}

impl Default for FastInputSample {
    fn default() -> Self {
        Self {
            timestamp_ns: 0,
            sequence: 0,
            buttons: 0,
            left_stick_x: 0.0,
            left_stick_y: 0.0,
            right_stick_x: 0.0,
            right_stick_y: 0.0,
            left_trigger: 0.0,
            right_trigger: 0.0,
            raw_left_stick_x: 0,
            raw_left_stick_y: 0,
            raw_right_stick_x: 0,
            raw_right_stick_y: 0,
            raw_left_trigger: 0,
            raw_right_trigger: 0,
            flags: 0,
            _reserved: 0,
        }
    }
}

/// Cache-line aligned shared memory ring header mapped into userspace.
#[repr(C, align(64))]
pub struct FastRingHeader {
    /// Magic identifier verification token ("KNTIFSTR").
    pub magic: u64,
    /// ABI format version.
    pub version: u32,
    /// Total capacity of circular buffer in sample count (must be power of 2).
    pub capacity: u32,
    /// Producer head index (written by kernel polling engine, read by userspace).
    pub head: AtomicU32,
    /// Consumer tail index (read by kernel, written by userspace consumer).
    pub tail: AtomicU32,
    /// Total count of dropped samples due to ring buffer overflow.
    pub dropped_samples: AtomicU64,
    /// Monotonic timestamp of the most recent hardware poll in nanoseconds.
    pub last_poll_timestamp_ns: AtomicU64,
    /// Stride in bytes between consecutive samples (sizeof(FastInputSample)).
    pub sample_stride: u32,
    /// Header flags (e.g. 1 = ACTIVE, 2 = OVERRUN_OCCURRED).
    pub flags: AtomicU32,
    /// Reserved padding to fill 64-byte cache line.
    pub _pad: [u8; 8],
}

impl FastRingHeader {
    pub fn new(capacity: u32) -> Self {
        Self {
            magic: FAST_RING_MAGIC,
            version: FAST_RING_VERSION,
            capacity,
            head: AtomicU32::new(0),
            tail: AtomicU32::new(0),
            dropped_samples: AtomicU64::new(0),
            last_poll_timestamp_ns: AtomicU64::new(0),
            sample_stride: core::mem::size_of::<FastInputSample>() as u32,
            flags: AtomicU32::new(1),
            _pad: [0u8; 8],
        }
    }
}

/// Zero-Latency Kernel-Bypassed Input Ring Buffer.
///
/// Contains the shared header and ring slots. Can export memory descriptors
/// to be mapped directly into userspace process address space.
pub struct FastInputRingBuffer {
    pub header: FastRingHeader,
    pub buffer: Vec<FastInputSample>,
    capacity_mask: u32,
}

impl FastInputRingBuffer {
    /// Allocates a new fast input ring with power-of-two capacity.
    pub fn new(mut capacity: usize) -> Self {
        if !capacity.is_power_of_two() {
            capacity = capacity.next_power_of_two().max(16);
        }
        let cap_u32 = capacity as u32;
        let mut buffer = Vec::with_capacity(capacity);
        buffer.resize(capacity, FastInputSample::default());

        Self {
            header: FastRingHeader::new(cap_u32),
            buffer,
            capacity_mask: cap_u32 - 1,
        }
    }

    /// Pushes a new input sample into the ring buffer from the kernel producer.
    ///
    /// If the ring is full, drops the oldest unread sample by advancing the tail
    /// and incrementing the dropped sample counter (lossless for current state).
    pub fn push(&mut self, mut sample: FastInputSample) {
        let head = self.header.head.load(Ordering::Relaxed);
        let tail = self.header.tail.load(Ordering::Acquire);

        // Check if buffer is full
        if head.wrapping_sub(tail) >= self.header.capacity {
            // Drop oldest sample by incrementing tail and tracking dropped count
            self.header.tail.store(tail.wrapping_add(1), Ordering::Release);
            self.header.dropped_samples.fetch_add(1, Ordering::Relaxed);
        }

        let slot = (head & self.capacity_mask) as usize;
        sample.sequence = head as u64;
        self.buffer[slot] = sample;

        // Commit sample with release semantics so userspace consumer sees fully written sample
        self.header.head.store(head.wrapping_add(1), Ordering::Release);
        self.header.last_poll_timestamp_ns.store(sample.timestamp_ns, Ordering::Relaxed);
    }

    /// Pops the oldest available sample from the ring buffer.
    pub fn pop(&self) -> Option<FastInputSample> {
        let tail = self.header.tail.load(Ordering::Relaxed);
        let head = self.header.head.load(Ordering::Acquire);

        if tail == head {
            return None; // Buffer is empty
        }

        let slot = (tail & self.capacity_mask) as usize;
        let sample = self.buffer[slot];

        self.header.tail.store(tail.wrapping_add(1), Ordering::Release);
        Some(sample)
    }

    /// Peeks at the most recent sample without consuming it from the ring.
    pub fn peek_latest(&self) -> Option<FastInputSample> {
        let head = self.header.head.load(Ordering::Acquire);
        if head == 0 {
            return None;
        }

        let slot = ((head.wrapping_sub(1)) & self.capacity_mask) as usize;
        Some(self.buffer[slot])
    }

    /// Returns the number of unread samples currently available in the ring.
    pub fn len(&self) -> usize {
        let head = self.header.head.load(Ordering::Acquire);
        let tail = self.header.tail.load(Ordering::Relaxed);
        head.wrapping_sub(tail) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn capacity(&self) -> usize {
        self.header.capacity as usize
    }

    pub fn dropped_count(&self) -> u64 {
        self.header.dropped_samples.load(Ordering::Relaxed)
    }

    /// Creates a direct userspace mapping descriptor.
    pub fn export_userspace_descriptor(&self) -> UserspaceRingDescriptor {
        UserspaceRingDescriptor {
            header_addr: &self.header as *const FastRingHeader as usize as u64,
            buffer_addr: self.buffer.as_ptr() as usize as u64,
            capacity: self.header.capacity,
            sample_stride: self.header.sample_stride,
        }
    }
}

/// Userspace descriptor for mapping the fast input ring buffer into a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserspaceRingDescriptor {
    pub header_addr: u64,
    pub buffer_addr: u64,
    pub capacity: u32,
    pub sample_stride: u32,
}

/// Simulated Direct Userspace View of the Fast Input Ring.
///
/// Demonstrates how a game engine or high-performance userspace client consumes
/// samples directly from shared memory with 0 syscalls.
pub struct DirectUserspaceRingView {
    header_ptr: *const FastRingHeader,
    buffer_ptr: *const FastInputSample,
    capacity: u32,
    capacity_mask: u32,
}

impl DirectUserspaceRingView {
    /// Constructs a userspace view from exported memory addresses.
    pub unsafe fn from_descriptor(desc: &UserspaceRingDescriptor) -> Self {
        Self {
            header_ptr: desc.header_addr as *const FastRingHeader,
            buffer_ptr: desc.buffer_addr as *const FastInputSample,
            capacity: desc.capacity,
            capacity_mask: desc.capacity - 1,
        }
    }

    /// Reads the most recent sample in zero-copy mode with memory barrier synchronization.
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    pub fn read_latest(&self) -> Option<FastInputSample> {
        unsafe {
            let header = &*self.header_ptr;
            if header.magic != FAST_RING_MAGIC {
                return None;
            }

            let head = header.head.load(Ordering::Acquire);
            if head == 0 {
                return None;
            }

            let slot = ((head.wrapping_sub(1)) & self.capacity_mask) as usize;
            let sample = *self.buffer_ptr.add(slot);
            Some(sample)
        }
    }

    /// Consumes and drains up to `max_count` available samples without making any syscalls.
    pub fn drain_samples(&self, max_count: usize) -> Vec<FastInputSample> {
        let mut results = Vec::new();
        unsafe {
            let header = &*self.header_ptr;
            let mut tail = header.tail.load(Ordering::Relaxed);
            let head = header.head.load(Ordering::Acquire);

            let available = head.wrapping_sub(tail) as usize;
            let to_read = available.min(max_count);

            for _ in 0..to_read {
                let slot = (tail & self.capacity_mask) as usize;
                let sample = *self.buffer_ptr.add(slot);
                results.push(sample);
                tail = tail.wrapping_add(1);
            }

            header.tail.store(tail, Ordering::Release);
        }
        results
    }
}

/// Unit test verifying lock-free SPSC fast ring buffer and direct userspace view.
pub fn verify_fast_ring_buffer() -> bool {
    let mut ring = FastInputRingBuffer::new(16);
    if ring.capacity() != 16 || !ring.is_empty() {
        return false;
    }

    // 1. Push 5 samples into the ring
    for i in 0..5 {
        let sample = FastInputSample {
            timestamp_ns: 1_000_000 * (i as u64 + 1),
            sequence: 0,
            buttons: 1 << i,
            left_stick_x: (i as f32) * 0.2,
            left_stick_y: -(i as f32) * 0.2,
            right_stick_x: 0.0,
            right_stick_y: 0.0,
            left_trigger: (i as f32) * 0.1,
            right_trigger: 0.0,
            raw_left_stick_x: (i as i16) * 6000,
            raw_left_stick_y: -(i as i16) * 6000,
            raw_right_stick_x: 0,
            raw_right_stick_y: 0,
            raw_left_trigger: (i as u16) * 5000,
            raw_right_trigger: 0,
            flags: SAMPLE_FLAG_VALID,
            _reserved: 0,
        };
        ring.push(sample);
    }

    if ring.len() != 5 {
        return false;
    }

    // Verify peek_latest
    let latest = match ring.peek_latest() {
        Some(s) => s,
        None => return false,
    };
    if latest.buttons != (1 << 4) || (latest.left_stick_x - 0.8).abs() > 1e-4 {
        return false;
    }

    // 2. Test Direct Userspace View reading without syscall
    let desc = ring.export_userspace_descriptor();
    let userspace_view = unsafe { DirectUserspaceRingView::from_descriptor(&desc) };

    let user_latest = match userspace_view.read_latest() {
        Some(s) => s,
        None => return false,
    };
    if user_latest.buttons != (1 << 4) {
        return false;
    }

    // Drain 3 samples from userspace view
    let drained = userspace_view.drain_samples(3);
    if drained.len() != 3 || drained[0].buttons != (1 << 0) || drained[2].buttons != (1 << 2) {
        return false;
    }

    // 2 samples should remain in ring
    if ring.len() != 2 {
        return false;
    }

    // 3. Test ring overrun / overwrite behavior: push 20 samples to force wrap-around
    for i in 10..30 {
        let sample = FastInputSample {
            timestamp_ns: 1_000_000 * (i as u64),
            sequence: 0,
            buttons: i as u32,
            left_stick_x: 0.5,
            left_stick_y: 0.5,
            right_stick_x: 0.0,
            right_stick_y: 0.0,
            left_trigger: 0.5,
            right_trigger: 0.5,
            raw_left_stick_x: 16000,
            raw_left_stick_y: 16000,
            raw_right_stick_x: 0,
            raw_right_stick_y: 0,
            raw_left_trigger: 32000,
            raw_right_trigger: 32000,
            flags: SAMPLE_FLAG_VALID,
            _reserved: 0,
        };
        ring.push(sample);
    }

    // Buffer has capacity 16, so ring should now have 16 samples and dropped > 0
    if ring.len() != 16 || ring.dropped_count() == 0 {
        return false;
    }

    // Most recent sample should match sequence 30-1=29
    let latest_after_wrap = match ring.peek_latest() {
        Some(s) => s,
        None => return false,
    };
    if latest_after_wrap.buttons != 29 {
        return false;
    }

    true
}
