//! Multi-Channel PCM Software Audio Mixer and Kernel Sample Queue.
//!
//! Provides 16-bit signed stereo sample manipulation, saturating arithmetic,
//! per-channel volume scaling, master volume control, and DMA buffer rendering.

pub const MAX_CHANNELS: usize = 8;
pub const QUEUE_CAPACITY: usize = 1024;
pub const DEFAULT_VOLUME: u16 = 256; // 1.0 (Unity Gain, range 0..=256)

/// 16-bit Signed Stereo Audio Sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmSample {
    pub left: i16,
    pub right: i16,
}

impl PcmSample {
    pub const fn new(left: i16, right: i16) -> Self {
        Self { left, right }
    }

    pub const fn silent() -> Self {
        Self { left: 0, right: 0 }
    }

    /// Scales sample amplitude by a volume factor in 0..=256 (where 256 = 100%).
    #[inline]
    pub fn scale(&self, volume: u16) -> Self {
        let vol = volume.min(256) as i32;
        let l = ((self.left as i32 * vol) / 256) as i16;
        let r = ((self.right as i32 * vol) / 256) as i16;
        Self { left: l, right: r }
    }

    /// Clamped addition preventing integer overflow / distortion wrapping.
    #[inline]
    pub fn saturating_add(&self, other: &Self) -> Self {
        let l = (self.left as i32 + other.left as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        let r = (self.right as i32 + other.right as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        Self { left: l, right: r }
    }

    /// Serializes stereo sample to 4 little-endian bytes [L_lo, L_hi, R_lo, R_hi].
    #[inline]
    pub fn to_bytes(&self) -> [u8; 4] {
        let l = self.left.to_le_bytes();
        let r = self.right.to_le_bytes();
        [l[0], l[1], r[0], r[1]]
    }

    /// Deserializes stereo sample from 4 little-endian bytes.
    #[inline]
    pub fn from_bytes(bytes: [u8; 4]) -> Self {
        let left = i16::from_le_bytes([bytes[0], bytes[1]]);
        let right = i16::from_le_bytes([bytes[2], bytes[3]]);
        Self { left, right }
    }
}

/// Circular FIFO Queue for PCM audio samples.
pub struct AudioSampleQueue {
    samples: [PcmSample; QUEUE_CAPACITY],
    head: usize,
    tail: usize,
    count: usize,
}

impl AudioSampleQueue {
    pub const fn new() -> Self {
        Self {
            samples: [PcmSample::silent(); QUEUE_CAPACITY],
            head: 0,
            tail: 0,
            count: 0,
        }
    }

    pub fn push(&mut self, sample: PcmSample) -> bool {
        if self.count >= QUEUE_CAPACITY {
            return false;
        }
        self.samples[self.tail] = sample;
        self.tail = (self.tail + 1) % QUEUE_CAPACITY;
        self.count += 1;
        true
    }

    pub fn pop(&mut self) -> Option<PcmSample> {
        if self.count == 0 {
            return None;
        }
        let sample = self.samples[self.head];
        self.head = (self.head + 1) % QUEUE_CAPACITY;
        self.count -= 1;
        Some(sample)
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.count
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    #[inline(always)]
    pub fn is_full(&self) -> bool {
        self.count >= QUEUE_CAPACITY
    }

    pub fn clear(&mut self) {
        self.head = 0;
        self.tail = 0;
        self.count = 0;
    }
}

/// Independent audio playback channel.
pub struct AudioChannel {
    pub id: u8,
    pub active: bool,
    pub volume: u16, // 0..=256
    pub muted: bool,
    pub queue: AudioSampleQueue,
}

impl AudioChannel {
    pub const fn new(id: u8) -> Self {
        Self {
            id,
            active: false,
            volume: DEFAULT_VOLUME,
            muted: false,
            queue: AudioSampleQueue::new(),
        }
    }

    pub fn set_volume(&mut self, volume: u16) {
        self.volume = volume.min(256);
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    pub fn enqueue_sample(&mut self, sample: PcmSample) -> bool {
        self.active = true;
        self.queue.push(sample)
    }

    pub fn enqueue_buffer(&mut self, samples: &[PcmSample]) -> usize {
        self.active = true;
        let mut count = 0;
        for &s in samples {
            if !self.queue.push(s) {
                break;
            }
            count += 1;
        }
        count
    }

    pub fn read_sample(&mut self) -> Option<PcmSample> {
        let sample = self.queue.pop()?;
        if self.queue.is_empty() {
            self.active = false;
        }
        if self.muted {
            Some(PcmSample::silent())
        } else {
            Some(sample.scale(self.volume))
        }
    }
}

/// Multi-Channel Software Audio Mixer.
pub struct AudioMixer {
    pub channels: [AudioChannel; MAX_CHANNELS],
    pub master_volume: u16, // 0..=256
    pub master_muted: bool,
    pub sample_rate: u32,
}

impl AudioMixer {
    pub const fn new() -> Self {
        Self {
            channels: [
                AudioChannel::new(0),
                AudioChannel::new(1),
                AudioChannel::new(2),
                AudioChannel::new(3),
                AudioChannel::new(4),
                AudioChannel::new(5),
                AudioChannel::new(6),
                AudioChannel::new(7),
            ],
            master_volume: DEFAULT_VOLUME,
            master_muted: false,
            sample_rate: 48000,
        }
    }

    pub fn set_master_volume(&mut self, volume: u16) {
        self.master_volume = volume.min(256);
    }

    pub fn set_master_muted(&mut self, muted: bool) {
        self.master_muted = muted;
    }

    pub fn channel(&mut self, index: usize) -> Option<&mut AudioChannel> {
        if index < MAX_CHANNELS {
            Some(&mut self.channels[index])
        } else {
            None
        }
    }

    /// Renders a single mixed stereo sample across all channels with saturation clamping.
    pub fn mix_sample(&mut self) -> PcmSample {
        if self.master_muted {
            // Still consume samples from active channels to prevent queues from lagging behind
            for ch in &mut self.channels {
                if ch.active {
                    ch.read_sample();
                }
            }
            return PcmSample::silent();
        }

        let mut acc_left: i32 = 0;
        let mut acc_right: i32 = 0;
        let mut has_active = false;

        for ch in &mut self.channels {
            if ch.active {
                if let Some(sample) = ch.read_sample() {
                    acc_left += sample.left as i32;
                    acc_right += sample.right as i32;
                    has_active = true;
                }
            }
        }

        if !has_active {
            return PcmSample::silent();
        }

        // Apply saturation clamping to prevent wrapping artifacts
        let clamped_left = acc_left.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        let clamped_right = acc_right.clamp(i16::MIN as i32, i16::MAX as i32) as i16;

        let sample = PcmSample::new(clamped_left, clamped_right);
        sample.scale(self.master_volume)
    }

    /// Mixes audio samples directly into a raw byte buffer formatted for Intel HDA DMA playback.
    /// Each sample occupies 4 bytes (16-bit left little-endian, 16-bit right little-endian).
    pub fn mix_into_buffer(&mut self, out_buffer: &mut [u8]) -> usize {
        let sample_count = out_buffer.len() / 4;
        let mut bytes_written = 0;

        for i in 0..sample_count {
            let sample = self.mix_sample();
            let bytes = sample.to_bytes();
            let offset = i * 4;
            out_buffer[offset..offset + 4].copy_from_slice(&bytes);
            bytes_written += 4;
        }

        bytes_written
    }
}

/// Self-test verifying sample arithmetic, queue FIFO operations, and multi-channel mixing.
pub fn verify_audio_mixer() -> bool {
    // 1. Verify PcmSample saturating arithmetic and byte conversion
    let s1 = PcmSample::new(20000, -20000);
    let s2 = PcmSample::new(20000, -20000);
    let sum = s1.saturating_add(&s2);
    if sum.left != i16::MAX || sum.right != i16::MIN {
        return false;
    }

    let bytes = s1.to_bytes();
    let s1_roundtrip = PcmSample::from_bytes(bytes);
    if s1_roundtrip != s1 {
        return false;
    }

    // 2. Verify AudioSampleQueue operations
    let mut queue = AudioSampleQueue::new();
    if !queue.is_empty() {
        return false;
    }

    for i in 0..5 {
        queue.push(PcmSample::new(i as i16, -(i as i16)));
    }
    if queue.len() != 5 {
        return false;
    }

    for i in 0..5 {
        let s = match queue.pop() {
            Some(val) => val,
            None => return false,
        };
        if s.left != i as i16 || s.right != -(i as i16) {
            return false;
        }
    }
    if !queue.is_empty() {
        return false;
    }

    // 3. Verify Multi-Channel AudioMixer
    let mut mixer = AudioMixer::new();

    // Channel 0: Unity gain (256)
    if let Some(ch0) = mixer.channel(0) {
        ch0.set_volume(256);
        ch0.enqueue_sample(PcmSample::new(10000, 5000));
    } else {
        return false;
    }

    // Channel 1: Half volume (128) -> 5000 / 2 = 2500 L, 10000 / 2 = 5000 R
    if let Some(ch1) = mixer.channel(1) {
        ch1.set_volume(128);
        ch1.enqueue_sample(PcmSample::new(5000, 10000));
    } else {
        return false;
    }

    // Mixed sample should be (10000 + 2500, 5000 + 5000) = (12500, 10000)
    let mixed = mixer.mix_sample();
    if mixed.left != 12500 || mixed.right != 10000 {
        return false;
    }

    // 4. Verify buffer rendering
    let mut out_buf = [0u8; 16];
    if let Some(ch) = mixer.channel(0) {
        ch.enqueue_sample(PcmSample::new(1000, 2000));
        ch.enqueue_sample(PcmSample::new(3000, 4000));
    }
    let written = mixer.mix_into_buffer(&mut out_buf);
    if written != 16 {
        return false;
    }

    // First sample in buffer should be (1000, 2000)
    let sample0 = PcmSample::from_bytes([out_buf[0], out_buf[1], out_buf[2], out_buf[3]]);
    if sample0.left != 1000 || sample0.right != 2000 {
        return false;
    }

    // Second sample in buffer should be (3000, 4000)
    let sample1 = PcmSample::from_bytes([out_buf[4], out_buf[5], out_buf[6], out_buf[7]]);
    if sample1.left != 3000 || sample1.right != 4000 {
        return false;
    }

    // Third sample should be silent (0, 0) since queues emptied
    let sample2 = PcmSample::from_bytes([out_buf[8], out_buf[9], out_buf[10], out_buf[11]]);
    if sample2 != PcmSample::silent() {
        return false;
    }

    true
}
