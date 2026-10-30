//! Zero-latency kernel-bypassed input polling ring buffer.

extern crate alloc;

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use super::jitter::{AntiReboundSmoother, HallEffectCenterCalibrator, JitterDeadzoneConfig, OneEuroFilter2D};

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

/// Zero-latency kernel-bypassed input ring buffer supporting direct userspace mapping.
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

    /// Pushes a new input sample into the ring buffer, dropping oldest on overflow.
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

/// Direct userspace view of the fast input ring for zero-syscall sample consumption.
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


/// Raw hardware input sample packet polled from USB/Bluetooth/SPI at 1000Hz.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RawFastInputReport {
    /// Timestamp in nanoseconds when this raw sample was sampled.
    pub timestamp_ns: u64,
    /// Physical button bitmask.
    pub buttons: u32,
    /// Raw left stick X (-32768..32767).
    pub raw_left_stick_x: i16,
    /// Raw left stick Y (-32768..32767).
    pub raw_left_stick_y: i16,
    /// Raw right stick X (-32768..32767).
    pub raw_right_stick_x: i16,
    /// Raw right stick Y (-32768..32767).
    pub raw_right_stick_y: i16,
    /// Raw left trigger (0..65535).
    pub raw_left_trigger: u16,
    /// Raw right trigger (0..65535).
    pub raw_right_trigger: u16,
}

/// High-speed 1000Hz input polling dispatcher and pipeline orchestrator.
pub struct FastInputPollingDispatcher {
    pub ring: FastInputRingBuffer,
    pub left_stick_filter: OneEuroFilter2D,
    pub right_stick_filter: OneEuroFilter2D,
    pub left_stick_deadzone: JitterDeadzoneConfig,
    pub right_stick_deadzone: JitterDeadzoneConfig,
    pub left_stick_anti_rebound: AntiReboundSmoother,
    pub right_stick_anti_rebound: AntiReboundSmoother,
    pub left_hall_calibrator: HallEffectCenterCalibrator,
    pub right_hall_calibrator: HallEffectCenterCalibrator,
    pub left_trigger_inner: f32,
    pub left_trigger_outer: f32,
    pub right_trigger_inner: f32,
    pub right_trigger_outer: f32,

    // Diagnostics & performance metrics
    pub total_polls: u64,
    pub last_poll_timestamp_ns: u64,
    pub min_poll_interval_ns: u64,
    pub max_poll_interval_ns: u64,
    pub total_rebounds_suppressed: u64,
}

impl FastInputPollingDispatcher {
    pub fn new(ring_capacity: usize) -> Self {
        Self {
            ring: FastInputRingBuffer::new(ring_capacity),
            left_stick_filter: OneEuroFilter2D::default_stick(),
            right_stick_filter: OneEuroFilter2D::default_stick(),
            left_stick_deadzone: JitterDeadzoneConfig::default(),
            right_stick_deadzone: JitterDeadzoneConfig::default(),
            left_stick_anti_rebound: AntiReboundSmoother::default(),
            right_stick_anti_rebound: AntiReboundSmoother::default(),
            left_hall_calibrator: HallEffectCenterCalibrator::new(0.08),
            right_hall_calibrator: HallEffectCenterCalibrator::new(0.08),
            left_trigger_inner: 0.05,
            left_trigger_outer: 0.95,
            right_trigger_inner: 0.05,
            right_trigger_outer: 0.95,
            total_polls: 0,
            last_poll_timestamp_ns: 0,
            min_poll_interval_ns: u64::MAX,
            max_poll_interval_ns: 0,
            total_rebounds_suppressed: 0,
        }
    }

    /// Normalizes a 16-bit signed raw stick coordinate into [-1.0, 1.0].
    #[inline(always)]
    fn normalize_raw_axis(raw: i16) -> f32 {
        if raw == -32768 {
            -1.0
        } else {
            raw as f32 / 32767.0
        }
    }

