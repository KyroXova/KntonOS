//! Low-Latency Zero-Copy Audio Client Protocol & Shared-Memory Ring Buffer.
//! Provides a lock-free Single-Producer Single-Consumer (SPSC) circular ring buffer

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

/// Standard sample rate for the kernel audio subsystem (48 kHz).
pub const AUDIO_SAMPLE_RATE_HZ: u32 = 48_000;

/// Default ring buffer frame capacity (must be a power of two).
/// 2048 frames @ 48 kHz provides ~42.66 ms of buffer headroom.
pub const AUDIO_RING_CAPACITY_FRAMES: usize = 2048;
pub const AUDIO_RING_MASK: usize = AUDIO_RING_CAPACITY_FRAMES - 1;

/// Maximum number of concurrent audio client channels.
pub const MAX_AUDIO_CLIENTS: usize = 16;

/// A stereo audio frame containing 32-bit floating point left and right samples.
/// Samples are normalized within [-1.0, 1.0].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioFrame {
    pub left: f32,
    pub right: f32,
}

impl AudioFrame {
    /// Zero/silent audio frame.
    pub const ZERO: Self = Self {
        left: 0.0,
        right: 0.0,
    };

    /// Creates a stereo frame with distinct left and right channels.
    #[inline]
    pub const fn new(left: f32, right: f32) -> Self {
        Self { left, right }
    }

    /// Creates a mono frame duplicated to both channels.
    #[inline]
    pub const fn mono(val: f32) -> Self {
        Self {
            left: val,
            right: val,
        }
    }
}

/// Lock-free Single-Producer Single-Consumer (SPSC) shared-memory circular ring buffer.
pub struct AudioClientRing {
    /// Circular buffer storing interleaved stereo audio frames.
    buffer: Vec<AudioFrame>,
    /// Capacity in frames (always power of two).
    capacity: usize,
    /// Write index updated monotonically by the producer (client).
    write_index: AtomicUsize,
    /// Read index updated monotonically by the consumer (kernel mixer).
    read_index: AtomicUsize,
    /// Flag indicating whether the producer is actively connected.
    connected: AtomicBool,
    /// Cumulative frames written by client.
    total_written: AtomicU64,
    /// Cumulative frames read by mixer.
    total_read: AtomicU64,
    /// Count of buffer underruns (starvations).
    underrun_count: AtomicU32,
    /// Count of buffer overruns (client writes when full).
    overrun_count: AtomicU32,
}

impl AudioClientRing {
    /// Creates a new lock-free audio client ring buffer.
    pub fn new() -> Self {
        Self {
            buffer: vec![AudioFrame::ZERO; AUDIO_RING_CAPACITY_FRAMES],
            capacity: AUDIO_RING_CAPACITY_FRAMES,
            write_index: AtomicUsize::new(0),
            read_index: AtomicUsize::new(0),
            connected: AtomicBool::new(true),
            total_written: AtomicU64::new(0),
            total_read: AtomicU64::new(0),
            underrun_count: AtomicU32::new(0),
            overrun_count: AtomicU32::new(0),
        }
    }

    /// Number of audio frames currently buffered and ready to read.
    #[inline]
    pub fn available_read(&self) -> usize {
        let w = self.write_index.load(Ordering::Acquire);
        let r = self.read_index.load(Ordering::Acquire);
        w.saturating_sub(r)
    }

    /// Number of free audio frame slots available for writing.
    #[inline]
    pub fn available_write(&self) -> usize {
        let r = self.read_index.load(Ordering::Acquire);
        let w = self.write_index.load(Ordering::Acquire);
        let occupied = w.saturating_sub(r);
        self.capacity.saturating_sub(occupied)
    }

    /// Writes audio frames into the ring buffer from the client producer.
    /// Returns the number of frames actually written.
    pub fn write_frames(&mut self, frames: &[AudioFrame]) -> usize {
        let w = self.write_index.load(Ordering::Relaxed);
        let r = self.read_index.load(Ordering::Acquire);
        let occupied = w.saturating_sub(r);
        let free_space = self.capacity.saturating_sub(occupied);

        if free_space == 0 {
            self.overrun_count.fetch_add(1, Ordering::Relaxed);
            return 0;
        }

        let to_write = core::cmp::min(frames.len(), free_space);
        for i in 0..to_write {
            let slot = (w + i) & AUDIO_RING_MASK;
            self.buffer[slot] = frames[i];
        }

        self.write_index.store(w + to_write, Ordering::Release);
        self.total_written
            .fetch_add(to_write as u64, Ordering::Relaxed);
        to_write
    }

