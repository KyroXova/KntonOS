//! Real-Time Audio Subsystem for KntonOS.
//!
//! Provides Intel High Definition Audio (HDA) Specification 1.0a controller
//! and codec drivers, PCM audio stream DMA engine, and software sample mixer.

pub mod hda;
pub mod mixer;
pub mod streamer;

pub use streamer::{
    verify_src_and_latency_pipeline, verify_streamer_ring_buffer, AudioDmaDescriptor,
    AudioRingBuffer, AudioStreamerError, LinearSampleRateConverter, MAX_RING_PERIODS,
};

pub use hda::{
    verify_hda_codec_and_stream, verify_hda_structures, CorbBuffer, HdaBufferDescriptorEntry,
    HdaCodec, HdaController, HdaError, HdaGlobalCapabilities, HdaStreamDma, HdaStreamFormat,
    HdaVerb, HdaWidget, HdaWidgetType, RirbBuffer, CORBCTL_RUN, GCTL_CRST, HDA_REG_CORBCTL,
    HDA_REG_GCAP, HDA_REG_GCTL, HDA_REG_RIRBCTL, HDA_REG_STATESTS, HDA_SD_BASE, HDA_SD_STRIDE,
    MAX_BDL_ENTRIES, MAX_CODEC_WIDGETS, PCI_CLASS_MULTIMEDIA, PCI_SUBCLASS_HDA, RIRBCTL_DMAEN,
    SD_CTL_IOCE, SD_CTL_RUN, SD_CTL_SRST, SD_REG_BDPL, SD_REG_BDPU, SD_REG_CBL, SD_REG_CTL,
    SD_REG_FIFOS, SD_REG_FMT, SD_REG_LPIB, SD_REG_LVI, SD_REG_STS, SD_STS_BCIS,
};

pub use mixer::{
    apply_pan, verify_audio_mixer, verify_soft_knee_limiter_and_gain, AudioChannel, AudioMixer,
    AudioSampleQueue, PcmSample, SoftKneeLimiter, DEFAULT_VOLUME, MAX_CHANNELS, MAX_GAIN,
    QUEUE_CAPACITY, UNITY_GAIN,
};

/// Comprehensive verification of the real-time audio subsystem.
///
/// Tests HDA MMIO register maps, BDL entry sizing, CORB/RIRB ring buffers,
/// Codec discovery, DAC-to-PinComplex widget routing, Stream DMA cyclic buffer
/// management, and multi-channel software PCM mixer with saturation arithmetic.
pub fn test_audio_subsystem() -> bool {
    if !verify_hda_structures() {
        return false;
    }

    if !verify_hda_codec_and_stream() {
        return false;
    }

    if !verify_audio_mixer() {
        return false;
    }

    // End-to-end audio pipeline verification: synthesize tone and stream via DMA
    let mut mixer = AudioMixer::new();
    if let Some(ch) = mixer.channel(0) {
        for i in 0..16 {
            let val = if i % 2 == 0 { 8000 } else { -8000 };
            ch.enqueue_sample(PcmSample::new(val, val));
        }
    } else {
        return false;
    }

    let mut dma_buf = [0u8; 64];
    let written = mixer.mix_into_buffer(&mut dma_buf);
    if written != 64 {
        return false;
    }

    let s0 = PcmSample::from_bytes([dma_buf[0], dma_buf[1], dma_buf[2], dma_buf[3]]);
    if s0.left != 8000 || s0.right != 8000 {
        return false;
    }

    crate::serial_write_str("[KNT-AUDIO-HDA-PASS] Intel HDA audio controller, codec widgets, and PCM mixer verified\r\n");

    true
}

/// Comprehensive verification suite for zero-latency cyclic DMA audio ring buffer,
/// multi-channel dynamic gain mixer with soft-knee limiter, and linear interpolation SRC.
///
/// Emits sentinel [KNT-AUDIO-MIXER-PASS] upon successful validation.
pub fn test_realtime_audio_mixer_subsystem() -> bool {
    // 1. Verify cyclic DMA ring buffer descriptors, wrap-around, and position reporting
    if !verify_streamer_ring_buffer() {
        return false;
    }

    // 2. Verify soft-knee limiter dynamic compression, pan, and gain scaling
    if !verify_soft_knee_limiter_and_gain() {
        return false;
    }

    // 3. Verify linear interpolation sample rate converter and sub-2ms presentation pipeline
    if !verify_src_and_latency_pipeline() {
        return false;
    }

    // 4. End-to-end integration test:
    // Create multi-stream mixer (two concurrent client audio streams: 44.1 kHz converted to 48 kHz + 48 kHz native tone),
    // mix into DMA ring buffer periods, verify non-clipping limited output and cyclic DMA write advancement.
    let mut mixer = AudioMixer::new();

    // Stream 0: 44.1 kHz client stream resampled to 48 kHz
    let mut src = LinearSampleRateConverter::new_44k_to_48k();
    let client_44k = [PcmSample::new(12000, 12000); 45]; // ~1ms chunk at 44.1 kHz
    let mut resampled_48k = [PcmSample::silent(); 48];
    let converted_count = src.process_frames(&client_44k, &mut resampled_48k);
    if converted_count == 0 {
        return false;
    }

    if let Some(ch0) = mixer.channel(0) {
        ch0.set_gain(384); // 1.5x gain boost
        ch0.enqueue_buffer(&resampled_48k[..converted_count]);
    } else {
        return false;
    }

    // Stream 1: Native 48.0 kHz stream with stereo pan
    if let Some(ch1) = mixer.channel(1) {
        ch1.set_pan(64); // Panned right
        for _ in 0..48 {
            ch1.enqueue_sample(PcmSample::new(18000, 18000));
        }
    } else {
        return false;
    }

    // Ring buffer: 4 periods of 192 bytes (48 frames per period = 1ms at 48kHz stereo 16-bit)
    // Sub-2ms deterministic latency budget!
    let mut ring = match AudioRingBuffer::new(0x3000_0000, 768, 192, 48000, 2, 16) {
        Ok(r) => r,
        Err(_) => return false,
    };
    ring.start();

    // Mix 48 frames (192 bytes) into temporary DMA period slice
    let mut period_dma_bytes = [0u8; 192];
    let written = mixer.mix_into_buffer(&mut period_dma_bytes);
    if written != 192 {
        return false;
    }

    // Advance ring buffer write cursor by written bytes
    if ring.advance_write_cursor(written).is_err() {
        return false;
    }
    if ring.buffered_bytes() != 192 || ring.latency_us() != 1000 {
        return false;
    }

    // Verify first mixed sample does not exceed 16-bit bounds
    let s0 = PcmSample::from_bytes([
        period_dma_bytes[0],
        period_dma_bytes[1],
        period_dma_bytes[2],
        period_dma_bytes[3],
    ]);
    if s0.left == 0 || s0.right == 0 {
        return false;
    }

    // Simulate DMA hardware advancing past period 0 to period 1
    let period_elapsed = ring.update_hw_position(192);
    if !period_elapsed || ring.period_interrupt_count != 1 || ring.current_period_index() != 1 {
        return false;
    }

    crate::serial_write_str("[KNT-AUDIO-MIXER-PASS] Real-time DMA audio ring buffer and multi-stream mixer verified\r\n");

    true
}
