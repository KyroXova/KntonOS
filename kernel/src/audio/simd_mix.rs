//! High-Performance Multi-Stream Audio Mixer & TPDF Dithering Sample Converter.
//!
//! Performs 32-bit floating point accumulation across up to 16 concurrent audio client streams.
//! Implements independent stream volume scaling, stereo balance/panning, asymptotic soft
//! clipping to prevent digital overdrive, and Triangular Probability Density Function (TPDF)
//! dithering for artifact-free float-to-int16 PCM quantization.

use super::client_ring::{AudioClientManager, AudioFrame};
use alloc::vec;
use alloc::vec::Vec;

/// Maximum number of audio frames mixed in a single quantum buffer (e.g. 256 frames = 5.33ms).
pub const MAX_MIX_BUFFER_FRAMES: usize = 256;

/// Scale factor to convert normalized float [-1.0, 1.0] to 16-bit signed PCM integer.
pub const PCM_SCALE_I16: f32 = 32767.0;

/// Fast pseudo-random number generator for audio dithering (Xorshift32).
#[inline]
fn xorshift32(state: &mut u32) -> u32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    x
}

/// Generates a Triangular Probability Density Function (TPDF) dither value in [-1.0, 1.0] LSB.
/// Formed by summing two independent uniform random variables in [-0.5, 0.5] LSB.
#[inline]
pub fn generate_tpdf_dither(state: &mut u32) -> f32 {
    let r1 = ((xorshift32(state) & 0xFFFF) as f32 / 65535.0) - 0.5;
    let r2 = ((xorshift32(state) & 0xFFFF) as f32 / 65535.0) - 0.5;
    (r1 + r2) / PCM_SCALE_I16
}

/// Smooth asymptotic soft clipping saturator.
/// Preserves strict linearity for signals within [-0.8, 0.8], then smoothly compresses
/// excess peaks towards +/-1.0 without harmonic distortion.
#[inline]
pub fn soft_clip(sample: f32) -> f32 {
    const THRESHOLD: f32 = 0.8;
    if sample > THRESHOLD {
        let excess = sample - THRESHOLD;
        THRESHOLD + (1.0 - THRESHOLD) * (excess / (1.0 + excess))
    } else if sample < -THRESHOLD {
        let excess = -THRESHOLD - sample;
        -(THRESHOLD + (1.0 - THRESHOLD) * (excess / (1.0 + excess)))
    } else {
        sample
    }
}

/// Real-time multi-stream audio mixer and sample format converter.
pub struct AudioSimdMixer {
    /// Master playback volume (1.0 = unity gain).
    master_volume: f32,
    /// PRNG state for TPDF dithering.
    dither_prng_state: u32,
    /// Flag enabling TPDF dithering during float-to-int16 quantization.
    dither_enabled: bool,
    /// Flag enabling soft clipping saturation.
    soft_clip_enabled: bool,
    /// Cumulative frames processed.
    total_mixed_frames: u64,
    /// Peak amplitude observed across channels.
    peak_amplitude: f32,
    /// Accumulation buffer for left channel (f32).
    accum_left: Vec<f32>,
    /// Accumulation buffer for right channel (f32).
    accum_right: Vec<f32>,
    /// Scratch buffer for client frame pulls.
    client_scratch: Vec<AudioFrame>,
}

impl AudioSimdMixer {
    /// Creates a new audio mixer initialized with unity master volume and dithering enabled.
    pub fn new() -> Self {
        Self {
            master_volume: 1.0,
            dither_prng_state: 0x1337_ACDC,
            dither_enabled: true,
            soft_clip_enabled: true,
            total_mixed_frames: 0,
            peak_amplitude: 0.0,
            accum_left: vec![0.0; MAX_MIX_BUFFER_FRAMES],
            accum_right: vec![0.0; MAX_MIX_BUFFER_FRAMES],
            client_scratch: vec![AudioFrame::ZERO; MAX_MIX_BUFFER_FRAMES],
        }
    }