    /// Reads audio frames from the ring buffer into the kernel mixer buffer.
    /// If fewer frames are available than requested, fills the remainder with silence
    pub fn read_frames(&mut self, output: &mut [AudioFrame]) -> usize {
        let r = self.read_index.load(Ordering::Relaxed);
        let w = self.write_index.load(Ordering::Acquire);
        let available = w.saturating_sub(r);

        let actual_read = core::cmp::min(output.len(), available);
        for i in 0..actual_read {
            let slot = (r + i) & AUDIO_RING_MASK;
            output[i] = self.buffer[slot];
        }

        // Starvation / underrun handling: fill deficit with silence
        if actual_read < output.len() {
            self.underrun_count.fetch_add(1, Ordering::Relaxed);
            for i in actual_read..output.len() {
                output[i] = AudioFrame::ZERO;
            }
        }

        if actual_read > 0 {
            self.read_index.store(r + actual_read, Ordering::Release);
            self.total_read
                .fetch_add(actual_read as u64, Ordering::Relaxed);
        }

        actual_read
    }

    /// Resets the ring buffer pointers and clears all audio frame contents.
    pub fn reset(&mut self) {
        self.write_index.store(0, Ordering::Release);
        self.read_index.store(0, Ordering::Release);
        for slot in &mut self.buffer {
            *slot = AudioFrame::ZERO;
        }
    }

    /// Client connection status.
    #[inline]
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    /// Marks the client as disconnected.
    pub fn disconnect(&self) {
        self.connected.store(false, Ordering::Release);
    }

    /// Marks the client as connected.
    pub fn connect(&self) {
        self.connected.store(true, Ordering::Release);
    }

    /// Total underruns encountered.
    #[inline]
    pub fn underrun_count(&self) -> u32 {
        self.underrun_count.load(Ordering::Relaxed)
    }

    /// Total overruns encountered.
    #[inline]
    pub fn overrun_count(&self) -> u32 {
        self.overrun_count.load(Ordering::Relaxed)
    }

    /// Total frames written over lifetime.
    #[inline]
    pub fn total_written(&self) -> u64 {
        self.total_written.load(Ordering::Relaxed)
    }

    /// Total frames read over lifetime.
    #[inline]
    pub fn total_read(&self) -> u64 {
        self.total_read.load(Ordering::Relaxed)
    }

    /// Buffer capacity in frames.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

/// Client audio channel descriptor tracking stream properties, volume, and panning.
pub struct AudioClientChannel {
    /// Unique client stream identifier.
    pub client_id: u32,
    /// Channel name string (up to 32 ASCII bytes).
    pub name: [u8; 32],
    /// Active state flag.
    pub active: bool,
    /// Lock-free shared-memory ring buffer.
    pub ring: AudioClientRing,
    /// Linear volume multiplier (0.0 = silent, 1.0 = unity, 2.0 = +6dB gain).
    pub volume: f32,
    /// Stereo panning position (-1.0 = 100% left, 0.0 = center, 1.0 = 100% right).
    pub pan: f32,
    /// Mute flag.
    pub muted: bool,
    /// Stream priority (0 = lowest, 255 = critical emergency audio).
    pub priority: u8,
    /// Count of consecutive read cycles where the client was starved.
    pub consecutive_underruns: u32,
    /// Stream throughput health state.
    pub health: ClientStreamHealth,
}

/// Health and throughput status of a client audio stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientStreamHealth {
    /// Normal streaming with sufficient buffer headroom.
    Healthy,
    /// Client buffer is running low (< 1 period remaining).
    Warning,
    /// Client buffer is starved / underrun for consecutive cycles.
    Starved,
    /// Client has ceased sending frames and has been quiesced/disconnected.
    Disconnected,
}

impl AudioClientChannel {
    /// Creates a new audio client channel with default parameters.
    pub fn new(client_id: u32, name_str: &str) -> Self {
        let mut name = [0u8; 32];
        let bytes = name_str.as_bytes();
        let len = core::cmp::min(bytes.len(), 31);
        name[..len].copy_from_slice(&bytes[..len]);

        Self {
            client_id,
            name,
            active: true,
            ring: AudioClientRing::new(),
            volume: 1.0,
            pan: 0.0,
            muted: false,
            priority: 128,
            consecutive_underruns: 0,
            health: ClientStreamHealth::Healthy,
        }
    }

