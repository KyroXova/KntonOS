//! Hardware-Synchronized Cyclic DMA Audio Ring Buffer and Streamer Subsystem.
//!
//! Provides zero-latency cyclic DMA audio ring buffer descriptors matching
//! Intel High Definition Audio (HDA) / AC97 Buffer Descriptor Lists (BDL),
//! cyclic hardware position reporting, period interrupt progression tracking,
//! and xrun (overrun/underrun) detection.

pub const MAX_RING_PERIODS: usize = 32;

/// Error conditions for DMA audio streaming operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioStreamerError {
    InvalidConfiguration,
    InvalidPeriodSize,
    BufferOverrun,
    BufferUnderrun,
    StreamNotRunning,
    BufferTooSmall,
}

/// Hardware DMA Buffer Descriptor matching Intel HDA BDL entries.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioDmaDescriptor {
    pub phys_addr_low: u32,
    pub phys_addr_high: u32,
    pub length: u32,
    pub flags: u32, // Bit 0 = Interrupt on Completion (IOC)
}

impl AudioDmaDescriptor {
    pub const IOC_FLAG: u32 = 1 << 0;

    pub const fn new(phys_addr: u64, length: u32, ioc: bool) -> Self {
        Self {
            phys_addr_low: (phys_addr & 0xFFFF_FFFF) as u32,
            phys_addr_high: ((phys_addr >> 32) & 0xFFFF_FFFF) as u32,
            length,
            flags: if ioc { Self::IOC_FLAG } else { 0 },
        }
    }

    #[inline(always)]
    pub fn physical_address(&self) -> u64 {
        (self.phys_addr_low as u64) | ((self.phys_addr_high as u64) << 32)
    }

    #[inline(always)]
    pub fn has_ioc(&self) -> bool {
        (self.flags & Self::IOC_FLAG) != 0
    }
}

/// Zero-latency cyclic hardware-synchronized DMA audio ring buffer.
#[derive(Debug, Clone)]
pub struct AudioRingBuffer {
    pub phys_base: u64,
    pub total_size: usize,
    pub period_size: usize,
    pub num_periods: usize,
    pub sample_rate: u32,
    pub channels: u8,
    pub bit_depth: u8,
    pub frame_size: usize,

    // Hardware descriptor list
    pub descriptors: [AudioDmaDescriptor; MAX_RING_PERIODS],
    pub descriptor_count: usize,

    // Cursors and cyclic state
    pub write_cursor: usize,
    pub hw_read_cursor: usize,
    pub last_period_index: usize,
    pub period_interrupt_count: u64,
    pub xrun_underruns: u32,
    pub xrun_overruns: u32,
    pub running: bool,
}

impl AudioRingBuffer {
    /// Creates a cyclic DMA audio ring buffer.
    pub fn new(
        phys_base: u64,
        total_size: usize,
        period_size: usize,
        sample_rate: u32,
        channels: u8,
        bit_depth: u8,
    ) -> Result<Self, AudioStreamerError> {
        if period_size == 0 || total_size < period_size || total_size % period_size != 0 {
            return Err(AudioStreamerError::InvalidPeriodSize);
        }
        let num_periods = total_size / period_size;
        if num_periods > MAX_RING_PERIODS {
            return Err(AudioStreamerError::InvalidConfiguration);
        }

        let bytes_per_sample = (bit_depth as usize + 7) / 8;
        let frame_size = channels as usize * bytes_per_sample;
        if frame_size == 0 || period_size % frame_size != 0 {
            return Err(AudioStreamerError::InvalidConfiguration);
        }

        let mut descriptors = [AudioDmaDescriptor::new(0, 0, false); MAX_RING_PERIODS];
        for i in 0..num_periods {
            let period_addr = phys_base + (i * period_size) as u64;
            descriptors[i] = AudioDmaDescriptor::new(period_addr, period_size as u32, true);
        }

        Ok(Self {
            phys_base,
            total_size,
            period_size,
            num_periods,
            sample_rate,
            channels,
            bit_depth,
            frame_size,
            descriptors,
            descriptor_count: num_periods,
            write_cursor: 0,
            hw_read_cursor: 0,
            last_period_index: 0,
            period_interrupt_count: 0,
            xrun_underruns: 0,
            xrun_overruns: 0,
            running: false,
        })
    }

