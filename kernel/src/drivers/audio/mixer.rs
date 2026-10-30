//! Multi-Channel PCM Software Audio Mixer and Real-Time Soft-Knee Limiter.
//!
//! Provides 16-bit signed stereo sample manipulation, dynamic gain scaling (up to 2.0x boost),
//! stereo panning (-128..=127), soft-knee dynamic headroom compression to prevent clipping
//! distortion, per-channel playback lifecycle control, and DMA buffer rendering.

pub const MAX_CHANNELS: usize = 8;
pub const QUEUE_CAPACITY: usize = 1024;
pub const DEFAULT_VOLUME: u16 = 256; // 1.0 (Unity Gain, range 0..=256)
pub const UNITY_GAIN: u16 = 256;     // 1.0x gain factor
pub const MAX_GAIN: u16 = 512;        // 2.0x gain boost

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

/// Applies stereo pan (-128 = Left, 0 = Center, +127 = Right).
#[inline]
pub fn apply_pan(left: i32, right: i32, pan: i8) -> (i32, i32) {
    if pan == 0 {
        return (left, right);
    }
    if pan <= -127 {
        return (left, 0);
    }
    if pan >= 127 {
        return (0, right);
    }
    let (l_mult, r_mult) = if pan < 0 {
        let p = pan as i32;
        let r_scale = 256 + (p * 2);
        (256, r_scale.clamp(0, 256))
    } else {
        let p = pan as i32;
        let l_scale = 256 - (p * 2);
        (l_scale.clamp(0, 256), 256)
    };
    ((left * l_mult) / 256, (right * r_mult) / 256)
}

/// Real-time Soft-Knee Limiter and Dynamic Headroom Compressor.
///
/// Prevents harsh digital clipping when multiple audio streams sum together,
/// applying a transparent linear curve below `threshold` and a smooth rational
/// asymptotic saturation curve between `threshold` and `ceiling`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoftKneeLimiter {
    pub threshold: i32,
    pub ceiling: i32,
    pub enabled: bool,
    pub clipped_samples_count: u64,
}

impl SoftKneeLimiter {
    pub const DEFAULT_THRESHOLD: i32 = 24576; // 75% full scale (approx -2.5 dB)
    pub const DEFAULT_CEILING: i32 = 32767;   // 100% full scale (0 dBFS)

    pub const fn new(threshold: i32, ceiling: i32) -> Self {
        Self {
            threshold,
            ceiling,
            enabled: true,
            clipped_samples_count: 0,
        }
    }

    pub const fn default_stereo() -> Self {
        Self::new(Self::DEFAULT_THRESHOLD, Self::DEFAULT_CEILING)
    }

    /// Compresses a single audio sample component via rational soft-knee curve.
    pub fn limit_sample(&mut self, val: i32) -> i16 {
        if !self.enabled {
            return val.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }

        let sign = if val < 0 { -1 } else { 1 };
        let mag = val.abs();

        if mag <= self.threshold {
            return val.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }

        self.clipped_samples_count += 1;
        let range = self.ceiling - self.threshold;
        let delta = mag - self.threshold;
        let compressed = ((range as i64 * delta as i64) / ((range + delta) as i64)) as i32;
        let final_mag = (self.threshold + compressed).min(self.ceiling);
        (sign * final_mag) as i16
    }

    /// Limits a stereo pair of 32-bit accumulated samples.
    pub fn limit_stereo(&mut self, left: i32, right: i32) -> (i16, i16) {
        (self.limit_sample(left), self.limit_sample(right))
    }