    /// Computes effective left and right gain coefficients accounting for volume, pan, and mute.
    #[inline]
    pub fn compute_stereo_gains(&self) -> (f32, f32) {
        if self.muted || !self.active || !self.ring.is_connected() {
            return (0.0, 0.0);
        }

        // Constant-power / linear panning balance
        let pan_clamped = self.pan.clamp(-1.0, 1.0);
        let left_gain = self.volume * (0.5 * (1.0 - pan_clamped));
        let right_gain = self.volume * (0.5 * (1.0 + pan_clamped));

        (left_gain, right_gain)
    }
}

/// Sub-millisecond audio frame pacing engine for real-time mixer cycles.
pub struct AudioFramePacer {
    period_frames: usize,
    period_duration_ns: u64,
    next_deadline_ns: u64,
    period_count: u64,
    total_starvations: u64,
    max_jitter_ns: u64,
}

impl AudioFramePacer {
    /// Creates an audio frame pacer for `period_frames` buffer quantum (e.g. 48 frames = 1ms @ 48kHz).
    pub fn new(period_frames: usize, start_time_ns: u64) -> Self {
        let period_duration_ns =
            (period_frames as u64 * 1_000_000_000) / AUDIO_SAMPLE_RATE_HZ as u64;
        Self {
            period_frames,
            period_duration_ns,
            next_deadline_ns: start_time_ns + period_duration_ns,
            period_count: 0,
            total_starvations: 0,
            max_jitter_ns: 0,
        }
    }

    /// Evaluates if the current time has reached or passed the period deadline.
    #[inline]
    pub fn is_deadline_reached(&self, now_ns: u64) -> bool {
        now_ns >= self.next_deadline_ns
    }

    /// Advances to the next audio period, recording timing jitter.
    pub fn advance_period(&mut self, now_ns: u64) {
        let jitter = if now_ns > self.next_deadline_ns {
            now_ns - self.next_deadline_ns
        } else {
            self.next_deadline_ns - now_ns
        };

        if jitter > self.max_jitter_ns {
            self.max_jitter_ns = jitter;
        }

        self.period_count += 1;
        self.next_deadline_ns += self.period_duration_ns;
    }

    /// Configured audio quantum in frames.
    #[inline]
    pub fn period_frames(&self) -> usize {
        self.period_frames
    }

    /// Duration of a single audio period in nanoseconds.
    #[inline]
    pub fn period_duration_ns(&self) -> u64 {
        self.period_duration_ns
    }

    /// Next presentation deadline in nanoseconds.
    #[inline]
    pub fn next_deadline_ns(&self) -> u64 {
        self.next_deadline_ns
    }

    /// Increments starvation counter.
    #[inline]
    pub fn record_starvation(&mut self) {
        self.total_starvations += 1;
    }

    /// Total starvations encountered across period cycles.
    #[inline]
    pub fn total_starvations(&self) -> u64 {
        self.total_starvations
    }

    /// Total audio periods completed.
    #[inline]
    pub fn period_count(&self) -> u64 {
        self.period_count
    }

    /// Maximum timing jitter observed across period cycles.
    #[inline]
    pub fn max_jitter_ns(&self) -> u64 {
        self.max_jitter_ns
    }
}

/// Central registry managing up to 16 concurrent audio client streams.
pub struct AudioClientManager {
    clients: [Option<AudioClientChannel>; MAX_AUDIO_CLIENTS],
    pacer: AudioFramePacer,
    next_client_id: u32,
    active_client_count: usize,
}

impl AudioClientManager {
    /// Creates a new audio client manager.
    pub fn new(period_frames: usize, start_time_ns: u64) -> Self {
        const NONE_CLIENT: Option<AudioClientChannel> = None;
        Self {
            clients: [NONE_CLIENT; MAX_AUDIO_CLIENTS],
            pacer: AudioFramePacer::new(period_frames, start_time_ns),
            next_client_id: 1,
            active_client_count: 0,
        }
    }

    /// Registers a new audio client, returning its unique `client_id`.
    pub fn register_client(&mut self, name: &str) -> Option<u32> {
        for slot in &mut self.clients {
            if slot.is_none() {
                let id = self.next_client_id;
                self.next_client_id += 1;
                *slot = Some(AudioClientChannel::new(id, name));
                self.active_client_count += 1;
                return Some(id);
            }
        }
        None
    }