    /// Sets the master output volume multiplier.
    pub fn set_master_volume(&mut self, vol: f32) {
        self.master_volume = vol.max(0.0);
    }

    /// Enables or disables TPDF dithering.
    pub fn set_dithering(&mut self, enabled: bool) {
        self.dither_enabled = enabled;
    }

    /// Enables or disables soft clipping.
    pub fn set_soft_clipping(&mut self, enabled: bool) {
        self.soft_clip_enabled = enabled;
    }

    /// Pulls frames from all active clients in `manager`, performs 32-bit floating point mixing,
    /// applies volume scaling, panning, soft clipping, and writes interleaved int16 PCM to `out_pcm`.
    /// `out_pcm` length must be at least `frames_to_mix * 2`. Returns the number of stereo frames mixed.
    pub fn mix_clients(
        &mut self,
        manager: &mut AudioClientManager,
        frames_to_mix: usize,
        out_pcm_interleaved: &mut [i16],
    ) -> usize {
        let frame_count = core::cmp::min(frames_to_mix, MAX_MIX_BUFFER_FRAMES);
        if out_pcm_interleaved.len() < frame_count * 2 {
            return 0;
        }

        // Clear accumulation buffers
        for i in 0..frame_count {
            self.accum_left[i] = 0.0;
            self.accum_right[i] = 0.0;
        }

        // Iterate through all 16 client channel slots
        let client_slots = manager.client_slots_mut();
        for slot in client_slots.iter_mut() {
            if let Some(ref mut channel) = slot {
                if !channel.active || channel.muted || !channel.ring.is_connected() {
                    continue;
                }

                let (gain_left, gain_right) = channel.compute_stereo_gains();
                if gain_left == 0.0 && gain_right == 0.0 {
                    continue;
                }

                // Pull frames from client ring buffer
                let read = channel
                    .ring
                    .read_frames(&mut self.client_scratch[..frame_count]);

                // Accumulate with vectorization-friendly unrolling
                let chunks = frame_count / 4;
                let remainder = frame_count % 4;

                for c in 0..chunks {
                    let base = c * 4;
                    self.accum_left[base] += self.client_scratch[base].left * gain_left;
                    self.accum_right[base] += self.client_scratch[base].right * gain_right;

                    self.accum_left[base + 1] += self.client_scratch[base + 1].left * gain_left;
                    self.accum_right[base + 1] += self.client_scratch[base + 1].right * gain_right;

                    self.accum_left[base + 2] += self.client_scratch[base + 2].left * gain_left;
                    self.accum_right[base + 2] += self.client_scratch[base + 2].right * gain_right;

                    self.accum_left[base + 3] += self.client_scratch[base + 3].left * gain_left;
                    self.accum_right[base + 3] += self.client_scratch[base + 3].right * gain_right;
                }

                for r in 0..remainder {
                    let idx = chunks * 4 + r;
                    self.accum_left[idx] += self.client_scratch[idx].left * gain_left;
                    self.accum_right[idx] += self.client_scratch[idx].right * gain_right;
                }

                // Update channel health
                if read < frame_count {
                    channel.consecutive_underruns += 1;
                } else {
                    channel.consecutive_underruns = 0;
                }
            }
        }

        // Post-processing: Master volume, soft clipping, dithering, and int16 conversion
        let master = self.master_volume;
        let dither = self.dither_enabled;
        let soft_clip_on = self.soft_clip_enabled;

        for i in 0..frame_count {
            let mut left_f = self.accum_left[i] * master;
            let mut right_f = self.accum_right[i] * master;

            // Track peak amplitude
            let peak_cur = left_f.abs().max(right_f.abs());
            if peak_cur > self.peak_amplitude {
                self.peak_amplitude = peak_cur;
            }

            // Soft clipping
            if soft_clip_on {
                left_f = soft_clip(left_f);
                right_f = soft_clip(right_f);
            }

            // TPDF Dithering
            if dither {
                left_f += generate_tpdf_dither(&mut self.dither_prng_state);
                right_f += generate_tpdf_dither(&mut self.dither_prng_state);
            }

            // Quantization to int16 PCM
            let pcm_left = (left_f * PCM_SCALE_I16).clamp(-32768.0, 32767.0) as i16;
            let pcm_right = (right_f * PCM_SCALE_I16).clamp(-32768.0, 32767.0) as i16;

            out_pcm_interleaved[i * 2] = pcm_left;
            out_pcm_interleaved[i * 2 + 1] = pcm_right;
        }

        self.total_mixed_frames += frame_count as u64;
        frame_count
    }