    /// Normalizes a 16-bit unsigned raw trigger reading into [0.0, 1.0] with deadzone.
    #[inline(always)]
    fn normalize_raw_trigger(raw: u16, inner: f32, outer: f32) -> f32 {
        let norm = raw as f32 / 65535.0;
        if norm <= inner {
            0.0
        } else if norm >= outer {
            1.0
        } else {
            let denom = (outer - inner).max(1e-4);
            (norm - inner) / denom
        }
    }

    /// Processes a raw report at 1000Hz, executes all filter stages, and writes to shared ring buffer.
    pub fn dispatch_report(&mut self, report: RawFastInputReport) -> FastInputSample {
        // Track poll interval jitter
        if self.last_poll_timestamp_ns > 0 && report.timestamp_ns > self.last_poll_timestamp_ns {
            let interval = report.timestamp_ns - self.last_poll_timestamp_ns;
            if interval < self.min_poll_interval_ns {
                self.min_poll_interval_ns = interval;
            }
            if interval > self.max_poll_interval_ns {
                self.max_poll_interval_ns = interval;
            }
        }
        self.last_poll_timestamp_ns = report.timestamp_ns;
        self.total_polls += 1;

        // 1. Normalize raw axes to [-1.0, 1.0]
        let raw_lx = Self::normalize_raw_axis(report.raw_left_stick_x);
        let raw_ly = Self::normalize_raw_axis(report.raw_left_stick_y);
        let raw_rx = Self::normalize_raw_axis(report.raw_right_stick_x);
        let raw_ry = Self::normalize_raw_axis(report.raw_right_stick_y);

        // 2. Hall effect resting center drift calibration
        let left_resting = raw_lx.abs() < 0.05 && raw_ly.abs() < 0.05;
        let right_resting = raw_rx.abs() < 0.05 && raw_ry.abs() < 0.05;
        let (cal_lx, cal_ly) = self.left_hall_calibrator.calibrate(raw_lx, raw_ly, left_resting);
        let (cal_rx, cal_ry) = self.right_hall_calibrator.calibrate(raw_rx, raw_ry, right_resting);

        // 3. 1-Euro adaptive low-pass filter (micro-jitter suppression)
        let (filt_lx, filt_ly) = self.left_stick_filter.filter_stick(cal_lx, cal_ly, report.timestamp_ns);
        let (filt_rx, filt_ry) = self.right_stick_filter.filter_stick(cal_rx, cal_ry, report.timestamp_ns);

        // 4. Anti-rebound snapback smoothing
        let (smooth_lx, smooth_ly, l_suppressed) = self.left_stick_anti_rebound.process(filt_lx, filt_ly);
        let (smooth_rx, smooth_ry, r_suppressed) = self.right_stick_anti_rebound.process(filt_rx, filt_ry);
        if l_suppressed || r_suppressed {
            self.total_rebounds_suppressed += 1;
        }

        // 5. Radial deadzone shaping and response curve scaling
        let (final_lx, final_ly, _) = self.left_stick_deadzone.apply(smooth_lx, smooth_ly);
        let (final_rx, final_ry, _) = self.right_stick_deadzone.apply(smooth_rx, smooth_ry);

        // 6. Trigger normalization
        let norm_lt = Self::normalize_raw_trigger(report.raw_left_trigger, self.left_trigger_inner, self.left_trigger_outer);
        let norm_rt = Self::normalize_raw_trigger(report.raw_right_trigger, self.right_trigger_inner, self.right_trigger_outer);

        let mut flags = SAMPLE_FLAG_VALID | SAMPLE_FLAG_FILTERED;
        if l_suppressed || r_suppressed {
            flags |= SAMPLE_FLAG_ANTI_REBOUND;
        }
        if self.left_hall_calibrator.resting_sample_count() > 0 || self.right_hall_calibrator.resting_sample_count() > 0 {
            flags |= SAMPLE_FLAG_HALL_EFFECT;
        }

        let sample = FastInputSample {
            timestamp_ns: report.timestamp_ns,
            sequence: 0,
            buttons: report.buttons,
            left_stick_x: final_lx,
            left_stick_y: final_ly,
            right_stick_x: final_rx,
            right_stick_y: final_ry,
            left_trigger: norm_lt,
            right_trigger: norm_rt,
            raw_left_stick_x: report.raw_left_stick_x,
            raw_left_stick_y: report.raw_left_stick_y,
            raw_right_stick_x: report.raw_right_stick_x,
            raw_right_stick_y: report.raw_right_stick_y,
            raw_left_trigger: report.raw_left_trigger,
            raw_right_trigger: report.raw_right_trigger,
            flags,
            _reserved: 0,
        };

        // 7. Write to lock-free SPSC shared ring
        self.ring.push(sample);

        sample
    }