    /// Unregisters and disconnects an audio client by its `client_id`.
    pub fn unregister_client(&mut self, client_id: u32) -> bool {
        for slot in &mut self.clients {
            if let Some(ref c) = slot {
                if c.client_id == client_id {
                    *slot = None;
                    self.active_client_count = self.active_client_count.saturating_sub(1);
                    return true;
                }
            }
        }
        false
    }

    /// Pulls frames from a specific client for the current mixer period.
    /// Detects starvation and manages progressive health degradation.
    pub fn pull_client_frames(
        &mut self,
        client_id: u32,
        output: &mut [AudioFrame],
    ) -> (usize, ClientStreamHealth) {
        let channel = match self.get_client_mut(client_id) {
            Some(c) => c,
            None => {
                for f in output.iter_mut() {
                    *f = AudioFrame::ZERO;
                }
                return (0, ClientStreamHealth::Disconnected);
            }
        };

        let requested = output.len();
        let read = channel.ring.read_frames(output);

        if read < requested {
            channel.consecutive_underruns += 1;
            channel.health = if channel.consecutive_underruns >= 5 {
                ClientStreamHealth::Starved
            } else {
                ClientStreamHealth::Warning
            };
        } else {
            channel.consecutive_underruns = 0;
            channel.health = ClientStreamHealth::Healthy;
        }

        (read, channel.health)
    }

    /// Obtains an immutable reference to a client channel.
    pub fn get_client(&self, client_id: u32) -> Option<&AudioClientChannel> {
        for slot in &self.clients {
            if let Some(ref c) = slot {
                if c.client_id == client_id {
                    return Some(c);
                }
            }
        }
        None
    }

    /// Obtains a mutable reference to a client channel.
    pub fn get_client_mut(&mut self, client_id: u32) -> Option<&mut AudioClientChannel> {
        for slot in &mut self.clients {
            if let Some(ref mut c) = slot {
                if c.client_id == client_id {
                    return Some(c);
                }
            }
        }
        None
    }

    /// Access the frame pacer.
    #[inline]
    pub fn pacer(&self) -> &AudioFramePacer {
        &self.pacer
    }

    /// Access the frame pacer mutably.
    #[inline]
    pub fn pacer_mut(&mut self) -> &mut AudioFramePacer {
        &mut self.pacer
    }

    /// Total active clients.
    #[inline]
    pub fn active_client_count(&self) -> usize {
        self.active_client_count
    }

    /// Direct access to client slots for mixer iteration.
    #[inline]
    pub fn client_slots(&self) -> &[Option<AudioClientChannel>; MAX_AUDIO_CLIENTS] {
        &self.clients
    }

    /// Direct mutable access to client slots for mixer iteration.
    #[inline]
    pub fn client_slots_mut(&mut self) -> &mut [Option<AudioClientChannel>; MAX_AUDIO_CLIENTS] {
        &mut self.clients
    }
}

