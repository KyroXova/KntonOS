//! Real-Time Audio Subsystem for KntonOS.
//!
//! Provides Intel High Definition Audio (HDA) Specification 1.0a controller
//! and codec drivers, PCM audio stream DMA engine, and software sample mixer.

pub mod hda;
pub mod mixer;

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
    verify_audio_mixer, AudioChannel, AudioMixer, AudioSampleQueue, PcmSample, DEFAULT_VOLUME,
    MAX_CHANNELS, QUEUE_CAPACITY,
};