    /// Master volume.
    #[inline]
    pub fn master_volume(&self) -> f32 {
        self.master_volume
    }

    /// Peak amplitude observed.
    #[inline]
    pub fn peak_amplitude(&self) -> f32 {
        self.peak_amplitude
    }

    /// Total frames mixed.
    #[inline]
    pub fn total_mixed_frames(&self) -> u64 {
        self.total_mixed_frames
    }
}

/// Unit test verifying multi-stream mixing, panning gains, soft clipping, and TPDF dithering.
pub fn test_simd_mixer_and_tpdf_dithering() -> bool {
    let mut mixer = AudioSimdMixer::new();
    let mut manager = AudioClientManager::new(48, 0);

    // Register two audio clients:
    // Client 1 ("game_sfx"): amplitude 0.3, center pan (0.0)
    // Client 2 ("bgm"): amplitude 0.2, center pan (0.0)
    let c1_id = manager.register_client("game_sfx").unwrap();
    let c2_id = manager.register_client("bgm").unwrap();

    let c1 = manager.get_client_mut(c1_id).unwrap();
    let sfx_frames = [AudioFrame::new(0.3, 0.3); 48];
    c1.ring.write_frames(&sfx_frames);

    let c2 = manager.get_client_mut(c2_id).unwrap();
    let bgm_frames = [AudioFrame::new(0.2, 0.2); 48];
    c2.ring.write_frames(&bgm_frames);

    let mut out_pcm = [0i16; 96]; // 48 frames * 2 channels = 96 interleaved samples
    let mixed = mixer.mix_clients(&mut manager, 48, &mut out_pcm);
    if mixed != 48 {
        return false;
    }

    // Expected mixed amplitude:
    // Client 1 center gain: 0.3 * (0.5 * 1.0) = 0.15
    // Client 2 center gain: 0.2 * (0.5 * 1.0) = 0.10
    // Sum: 0.25 -> ~8191 in 16-bit PCM (allow small tolerance for TPDF dither noise)
    let expected_pcm = (0.25 * PCM_SCALE_I16) as i16;
    for sample in &out_pcm {
        if (sample - expected_pcm).abs() > 3 {
            return false;
        }
    }

    // 2. Test Soft Clipping Overdrive Saturation
    // Inject massive overdrive (amplitude 5.0) on client 1
    let c1 = manager.get_client_mut(c1_id).unwrap();
    let overdrive_frames = [AudioFrame::new(5.0, 5.0); 48];
    c1.ring.write_frames(&overdrive_frames);

    let mut over_pcm = [0i16; 96];
    mixer.mix_clients(&mut manager, 48, &mut over_pcm);

    // Output must be saturated high near 32767 without wrapping or negative rollover
    for &sample in &over_pcm {
        if sample < 30000 {
            return false;
        }
    }

    // 3. Test TPDF Dithering Distribution
    // Generate 1000 dither values, verify peak within +/- 1 LSB (< 1.0 / 32767.0)
    let mut prng = 0xDEAD_BEEF;
    let lsb = 1.0 / PCM_SCALE_I16;
    for _ in 0..1000 {
        let d = generate_tpdf_dither(&mut prng);
        if d.abs() > lsb * 1.05 {
            return false;
        }
    }

    true
}