    #[inline(always)]
    pub fn start(&mut self) {
        self.running = true;
    }

    #[inline(always)]
    pub fn stop(&mut self) {
        self.running = false;
    }

    #[inline(always)]
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Resets buffer cursors, xrun counters, and interrupt counts.
    pub fn reset(&mut self) {
        self.write_cursor = 0;
        self.hw_read_cursor = 0;
        self.last_period_index = 0;
        self.period_interrupt_count = 0;
        self.xrun_underruns = 0;
        self.xrun_overruns = 0;
        self.running = false;
    }

    /// Returns the number of unread bytes queued in the ring buffer ahead of the DMA read cursor.
    pub fn buffered_bytes(&self) -> usize {
        if self.write_cursor >= self.hw_read_cursor {
            self.write_cursor - self.hw_read_cursor
        } else {
            self.total_size - (self.hw_read_cursor - self.write_cursor)
        }
    }

    /// Returns available byte capacity for writing without overwriting hardware read position.
    pub fn available_write_space(&self) -> usize {
        let buffered = self.buffered_bytes();
        if buffered >= self.total_size {
            0
        } else {
            self.total_size - buffered - self.frame_size
        }
    }

    /// Current hardware period index (0..num_periods - 1).
    #[inline(always)]
    pub fn current_period_index(&self) -> usize {
        if self.period_size == 0 {
            0
        } else {
            (self.hw_read_cursor / self.period_size) % self.num_periods
        }
    }

    /// Software write period index (0..num_periods - 1).
    #[inline(always)]
    pub fn write_period_index(&self) -> usize {
        if self.period_size == 0 {
            0
        } else {
            (self.write_cursor / self.period_size) % self.num_periods
        }
    }

    /// Advances write cursor by writing `bytes_count` bytes.
    pub fn advance_write_cursor(&mut self, bytes_count: usize) -> Result<(), AudioStreamerError> {
        let space = self.available_write_space();
        if bytes_count > space {
            self.xrun_overruns += 1;
            return Err(AudioStreamerError::BufferOverrun);
        }
        self.write_cursor = (self.write_cursor + bytes_count) % self.total_size;
        Ok(())
    }

    /// Updates hardware DMA read position (e.g. from Link Position in Buffer register LPIB).
    /// Detects period boundary crossing, triggers simulated IOC interrupt, and detects underrun.
    pub fn update_hw_position(&mut self, new_pos: usize) -> bool {
        if !self.running {
            return false;
        }

        let clamped_pos = new_pos % self.total_size;
        let old_period = self.last_period_index;
        let new_period = clamped_pos / self.period_size;

        self.hw_read_cursor = clamped_pos;

        // Check if DMA period boundary was crossed
        let period_crossed = new_period != old_period;
        if period_crossed {
            // Count periods elapsed (accounting for wrap-around)
            let periods_elapsed = if new_period >= old_period {
                new_period - old_period
            } else {
                (self.num_periods - old_period) + new_period
            };
            self.period_interrupt_count += periods_elapsed as u64;
            self.last_period_index = new_period;
        }

        // Underrun detection: if hardware caught up to or exceeded write cursor
        // When running, if buffered bytes is zero or near zero while stream is active
        if self.buffered_bytes() == 0 && self.running {
            self.xrun_underruns += 1;
        }

        period_crossed
    }

    /// Calculates current presentation latency in microseconds.
    pub fn latency_us(&self) -> u32 {
        if self.sample_rate == 0 || self.frame_size == 0 {
            return 0;
        }
        let buffered_frames = self.buffered_bytes() / self.frame_size;
        ((buffered_frames as u64 * 1_000_000) / (self.sample_rate as u64)) as u32
    }