    #[inline(always)]
    pub fn reset_stats(&mut self) {
        self.clipped_samples_count = 0;
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

/// Independent audio playback channel with dynamic gain and stereo pan.
pub struct AudioChannel {
    pub id: u8,
    pub active: bool,
    pub volume: u16, // 0..=256
    pub gain: u16,   // 0..=512 (256 = 1.0x unity, 512 = 2.0x boost)
    pub pan: i8,     // -128 = Left, 0 = Center, +127 = Right
    pub paused: bool,
    pub muted: bool,
    pub queue: AudioSampleQueue,
}

impl AudioChannel {
    pub const fn new(id: u8) -> Self {
        Self {
            id,
            active: false,
            volume: DEFAULT_VOLUME,
            gain: UNITY_GAIN,
            pan: 0,
            paused: false,
            muted: false,
            queue: AudioSampleQueue::new(),
        }
    }

    pub fn set_volume(&mut self, volume: u16) {
        self.volume = volume.min(256);
    }

    pub fn set_gain(&mut self, gain: u16) {
        self.gain = gain.min(MAX_GAIN);
    }

    pub fn set_pan(&mut self, pan: i8) {
        self.pan = pan;
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
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
        if self.paused {
            return None;
        }
        let sample = self.queue.pop()?;
        if self.queue.is_empty() {
            self.active = false;
        }
        if self.muted {
            Some(PcmSample::silent())
        } else {
            let vol_scaled = sample.scale(self.volume);
            if self.gain != UNITY_GAIN {
                let l = ((vol_scaled.left as i32 * self.gain as i32) / 256).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                let r = ((vol_scaled.right as i32 * self.gain as i32) / 256).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                Some(PcmSample::new(l, r))
            } else {
                Some(vol_scaled)
            }
        }
    }
}

/// Multi-Channel Software Audio Mixer with Soft-Knee Limiting.
pub struct AudioMixer {
    pub channels: [AudioChannel; MAX_CHANNELS],
    pub master_volume: u16, // 0..=256
    pub master_muted: bool,
    pub sample_rate: u32,
    pub limiter: SoftKneeLimiter,
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
            limiter: SoftKneeLimiter::default_stereo(),
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

    pub fn set_channel_gain(&mut self, index: usize, gain: u16) -> bool {
        if let Some(ch) = self.channel(index) {
            ch.set_gain(gain);
            true
        } else {
            false
        }
    }

    pub fn set_channel_pan(&mut self, index: usize, pan: i8) -> bool {
        if let Some(ch) = self.channel(index) {
            ch.set_pan(pan);
            true
        } else {
            false
        }
    }

    pub fn set_channel_paused(&mut self, index: usize, paused: bool) -> bool {
        if let Some(ch) = self.channel(index) {
            ch.set_paused(paused);
            true
        } else {
            false
        }
    }

    pub fn set_limiter_enabled(&mut self, enabled: bool) {
        self.limiter.enabled = enabled;
    }

    pub fn set_limiter_threshold(&mut self, threshold: i32) {
        self.limiter.threshold = threshold;
    }

    pub fn limiter_clipped_count(&self) -> u64 {
        self.limiter.clipped_samples_count
    }

    /// Renders a single mixed stereo sample across all channels with dynamic gain, panning,
    /// 32-bit accumulation, and soft-knee limiter compression.
    pub fn mix_sample(&mut self) -> PcmSample {
        if self.master_muted {
            for ch in &mut self.channels {
                if ch.active && !ch.paused {
                    ch.read_sample();
                }
            }
            return PcmSample::silent();
        }

        let mut acc_left: i32 = 0;
        let mut acc_right: i32 = 0;
        let mut has_active = false;

        for ch in &mut self.channels {
            if ch.active && !ch.paused {
                let pan = ch.pan;
                if let Some(sample) = ch.read_sample() {
                    let (pl, pr) = apply_pan(sample.left as i32, sample.right as i32, pan);
                    acc_left += pl;
                    acc_right += pr;
                    has_active = true;
                }
            }
        }

        if !has_active {
            return PcmSample::silent();
        }

        // Apply soft-knee limiter compression
        let (lim_left, lim_right) = self.limiter.limit_stereo(acc_left, acc_right);
        let sample = PcmSample::new(lim_left, lim_right);
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

/// Verification of dynamic gain, panning, and soft-knee limiter compression.
pub fn verify_soft_knee_limiter_and_gain() -> bool {
    // 1. Standalone limiter test
    let mut limiter = SoftKneeLimiter::default_stereo();

    // Below threshold: linear transparency
    if limiter.limit_sample(10000) != 10000 || limiter.limit_sample(-10000) != -10000 {
        return false;
    }
    if limiter.limit_sample(24576) != 24576 {
        return false;
    }
    if limiter.clipped_samples_count != 0 {
        return false;
    }

    // Above threshold: smooth saturation curve without hard clipping
    let compressed_pos = limiter.limit_sample(40000);
    if compressed_pos != 29925 && compressed_pos != 29926 {
        return false;
    }
    let compressed_neg = limiter.limit_sample(-40000);
    if compressed_neg != -29925 && compressed_neg != -29926 {
        return false;
    }
    if limiter.clipped_samples_count != 2 {
        return false;
    }

    // Extreme sum (e.g. 5 loud streams summing to 150,000)
    let extreme = limiter.limit_sample(150000);
    if extreme < 32000 {
        return false;
    }

    // 2. Panning test
    let (l_full, r_full) = apply_pan(10000, 10000, -128); // Full left
    if l_full != 10000 || r_full != 0 {
        return false;
    }
    let (l_right, r_right) = apply_pan(10000, 10000, 127); // Full right
    if l_right != 0 || r_right != 10000 {
        return false;
    }
    let (l_mid, r_mid) = apply_pan(10000, 10000, 0); // Center
    if l_mid != 10000 || r_mid != 10000 {
        return false;
    }

    // 3. Multi-stream mixer with dynamic gain and limiter
    let mut mixer = AudioMixer::new();

    // Stream 0: Gain 1.5x (384) -> 10,000 * 384 / 256 = 15,000
    if let Some(ch0) = mixer.channel(0) {
        ch0.set_gain(384);
        ch0.enqueue_sample(PcmSample::new(10000, 10000));
    } else {
        return false;
    }

    // Stream 1: Gain 2.0x (512) -> 10,000 * 512 / 256 = 20,000
    if let Some(ch1) = mixer.channel(1) {
        ch1.set_gain(512);
        ch1.enqueue_sample(PcmSample::new(10000, 10000));
    } else {
        return false;
    }

    // Stream 2: Pan full Left (-128) -> Left = 10,000, Right = 0
    if let Some(ch2) = mixer.channel(2) {
        ch2.set_pan(-128);
        ch2.enqueue_sample(PcmSample::new(10000, 10000));
    } else {
        return false;
    }

    // Acc Left: 15,000 + 20,000 + 10,000 = 45,000 (exceeds threshold 24,576!)
    // Acc Right: 15,000 + 20,000 + 0 = 35,000 (exceeds threshold 24,576!)
    let mixed = mixer.mix_sample();

    // Verify soft-knee limiter compressed the outputs safely below i16::MAX (32767)
    if mixed.left <= 24576 {
        return false;
    }
    if mixed.right <= 24576 {
        return false;
    }
    // Left had higher input (45,000) than Right (35,000), so compressed Left must be > Right
    if mixed.left <= mixed.right {
        return false;
    }
    if mixer.limiter_clipped_count() < 2 {
        return false;
    }

    // 4. Pausing verification
    if let Some(ch0) = mixer.channel(0) {
        ch0.enqueue_sample(PcmSample::new(5000, 5000));
        ch0.set_paused(true);
    }
    let silent_mix = mixer.mix_sample();
    if silent_mix != PcmSample::silent() {
        return false;
    }

    true
}