/// Unit test verifying lock-free shared-memory audio client ring buffer and channel descriptors.
pub fn test_audio_client_ring_and_channel() -> bool {
    let mut ring = AudioClientRing::new();

    if ring.capacity() != AUDIO_RING_CAPACITY_FRAMES
        || ring.available_read() != 0
        || ring.available_write() != AUDIO_RING_CAPACITY_FRAMES
    {
        return false;
    }

    // Prepare 512 test audio frames with known sine pattern
    let mut test_frames = [AudioFrame::ZERO; 512];
    for i in 0..512 {
        test_frames[i] = AudioFrame::new(0.5, -0.5);
    }

    // Write 512 frames
    let written = ring.write_frames(&test_frames);
    if written != 512 || ring.available_read() != 512 {
        return false;
    }
    if ring.available_write() != AUDIO_RING_CAPACITY_FRAMES - 512 {
        return false;
    }

    // Read 256 frames
    let mut out_frames = [AudioFrame::ZERO; 256];
    let read_count = ring.read_frames(&mut out_frames);
    if read_count != 256 || ring.available_read() != 256 {
        return false;
    }

    // Verify sample contents
    for f in &out_frames {
        if f.left != 0.5 || f.right != -0.5 {
            return false;
        }
    }

    // Fill remaining capacity
    let fill_count = ring.available_write();
    let fill_buf = vec![AudioFrame::new(0.25, 0.25); fill_count];
    let fill_written = ring.write_frames(&fill_buf);
    if fill_written != fill_count || ring.available_write() != 0 {
        return false;
    }

    // Attempting to write to full ring must return 0 and increment overrun counter
    let overflow_written = ring.write_frames(&test_frames);
    if overflow_written != 0 || ring.overrun_count() != 1 {
        return false;
    }

    // Read all buffered frames
    let total_buffered = ring.available_read();
    let mut drain_buf = vec![AudioFrame::ZERO; total_buffered];
    let drain_read = ring.read_frames(&mut drain_buf);
    if drain_read != total_buffered || ring.available_read() != 0 {
        return false;
    }

    // Test AudioClientChannel gain calculations
    let mut channel = AudioClientChannel::new(1, "game_sfx");
    if channel.client_id != 1 || !channel.active {
        return false;
    }

    // Center pan: gains should be (0.5 * volume, 0.5 * volume)
    let (gl, gr) = channel.compute_stereo_gains();
    if (gl - 0.5).abs() > 1e-5 || (gr - 0.5).abs() > 1e-5 {
        return false;
    }

    // Full left pan (-1.0)
    channel.pan = -1.0;
    let (gl_left, gr_left) = channel.compute_stereo_gains();
    if (gl_left - 1.0).abs() > 1e-5 || gr_left.abs() > 1e-5 {
        return false;
    }

    // Muted channel must yield (0.0, 0.0)
    channel.muted = true;
    let (gl_mute, gr_mute) = channel.compute_stereo_gains();
    if gl_mute != 0.0 || gr_mute != 0.0 {
        return false;
    }

    true
}

/// Unit test verifying sub-millisecond audio frame pacing and client starvation / underrun recovery.
pub fn test_audio_frame_pacing_and_underrun_handler() -> bool {
    // 1. Test AudioFramePacer (48 frames @ 48kHz = exactly 1,000,000 ns = 1 ms)
    let mut pacer = AudioFramePacer::new(48, 0);

    if pacer.period_frames() != 48
        || pacer.period_duration_ns() != 1_000_000
        || pacer.next_deadline_ns() != 1_000_000
        || pacer.period_count() != 0
    {
        return false;
    }

    // 999,999 ns -> deadline not reached
    if pacer.is_deadline_reached(999_999) {
        return false;
    }

    // 1,000,000 ns -> deadline reached
    if !pacer.is_deadline_reached(1_000_000) {
        return false;
    }

    // Advance period at 1,000,050 ns (50 ns jitter)
    pacer.advance_period(1_000_050);
    if pacer.period_count() != 1 || pacer.next_deadline_ns() != 2_000_000 {
        return false;
    }
    if pacer.max_jitter_ns() != 50 {
        return false;
    }

    // 2. Test AudioClientManager registration and progressive starvation tracking
    let mut manager = AudioClientManager::new(48, 0);

    let client_id = match manager.register_client("voice_chat") {
        Some(id) => id,
        None => return false,
    };

    if manager.active_client_count() != 1 {
        return false;
    }

    // Attempt to pull 48 frames when empty: ring returns 0 frames, underrun logged, health = Warning
    let mut period_buffer = [AudioFrame::ZERO; 48];
    let (read_count, health) = manager.pull_client_frames(client_id, &mut period_buffer);
    if read_count != 0 || health != ClientStreamHealth::Warning {
        return false;
    }

    // Simulate 4 more starved periods (total 5) -> transitions to Starved state
    for _ in 0..4 {
        manager.pull_client_frames(client_id, &mut period_buffer);
    }

    let channel = manager.get_client(client_id).unwrap();
    if channel.health != ClientStreamHealth::Starved || channel.consecutive_underruns != 5 {
        return false;
    }

    // Client resumes transmission: writes 96 frames
    let test_data = [AudioFrame::new(0.8, 0.8); 96];
    manager
        .get_client_mut(client_id)
        .unwrap()
        .ring
        .write_frames(&test_data);

    // Pull 48 frames -> fully satisfied, health immediately recovers to Healthy
    let (recovered_read, recovered_health) =
        manager.pull_client_frames(client_id, &mut period_buffer);
    if recovered_read != 48 || recovered_health != ClientStreamHealth::Healthy {
        return false;
    }

    let recovered_channel = manager.get_client(client_id).unwrap();
    if recovered_channel.consecutive_underruns != 0 {
        return false;
    }

    // Unregister client
    if !manager.unregister_client(client_id) || manager.active_client_count() != 0 {
        return false;
    }

    true
}