    /// Returns reference to hardware Buffer Descriptor List.
    pub fn descriptors(&self) -> &[AudioDmaDescriptor] {
        &self.descriptors[..self.descriptor_count]
    }
}

/// Self-test verifying cyclic DMA ring buffer descriptors, position wrap-around, and latency calculation.
pub fn verify_streamer_ring_buffer() -> bool {
    // 1. Verify descriptor size and bitmask
    if core::mem::size_of::<AudioDmaDescriptor>() != 16 {
        return false;
    }
    let desc = AudioDmaDescriptor::new(0x2_0000_4000, 2048, true);
    if desc.physical_address() != 0x2_0000_4000 || desc.length != 2048 || !desc.has_ioc() {
        return false;
    }

    // 2. Initialize ring buffer: 4 periods of 256 bytes = 1024 bytes total (48 kHz, stereo 16-bit = 4 bytes/frame)
    // 256 bytes = 64 frames per period.
    let mut ring = match AudioRingBuffer::new(0x1000_0000, 1024, 256, 48000, 2, 16) {
        Ok(r) => r,
        Err(_) => return false,
    };

    if ring.num_periods != 4 || ring.frame_size != 4 || ring.descriptor_count != 4 {
        return false;
    }
    if ring.descriptors()[0].physical_address() != 0x1000_0000
        || ring.descriptors()[1].physical_address() != 0x1000_0100
        || ring.descriptors()[2].physical_address() != 0x1000_0200
        || ring.descriptors()[3].physical_address() != 0x1000_0300
    {
        return false;
    }

    // 3. Initial state
    if ring.buffered_bytes() != 0 || ring.current_period_index() != 0 {
        return false;
    }

    // Write 2 periods (512 bytes = 128 frames)
    if ring.advance_write_cursor(512).is_err() {
        return false;
    }
    if ring.buffered_bytes() != 512 || ring.write_period_index() != 2 {
        return false;
    }

    // Check latency for 128 frames at 48kHz: 128 * 1_000_000 / 48000 = 2666 us
    let lat = ring.latency_us();
    if lat != 2666 {
        return false;
    }

    // Sub-2ms check: if we only had 1 period (256 bytes = 64 frames), latency is 64 * 1_000_000 / 48000 = 1333 us (< 2ms)
    // 4. Start streaming and update hardware read cursor
    ring.start();
    if !ring.is_running() {
        return false;
    }

    // Progress within period 0
    let period_elapsed = ring.update_hw_position(128);
    if period_elapsed || ring.period_interrupt_count != 0 {
        return false;
    }

    // Cross into period 1
    let period_elapsed = ring.update_hw_position(256);
    if !period_elapsed || ring.period_interrupt_count != 1 || ring.current_period_index() != 1 {
        return false;
    }

    // Cross into period 2
    let period_elapsed = ring.update_hw_position(512);
    if !period_elapsed || ring.period_interrupt_count != 2 || ring.current_period_index() != 2 {
        return false;
    }

    // Underrun detection: HW reached 512, write cursor was 512, so buffered_bytes is 0!
    if ring.xrun_underruns == 0 {
        return false;
    }

    // Write more data: 256 bytes
    if ring.advance_write_cursor(256).is_err() {
        return false;
    }

    // Wrap-around: HW advances across boundary 1024 -> 0
    let period_elapsed = ring.update_hw_position(768);
    if !period_elapsed || ring.period_interrupt_count != 3 {
        return false;
    }
    let period_elapsed = ring.update_hw_position(0);
    if !period_elapsed || ring.period_interrupt_count != 4 || ring.current_period_index() != 0 {
        return false;
    }

    // Overrun test
    let avail = ring.available_write_space();
    if ring.advance_write_cursor(avail + 10).is_ok() {
        return false;
    }
    if ring.xrun_overruns != 1 {
        return false;
    }

    ring.stop();
    if ring.is_running() {
        return false;
    }

    true
}
