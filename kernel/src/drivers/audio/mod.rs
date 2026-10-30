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