    pub fn poll_interval_jitter_ns(&self) -> (u64, u64) {
        (self.min_poll_interval_ns, self.max_poll_interval_ns)
    }

    pub fn export_userspace_descriptor(&self) -> UserspaceRingDescriptor {
        self.ring.export_userspace_descriptor()
    }
}


/// Unit test verifying anti-rebound smoothing filter and 1000Hz polling dispatcher.
pub fn verify_anti_rebound_and_polling_dispatcher() -> bool {
    // 1. Test AntiReboundSmoother snapback suppression
    let mut smoother = AntiReboundSmoother::new(0.50, 0.35, 20);

    // Deflect stick to +0.90 along X -> should arm smoother
    let (s1_x, s1_y, sup1) = smoother.process(0.90, 0.0);
    if (s1_x - 0.90).abs() > 1e-4 || s1_y != 0.0 || sup1 {
        return false;
    }

    // Stick released towards center (+0.08) -> activates 20-sample rebound window
    let (_s2_x, _s2_y, sup2) = smoother.process(0.08, 0.0);
    if sup2 {
        return false;
    }

    // Mechanical spring rebound into opposite direction: -0.22 along X
    let (s3_x, s3_y, sup3) = smoother.process(-0.22, 0.0);
    if !sup3 || s3_x != 0.0 || s3_y != 0.0 {
        return false; // Snapback was not suppressed!
    }
    if smoother.total_suppressed() != 1 {
        return false;
    }

    // 2. Test FastInputPollingDispatcher 1000Hz simulation
    let mut dispatcher = FastInputPollingDispatcher::new(32);

    let start_ts = 1_000_000_000u64; // 1 second
    for i in 0..100 {
        let ts = start_ts + (i as u64) * 1_000_000; // Exact 1ms ticks (1000Hz)
        let raw_report = RawFastInputReport {
            timestamp_ns: ts,
            buttons: if i >= 50 { 1 << 0 } else { 0 }, // Button A pressed in second half
            raw_left_stick_x: if i < 30 { 28000 } else if i < 40 { -6000 } else { 0 },
            raw_left_stick_y: 0,
            raw_right_stick_x: 0,
            raw_right_stick_y: 0,
            raw_left_trigger: if i >= 80 { 60000 } else { 0 },
            raw_right_trigger: 0,
        };
        let _ = dispatcher.dispatch_report(raw_report);
    }

    if dispatcher.total_polls != 100 {
        return false;
    }

    // Verify poll interval jitter tracking (all intervals were exactly 1,000,000 ns = 1ms)
    let (min_jitter, max_jitter) = dispatcher.poll_interval_jitter_ns();
    if min_jitter != 1_000_000 || max_jitter != 1_000_000 {
        return false;
    }

    // Verify ring buffer has 32 samples (capacity limit) and oldest were dropped
    if dispatcher.ring.len() != 32 || dispatcher.ring.dropped_count() != 100 - 32 {
        return false;
    }

    // Verify direct userspace zero-copy reading
    let desc = dispatcher.export_userspace_descriptor();
    let user_view = unsafe { DirectUserspaceRingView::from_descriptor(&desc) };

    let latest_sample = match user_view.read_latest() {
        Some(s) => s,
        None => return false,
    };

    // At sample 99, Button A was pressed (1 << 0) and left trigger was ~60000 / 65535
    if (latest_sample.buttons & (1 << 0)) == 0 || latest_sample.left_trigger < 0.80 {
        return false;
    }

    true
}
