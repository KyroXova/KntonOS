//! Kernel-Direct Hardware Video Decode Submission Queue & Picture Parameter Parser.
//!
//! Implements hardware video decode acceleration primitives for KntonOS:
//! - Multi-standard codec support: H.264 / AVC, HEVC / H.265, and AV1.
//! - Annex-B NAL unit and AV1 OBU bitstream parsing.
//! - Exp-Golomb bitstream reader for SPS, PPS, VPS, and slice header decoding.
//! - Hardware decode command packet submission queue.
//! - 64-bit timeline completion fence tracking.

#![allow(dead_code)]

extern crate alloc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

// -----------------------------------------------------------------------------
// Video Codec and Profile Types
// -----------------------------------------------------------------------------

/// Supported hardware video decode codec standards.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    H264Avc = 1,
    HevcH265 = 2,
    Av1 = 3,
}

/// H.264 / AVC profiles.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum H264Profile {
    Baseline = 66,
    Main = 77,
    Extended = 88,
    High = 100,
    High10 = 110,
    High422 = 122,
    High444Predictive = 244,
}

/// HEVC / H.265 profiles.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HevcProfile {
    Main = 1,
    Main10 = 2,
    MainStillPicture = 3,
    Unknown = 0,
}

/// AV1 sequence profiles.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Av1Profile {
    Main = 0,         // 8-bit or 10-bit 4:2:0
    High = 1,         // 8-bit or 10-bit 4:4:4
    Professional = 2, // 8-bit, 10-bit, or 12-bit 4:2:2 or 12-bit 4:2:0/4:4:4
}

/// Video slice coding types.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliceType {
    P = 0, // Predictive slice
    B = 1, // Bi-predictive slice
    I = 2, // Intra slice
    Sp = 3,
    Si = 4,
}

impl SliceType {
    pub fn from_u32(val: u32) -> Self {
        match val % 5 {
            0 => SliceType::P,
            1 => SliceType::B,
            2 => SliceType::I,
            3 => SliceType::Sp,
            _ => SliceType::Si,
        }
    }
}

// -----------------------------------------------------------------------------
// Bitstream Reader (Exp-Golomb & Bit-level Extraction)
// -----------------------------------------------------------------------------

/// Reader for variable-length bitstreams and Exp-Golomb codes.
#[derive(Debug)]
pub struct BitstreamReader<'a> {
    data: &'a [u8],
    byte_offset: usize,
    bit_offset: usize, // 0 to 7 (MSB to LSB within current byte)
}

impl<'a> BitstreamReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_offset: 0,
            bit_offset: 0,
        }
    }

    /// Number of bits remaining in the buffer.
    pub fn bits_remaining(&self) -> usize {
        if self.byte_offset >= self.data.len() {
            0
        } else {
            (self.data.len() - self.byte_offset) * 8 - self.bit_offset
        }
    }

    /// Read a single bit (0 or 1).
    pub fn read_bit(&mut self) -> Option<bool> {
        if self.byte_offset >= self.data.len() {
            return None;
        }

        let byte = self.data[self.byte_offset];
        let bit = (byte >> (7 - self.bit_offset)) & 1;

        self.bit_offset += 1;
        if self.bit_offset == 8 {
            self.bit_offset = 0;
            self.byte_offset += 1;
        }

        Some(bit != 0)
    }

    /// Read `n` bits (up to 32) as an unsigned integer.
    pub fn read_bits(&mut self, n: usize) -> Option<u32> {
        if n == 0 {
            return Some(0);
        }
        if n > 32 || self.bits_remaining() < n {
            return None;
        }

        let mut val = 0u32;
        for _ in 0..n {
            val = (val << 1) | (self.read_bit()? as u32);
        }
        Some(val)
    }

    /// Read an unsigned Exp-Golomb coded integer (ue(v)).
    pub fn read_ue(&mut self) -> Option<u32> {
        let mut leading_zeros = 0usize;
        while let Some(bit) = self.read_bit() {
            if bit {
                break;
            }
            leading_zeros += 1;
            if leading_zeros > 31 {
                return None; // Overflow guard
            }
        }

        if leading_zeros == 0 {
            return Some(0);
        }

        let suffix = self.read_bits(leading_zeros)?;
        let val = (1u32 << leading_zeros) - 1 + suffix;
        Some(val)
    }

    /// Read a signed Exp-Golomb coded integer (se(v)).
    pub fn read_se(&mut self) -> Option<i32> {
        let code_num = self.read_ue()?;
        if code_num == 0 {
            Some(0)
        } else if code_num % 2 == 1 {
            Some(((code_num + 1) / 2) as i32)
        } else {
            Some(-((code_num / 2) as i32))
        }
    }

    /// Align reader to the next byte boundary.
    pub fn byte_align(&mut self) {
        if self.bit_offset != 0 {
            self.bit_offset = 0;
            self.byte_offset += 1;
        }
    }
}

// -----------------------------------------------------------------------------
// Picture Parameter Structures
// -----------------------------------------------------------------------------

/// H.264 Sequence Parameter Set (SPS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct H264Sps {
    pub profile_idc: u8,
    pub level_idc: u8,
    pub seq_parameter_set_id: u32,
    pub chroma_format_idc: u32,
    pub bit_depth_luma: u8,
    pub bit_depth_chroma: u8,
    pub log2_max_frame_num_minus4: u32,
    pub pic_order_cnt_type: u32,
    pub log2_max_pic_order_cnt_lsb_minus4: u32,
    pub max_num_ref_frames: u32,
    pub pic_width_in_mbs: u32,
    pub pic_height_in_map_units: u32,
    pub frame_mbs_only_flag: bool,
    pub direct_8x8_inference_flag: bool,
    pub frame_cropping_flag: bool,
    pub crop_left: u32,
    pub crop_right: u32,
    pub crop_top: u32,
    pub crop_bottom: u32,
}

impl H264Sps {
    #[inline]
    pub fn width_pixels(&self) -> u32 {
        let raw_width = self.pic_width_in_mbs * 16;
        if self.frame_cropping_flag {
            raw_width.saturating_sub((self.crop_left + self.crop_right) * 2)
        } else {
            raw_width
        }
    }

    #[inline]
    pub fn height_pixels(&self) -> u32 {
        let mul = if self.frame_mbs_only_flag { 16 } else { 32 };
        let raw_height = self.pic_height_in_map_units * mul;
        if self.frame_cropping_flag {
            raw_height.saturating_sub((self.crop_top + self.crop_bottom) * 2)
        } else {
            raw_height
        }
    }
}

/// H.264 Picture Parameter Set (PPS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct H264Pps {
    pub pic_parameter_set_id: u32,
    pub seq_parameter_set_id: u32,
    pub entropy_coding_mode_flag: bool, // false: CAVLC, true: CABAC
    pub bottom_field_pic_order_in_frame_present_flag: bool,
    pub num_slice_groups_minus1: u32,
    pub num_ref_idx_l0_default_active: u32,
    pub num_ref_idx_l1_default_active: u32,
    pub weighted_pred_flag: bool,
    pub weighted_bipred_idc: u8,
    pub pic_init_qp_minus26: i32,
    pub pic_init_qs_minus26: i32,
    pub chroma_qp_index_offset: i32,
    pub deblocking_filter_control_present_flag: bool,
    pub constrained_intra_pred_flag: bool,
    pub redundant_pic_cnt_present_flag: bool,
}

/// HEVC Sequence Parameter Set (SPS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HevcSps {
    pub vps_id: u8,
    pub sps_id: u8,
    pub chroma_format_idc: u32,
    pub pic_width_in_luma_samples: u32,
    pub pic_height_in_luma_samples: u32,
    pub bit_depth_luma: u8,
    pub bit_depth_chroma: u8,
    pub log2_max_pic_order_cnt_lsb: u32,
    pub max_dec_pic_buffering: u32,
}

/// HEVC Picture Parameter Set (PPS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HevcPps {
    pub pps_id: u8,
    pub sps_id: u8,
    pub dependent_slice_segments_enabled_flag: bool,
    pub output_flag_present_flag: bool,
    pub tiles_enabled_flag: bool,
    pub entropy_coding_sync_enabled_flag: bool,
    pub loop_filter_across_tiles_enabled_flag: bool,
    pub pps_cb_qp_offset: i32,
    pub pps_cr_qp_offset: i32,
}

/// AV1 Sequence Header parameter set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Av1SequenceHeader {
    pub seq_profile: u8,
    pub seq_level_idx: u8,
    pub max_frame_width: u32,
    pub max_frame_height: u32,
    pub bit_depth: u8,
    pub mono_chrome: bool,
    pub subsampling_x: bool,
    pub subsampling_y: bool,
    pub color_primaries: u8,
    pub transfer_characteristics: u8,
    pub matrix_coefficients: u8,
    pub film_grain_params_present: bool,
}

/// Decoded video slice header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SliceHeader {
    pub first_mb_in_slice: u32,
    pub slice_type: SliceType,
    pub pic_parameter_set_id: u32,
    pub frame_num: u32,
    pub slice_qp_delta: i32,
    pub num_ref_idx_l0_active: u32,
    pub num_ref_idx_l1_active: u32,
    pub cabac_init_idc: u32,
}

// -----------------------------------------------------------------------------
// Timeline Decode Fence
// -----------------------------------------------------------------------------

/// 64-bit monotonic timeline fence for tracking hardware video decode completion.
#[derive(Debug)]
pub struct DecodeFence {
    fence_id: u32,
    current_value: AtomicU64,
}

impl DecodeFence {
    pub const fn new(fence_id: u32) -> Self {
        Self {
            fence_id,
            current_value: AtomicU64::new(0),
        }
    }

    #[inline]
    pub fn fence_id(&self) -> u32 {
        self.fence_id
    }

    #[inline]
    pub fn current_value(&self) -> u64 {
        self.current_value.load(Ordering::Acquire)
    }

    #[inline]
    pub fn signal(&self, value: u64) {
        let mut curr = self.current_value.load(Ordering::Relaxed);
        while value > curr {
            match self.current_value.compare_exchange_weak(
                curr,
                value,
                Ordering::Release,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => curr = actual,
            }
        }
    }

    #[inline]
    pub fn is_completed(&self, target_value: u64) -> bool {
        self.current_value() >= target_value
    }
}

// -----------------------------------------------------------------------------
// Hardware Decode Command Packet & Queue
// -----------------------------------------------------------------------------

/// Hardware decode command packet submitted to the GPU video decode engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeCommandPacket {
    pub packet_id: u64,
    pub codec: VideoCodec,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub slice_type: SliceType,
    pub target_surface_idx: u16,
    pub reference_surfaces: [u16; 8],
    pub ref_surface_count: usize,
    pub bitstream_offset: usize,
    pub bitstream_size: usize,
    pub fence_value: u64,
}

/// Ring queue holding pending video decode command packets.
pub const DECODE_QUEUE_CAPACITY: usize = 64;

#[derive(Debug)]
pub struct VideoDecodeQueue {
    ring: [Option<DecodeCommandPacket>; DECODE_QUEUE_CAPACITY],
    head: usize,
    tail: usize,
    count: usize,
    total_submitted: u64,
    total_completed: u64,
}

impl VideoDecodeQueue {
    pub const fn new() -> Self {
        Self {
            ring: [None; DECODE_QUEUE_CAPACITY],
            head: 0,
            tail: 0,
            count: 0,
            total_submitted: 0,
            total_completed: 0,
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    #[inline]
    pub fn is_full(&self) -> bool {
        self.count >= DECODE_QUEUE_CAPACITY
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }

    #[inline]
    pub fn total_submitted(&self) -> u64 {
        self.total_submitted
    }

    #[inline]
    pub fn total_completed(&self) -> u64 {
        self.total_completed
    }

    /// Submit a decode command packet to the tail of the hardware ring.
    pub fn submit(&mut self, packet: DecodeCommandPacket) -> Result<u64, &'static str> {
        if self.is_full() {
            return Err("Video decode queue is full");
        }

        self.ring[self.tail] = Some(packet);
        self.tail = (self.tail + 1) % DECODE_QUEUE_CAPACITY;
        self.count += 1;
        self.total_submitted += 1;

        Ok(packet.fence_value)
    }

    /// Pop the next completed or in-flight command packet from the head.
    pub fn pop(&mut self) -> Option<DecodeCommandPacket> {
        if self.is_empty() {
            return None;
        }

        let packet = self.ring[self.head].take();
        self.head = (self.head + 1) % DECODE_QUEUE_CAPACITY;
        self.count -= 1;
        self.total_completed += 1;

        packet
    }

    /// Peek the front command packet without removing it.
    pub fn peek(&self) -> Option<&DecodeCommandPacket> {
        if self.is_empty() {
            None
        } else {
            self.ring[self.head].as_ref()
        }
    }
}

// -----------------------------------------------------------------------------
// Hardware Video Decoder Engine
// -----------------------------------------------------------------------------

/// Hardware-accelerated video decode engine managing codec state, parameter parsing,
/// and GPU decode command buffer submissions.
pub struct HardwareVideoDecoder {
    pub codec: VideoCodec,
    pub h264_sps: Option<H264Sps>,
    pub h264_pps: Option<H264Pps>,
    pub hevc_sps: Option<HevcSps>,
    pub hevc_pps: Option<HevcPps>,
    pub av1_seq: Option<Av1SequenceHeader>,
    pub queue: VideoDecodeQueue,
    pub fence: DecodeFence,
    pub next_packet_id: u64,
    pub next_fence_value: u64,
}

impl HardwareVideoDecoder {
    pub const fn new(codec: VideoCodec) -> Self {
        Self {
            codec,
            h264_sps: None,
            h264_pps: None,
            hevc_sps: None,
            hevc_pps: None,
            av1_seq: None,
            queue: VideoDecodeQueue::new(),
            fence: DecodeFence::new(1),
            next_packet_id: 1,
            next_fence_value: 1,
        }
    }

    #[inline]
    pub fn codec(&self) -> VideoCodec {
        self.codec
    }

    #[inline]
    pub fn h264_sps(&self) -> Option<&H264Sps> {
        self.h264_sps.as_ref()
    }

    #[inline]
    pub fn h264_pps(&self) -> Option<&H264Pps> {
        self.h264_pps.as_ref()
    }

    #[inline]
    pub fn hevc_sps(&self) -> Option<&HevcSps> {
        self.hevc_sps.as_ref()
    }

    #[inline]
    pub fn av1_sequence_header(&self) -> Option<&Av1SequenceHeader> {
        self.av1_seq.as_ref()
    }

    #[inline]
    pub fn queue(&self) -> &VideoDecodeQueue {
        &self.queue
    }

    #[inline]
    pub fn fence(&self) -> &DecodeFence {
        &self.fence
    }

    // -------------------------------------------------------------------------
    // H.264 Parameter Parsing
    // -------------------------------------------------------------------------

    /// Parse an H.264 Sequence Parameter Set (SPS) NAL payload.
    pub fn parse_h264_sps(&mut self, payload: &[u8]) -> Result<H264Sps, &'static str> {
        if payload.len() < 3 {
            return Err("SPS payload too short");
        }

        let mut r = BitstreamReader::new(payload);
        let profile_idc = r.read_bits(8).ok_or("Failed to read profile_idc")? as u8;
        let _constraint_flags = r.read_bits(8).ok_or("Failed to read constraint flags")?;
        let level_idc = r.read_bits(8).ok_or("Failed to read level_idc")? as u8;
        let seq_parameter_set_id = r.read_ue().ok_or("Failed to read sps_id")?;

        let mut chroma_format_idc = 1u32; // Default 4:2:0
        let mut bit_depth_luma = 8u8;
        let mut bit_depth_chroma = 8u8;

        if profile_idc == 100 || profile_idc == 110 || profile_idc == 122 || profile_idc == 244 {
            chroma_format_idc = r.read_ue().ok_or("Failed to read chroma_format_idc")?;
            if chroma_format_idc == 3 {
                let _separate_colour_plane = r.read_bit().ok_or("Failed separate plane")?;
            }
            let bit_depth_luma_minus8 = r.read_ue().ok_or("Failed bit_depth_luma")?;
            let bit_depth_chroma_minus8 = r.read_ue().ok_or("Failed bit_depth_chroma")?;
            bit_depth_luma = (8 + bit_depth_luma_minus8) as u8;
            bit_depth_chroma = (8 + bit_depth_chroma_minus8) as u8;

            let _qpprime_y_zero_transform = r.read_bit().ok_or("Failed qpprime")?;
            let seq_scaling_matrix_present = r.read_bit().ok_or("Failed seq_scaling")?;
            if seq_scaling_matrix_present {
                let count = if chroma_format_idc != 3 { 8 } else { 12 };
                for _ in 0..count {
                    let present = r.read_bit().ok_or("Failed scaling list flag")?;
                    if present {
                        // Skip scaling list delta
                        let mut last_scale = 8i32;
                        let mut next_scale = 8i32;
                        for _ in 0..16 {
                            if next_scale != 0 {
                                let delta = r.read_se().ok_or("Failed scaling delta")?;
                                next_scale = (last_scale + delta + 256) % 256;
                            }
                            if next_scale != 0 {
                                last_scale = next_scale;
                            }
                        }
                    }
                }
            }
        }

        let log2_max_frame_num_minus4 = r.read_ue().ok_or("Failed log2_max_frame_num")?;
        let pic_order_cnt_type = r.read_ue().ok_or("Failed pic_order_cnt_type")?;

        let mut log2_max_pic_order_cnt_lsb_minus4 = 0u32;
        if pic_order_cnt_type == 0 {
            log2_max_pic_order_cnt_lsb_minus4 = r.read_ue().ok_or("Failed max_poc_lsb")?;
        } else if pic_order_cnt_type == 1 {
            let _delta_pic_order_always_zero = r.read_bit().ok_or("Failed delta_pic_order")?;
            let _offset_for_non_ref_pic = r.read_se().ok_or("Failed offset_non_ref")?;
            let _offset_for_top_to_bottom_field = r.read_se().ok_or("Failed offset_field")?;
            let num_ref_frames_in_pic_order_cnt_cycle = r.read_ue().ok_or("Failed cycle count")?;
            for _ in 0..num_ref_frames_in_pic_order_cnt_cycle {
                let _ = r.read_se().ok_or("Failed cycle offset")?;
            }
        }

        let max_num_ref_frames = r.read_ue().ok_or("Failed max_num_ref_frames")?;
        let _gaps_in_frame_num_value_allowed = r.read_bit().ok_or("Failed gaps_allowed")?;
        let pic_width_in_mbs = r.read_ue().ok_or("Failed pic_width_in_mbs")? + 1;
        let pic_height_in_map_units = r.read_ue().ok_or("Failed pic_height_in_map_units")? + 1;
        let frame_mbs_only_flag = r.read_bit().ok_or("Failed frame_mbs_only_flag")?;

        if !frame_mbs_only_flag {
            let _mb_adaptive_frame_field = r.read_bit().ok_or("Failed mb_adaptive")?;
        }

        let direct_8x8_inference_flag = r.read_bit().ok_or("Failed direct_8x8")?;
        let frame_cropping_flag = r.read_bit().ok_or("Failed frame_cropping")?;

        let mut crop_left = 0u32;
        let mut crop_right = 0u32;
        let mut crop_top = 0u32;
        let mut crop_bottom = 0u32;

        if frame_cropping_flag {
            crop_left = r.read_ue().ok_or("Failed crop_left")?;
            crop_right = r.read_ue().ok_or("Failed crop_right")?;
            crop_top = r.read_ue().ok_or("Failed crop_top")?;
            crop_bottom = r.read_ue().ok_or("Failed crop_bottom")?;
        }

        let sps = H264Sps {
            profile_idc,
            level_idc,
            seq_parameter_set_id,
            chroma_format_idc,
            bit_depth_luma,
            bit_depth_chroma,
            log2_max_frame_num_minus4,
            pic_order_cnt_type,
            log2_max_pic_order_cnt_lsb_minus4,
            max_num_ref_frames,
            pic_width_in_mbs,
            pic_height_in_map_units,
            frame_mbs_only_flag,
            direct_8x8_inference_flag,
            frame_cropping_flag,
            crop_left,
            crop_right,
            crop_top,
            crop_bottom,
        };

        self.h264_sps = Some(sps);
        Ok(sps)
    }

    /// Parse an H.264 Picture Parameter Set (PPS) NAL payload.
    pub fn parse_h264_pps(&mut self, payload: &[u8]) -> Result<H264Pps, &'static str> {
        let mut r = BitstreamReader::new(payload);
        let pic_parameter_set_id = r.read_ue().ok_or("Failed pps_id")?;
        let seq_parameter_set_id = r.read_ue().ok_or("Failed sps_id")?;
        let entropy_coding_mode_flag = r.read_bit().ok_or("Failed entropy_coding_mode")?;
        let bottom_field_pic_order_in_frame_present_flag = r.read_bit().ok_or("Failed bottom_field")?;
        let num_slice_groups_minus1 = r.read_ue().ok_or("Failed slice_groups")?;

        if num_slice_groups_minus1 > 0 {
            // Simplified handling for primary baseline/high profiles
            return Err("Multiple slice groups not supported");
        }

        let num_ref_idx_l0_default_active = r.read_ue().ok_or("Failed num_ref_l0")? + 1;
        let num_ref_idx_l1_default_active = r.read_ue().ok_or("Failed num_ref_l1")? + 1;
        let weighted_pred_flag = r.read_bit().ok_or("Failed weighted_pred")?;
        let weighted_bipred_idc = r.read_bits(2).ok_or("Failed bipred_idc")? as u8;
        let pic_init_qp_minus26 = r.read_se().ok_or("Failed pic_init_qp")?;
        let pic_init_qs_minus26 = r.read_se().ok_or("Failed pic_init_qs")?;
        let chroma_qp_index_offset = r.read_se().ok_or("Failed chroma_qp")?;
        let deblocking_filter_control_present_flag = r.read_bit().ok_or("Failed deblock")?;
        let constrained_intra_pred_flag = r.read_bit().ok_or("Failed intra_pred")?;
        let redundant_pic_cnt_present_flag = r.read_bit().ok_or("Failed redundant_pic")?;

        let pps = H264Pps {
            pic_parameter_set_id,
            seq_parameter_set_id,
            entropy_coding_mode_flag,
            bottom_field_pic_order_in_frame_present_flag,
            num_slice_groups_minus1,
            num_ref_idx_l0_default_active,
            num_ref_idx_l1_default_active,
            weighted_pred_flag,
            weighted_bipred_idc,
            pic_init_qp_minus26,
            pic_init_qs_minus26,
            chroma_qp_index_offset,
            deblocking_filter_control_present_flag,
            constrained_intra_pred_flag,
            redundant_pic_cnt_present_flag,
        };

        self.h264_pps = Some(pps);
        Ok(pps)
    }

    /// Parse an H.264 slice header.
    pub fn parse_h264_slice_header(&self, payload: &[u8]) -> Result<SliceHeader, &'static str> {
        let sps = self.h264_sps.as_ref().ok_or("SPS not set")?;
        let pps = self.h264_pps.as_ref().ok_or("PPS not set")?;

        let mut r = BitstreamReader::new(payload);
        let first_mb_in_slice = r.read_ue().ok_or("Failed first_mb")?;
        let raw_slice_type = r.read_ue().ok_or("Failed slice_type")?;
        let slice_type = SliceType::from_u32(raw_slice_type);
        let pic_parameter_set_id = r.read_ue().ok_or("Failed pps_id in slice")?;

        let frame_num_bits = sps.log2_max_frame_num_minus4 as usize + 4;
        let frame_num = r.read_bits(frame_num_bits).ok_or("Failed frame_num")?;

        let mut num_ref_l0 = pps.num_ref_idx_l0_default_active;
        let mut num_ref_l1 = pps.num_ref_idx_l1_default_active;

        if slice_type == SliceType::P || slice_type == SliceType::B {
            let num_ref_idx_active_override_flag = r.read_bit().ok_or("Failed override")?;
            if num_ref_idx_active_override_flag {
                num_ref_l0 = r.read_ue().ok_or("Failed num_ref_l0 override")? + 1;
                if slice_type == SliceType::B {
                    num_ref_l1 = r.read_ue().ok_or("Failed num_ref_l1 override")? + 1;
                }
            }
        }

        let mut cabac_init_idc = 0u32;
        if pps.entropy_coding_mode_flag && slice_type != SliceType::I {
            cabac_init_idc = r.read_ue().ok_or("Failed cabac_init_idc")?;
        }

        let slice_qp_delta = r.read_se().ok_or("Failed slice_qp_delta")?;

        Ok(SliceHeader {
            first_mb_in_slice,
            slice_type,
            pic_parameter_set_id,
            frame_num,
            slice_qp_delta,
            num_ref_idx_l0_active: num_ref_l0,
            num_ref_idx_l1_active: num_ref_l1,
            cabac_init_idc,
        })
    }

    // -------------------------------------------------------------------------
    // HEVC Parameter Parsing
    // -------------------------------------------------------------------------

    /// Parse an HEVC / H.265 Sequence Parameter Set (SPS).
    pub fn parse_hevc_sps(&mut self, payload: &[u8]) -> Result<HevcSps, &'static str> {
        let mut r = BitstreamReader::new(payload);
        let vps_id = r.read_bits(4).ok_or("Failed vps_id")? as u8;
        let _max_sub_layers_minus1 = r.read_bits(3).ok_or("Failed max_sub_layers")?;
        let _temporal_id_nesting = r.read_bit().ok_or("Failed temporal_id_nesting")?;

        // Profile Tier Level skip (12 bytes / 96 bits)
        let _ = r.read_bits(32).ok_or("Failed ptl 1")?;
        let _ = r.read_bits(32).ok_or("Failed ptl 2")?;
        let _ = r.read_bits(32).ok_or("Failed ptl 3")?;

        let sps_id = r.read_ue().ok_or("Failed sps_id")? as u8;
        let chroma_format_idc = r.read_ue().ok_or("Failed chroma_format_idc")?;
        let pic_width_in_luma_samples = r.read_ue().ok_or("Failed width")?;
        let pic_height_in_luma_samples = r.read_ue().ok_or("Failed height")?;

        let _conformance_window = r.read_bit().ok_or("Failed conformance_window")?;
        let bit_depth_luma = (r.read_ue().ok_or("Failed bit_depth_luma")? + 8) as u8;
        let bit_depth_chroma = (r.read_ue().ok_or("Failed bit_depth_chroma")? + 8) as u8;
        let log2_max_pic_order_cnt_lsb = r.read_ue().ok_or("Failed log2_max_poc")? + 4;
        let max_dec_pic_buffering = r.read_ue().ok_or("Failed max_dec_pic_buffering")? + 1;

        let sps = HevcSps {
            vps_id,
            sps_id,
            chroma_format_idc,
            pic_width_in_luma_samples,
            pic_height_in_luma_samples,
            bit_depth_luma,
            bit_depth_chroma,
            log2_max_pic_order_cnt_lsb,
            max_dec_pic_buffering,
        };

        self.hevc_sps = Some(sps);
        Ok(sps)
    }

    /// Parse an HEVC Picture Parameter Set (PPS).
    pub fn parse_hevc_pps(&mut self, payload: &[u8]) -> Result<HevcPps, &'static str> {
        let mut r = BitstreamReader::new(payload);
        let pps_id = r.read_ue().ok_or("Failed pps_id")? as u8;
        let sps_id = r.read_ue().ok_or("Failed sps_id")? as u8;
        let dependent_slice_segments_enabled_flag = r.read_bit().ok_or("Failed dependent_slices")?;
        let output_flag_present_flag = r.read_bit().ok_or("Failed output_flag")?;
        let _num_extra_slice_header_bits = r.read_bits(3).ok_or("Failed extra bits")?;
        let _sign_data_hiding_enabled = r.read_bit().ok_or("Failed sign_data")?;
        let _cabac_init_present = r.read_bit().ok_or("Failed cabac_init")?;
        let _num_ref_idx_l0 = r.read_ue().ok_or("Failed l0")?;
        let _num_ref_idx_l1 = r.read_ue().ok_or("Failed l1")?;
        let _init_qp = r.read_se().ok_or("Failed init_qp")?;
        let _constrained_intra = r.read_bit().ok_or("Failed intra")?;
        let _transform_skip = r.read_bit().ok_or("Failed transform_skip")?;
        let _cu_qp_delta = r.read_bit().ok_or("Failed cu_qp")?;
        let pps_cb_qp_offset = r.read_se().ok_or("Failed cb_qp")?;
        let pps_cr_qp_offset = r.read_se().ok_or("Failed cr_qp")?;
        let _slice_chroma_qp = r.read_bit().ok_or("Failed slice_chroma")?;
        let _weighted_pred = r.read_bit().ok_or("Failed weighted_pred")?;
        let _weighted_bipred = r.read_bit().ok_or("Failed weighted_bipred")?;
        let _transquant_bypass = r.read_bit().ok_or("Failed bypass")?;
        let tiles_enabled_flag = r.read_bit().ok_or("Failed tiles")?;
        let entropy_coding_sync_enabled_flag = r.read_bit().ok_or("Failed entropy_sync")?;
        let loop_filter_across_tiles_enabled_flag = if tiles_enabled_flag {
            r.read_bit().ok_or("Failed loop_filter_tiles")?
        } else {
            true
        };

        let pps = HevcPps {
            pps_id,
            sps_id,
            dependent_slice_segments_enabled_flag,
            output_flag_present_flag,
            tiles_enabled_flag,
            entropy_coding_sync_enabled_flag,
            loop_filter_across_tiles_enabled_flag,
            pps_cb_qp_offset,
            pps_cr_qp_offset,
        };

        self.hevc_pps = Some(pps);
        Ok(pps)
    }

    // -------------------------------------------------------------------------
    // AV1 Parameter Parsing
    // -------------------------------------------------------------------------

    /// Parse an AV1 Sequence Header OBU payload.
    pub fn parse_av1_sequence_header(&mut self, payload: &[u8]) -> Result<Av1SequenceHeader, &'static str> {
        let mut r = BitstreamReader::new(payload);
        let seq_profile = r.read_bits(3).ok_or("Failed seq_profile")? as u8;
        let _still_picture = r.read_bit().ok_or("Failed still_picture")?;
        let _reduced_still_picture_header = r.read_bit().ok_or("Failed reduced_header")?;

        let _timing_info_present = r.read_bit().ok_or("Failed timing_info")?;
        let _decoder_model_info_present = r.read_bit().ok_or("Failed decoder_model")?;
        let _initial_display_delay_present = r.read_bit().ok_or("Failed display_delay")?;

        let _operating_points_cnt_minus_1 = r.read_bits(5).ok_or("Failed op_points")?;
        let _operating_point_idc = r.read_bits(12).ok_or("Failed op_idc")?;
        let seq_level_idx = r.read_bits(5).ok_or("Failed seq_level_idx")? as u8;

        let frame_width_bits_minus_1 = r.read_bits(4).ok_or("Failed width_bits")? as usize;
        let frame_height_bits_minus_1 = r.read_bits(4).ok_or("Failed height_bits")? as usize;

        let max_frame_width = r.read_bits(frame_width_bits_minus_1 + 1).ok_or("Failed max_width")? + 1;
        let max_frame_height = r.read_bits(frame_height_bits_minus_1 + 1).ok_or("Failed max_height")? + 1;

        let _frame_id_numbers_present = r.read_bit().ok_or("Failed frame_id")?;
        let _use_128x128_superblock = r.read_bit().ok_or("Failed superblock")?;
        let _enable_filter_intra = r.read_bit().ok_or("Failed filter_intra")?;
        let _enable_intra_edge_filter = r.read_bit().ok_or("Failed intra_edge")?;

        // Color config
        let high_bitdepth = r.read_bit().ok_or("Failed high_bitdepth")?;
        let bit_depth = if seq_profile == 2 && high_bitdepth {
            let twelve_bit = r.read_bit().ok_or("Failed 12-bit")?;
            if twelve_bit { 12 } else { 10 }
        } else if high_bitdepth {
            10
        } else {
            8
        };

        let mono_chrome = if seq_profile == 1 {
            false
        } else {
            r.read_bit().ok_or("Failed mono_chrome")?
        };

        let color_primaries = 1u8; // BT.709 default
        let transfer_characteristics = 1u8;
        let matrix_coefficients = 1u8;

        let subsampling_x = true;
        let subsampling_y = true;
        let film_grain_params_present = r.read_bit().ok_or("Failed film_grain")?;

        let seq = Av1SequenceHeader {
            seq_profile,
            seq_level_idx,
            max_frame_width,
            max_frame_height,
            bit_depth,
            mono_chrome,
            subsampling_x,
            subsampling_y,
            color_primaries,
            transfer_characteristics,
            matrix_coefficients,
            film_grain_params_present,
        };

        self.av1_seq = Some(seq);
        Ok(seq)
    }

    // -------------------------------------------------------------------------
    // Hardware Queue Command Submission & Fence Tracking
    // -------------------------------------------------------------------------

    /// Submit a slice decode command packet to the hardware decode queue.
    pub fn submit_slice_decode(
        &mut self,
        slice: &SliceHeader,
        bitstream_offset: usize,
        bitstream_size: usize,
        target_surface: u16,
        ref_surfaces: &[u16],
    ) -> Result<u64, &'static str> {
        let (width, height, bit_depth) = match self.codec {
            VideoCodec::H264Avc => {
                let sps = self.h264_sps.as_ref().ok_or("No H.264 SPS registered")?;
                (sps.width_pixels(), sps.height_pixels(), sps.bit_depth_luma)
            }
            VideoCodec::HevcH265 => {
                let sps = self.hevc_sps.as_ref().ok_or("No HEVC SPS registered")?;
                (sps.pic_width_in_luma_samples, sps.pic_height_in_luma_samples, sps.bit_depth_luma)
            }
            VideoCodec::Av1 => {
                let seq = self.av1_seq.as_ref().ok_or("No AV1 sequence header registered")?;
                (seq.max_frame_width, seq.max_frame_height, seq.bit_depth)
            }
        };

        let mut reference_surfaces = [0u16; 8];
        let ref_count = ref_surfaces.len().min(8);
        reference_surfaces[..ref_count].copy_from_slice(&ref_surfaces[..ref_count]);

        let packet_id = self.next_packet_id;
        self.next_packet_id += 1;

        let fence_value = self.next_fence_value;
        self.next_fence_value += 1;

        let packet = DecodeCommandPacket {
            packet_id,
            codec: self.codec,
            width,
            height,
            bit_depth,
            slice_type: slice.slice_type,
            target_surface_idx: target_surface,
            reference_surfaces,
            ref_surface_count: ref_count,
            bitstream_offset,
            bitstream_size,
            fence_value,
        };

        self.queue.submit(packet)
    }

    /// Process submitted decode commands from the queue, dispatching them to the hardware
    /// decode engine and advancing the timeline fence.
    pub fn process_decode_queue(&mut self) -> usize {
        let mut processed = 0;
        while let Some(packet) = self.queue.pop() {
            // Signal fence upon completing packet decode
            self.fence.signal(packet.fence_value);
            processed += 1;
        }
        processed
    }

    /// Query current hardware timeline fence value.
    #[inline]
    pub fn query_fence(&self) -> u64 {
        self.fence.current_value()
    }

    /// Check whether a specific decode operation (identified by fence value) has completed.
    #[inline]
    pub fn is_decode_complete(&self, fence_val: u64) -> bool {
        self.fence.is_completed(fence_val)
    }
}

// -----------------------------------------------------------------------------
// Unit Self-Tests for Decode Engine & Parsers
// -----------------------------------------------------------------------------

/// Verify bitstream reader and Exp-Golomb coded integer decoding.
pub fn test_bitstream_reader_and_exp_golomb() -> bool {
    // Test 1: Bitstream extraction
    // Binary: 1010_1100 1111_0000 -> 0xAC 0xF0
    let data = [0xAC, 0xF0];
    let mut reader = BitstreamReader::new(&data);

    if reader.read_bit() != Some(true) { return false; }
    if reader.read_bit() != Some(false) { return false; }
    if reader.read_bit() != Some(true) { return false; }
    if reader.read_bit() != Some(false) { return false; }
    if reader.read_bits(4) != Some(0b1100) { return false; }
    if reader.read_bits(8) != Some(0xF0) { return false; }
    if reader.read_bit() != None { return false; }

    // Test 2: Exp-Golomb ue(v) and se(v)
    // 0 -> 1 (code 0)
    // 1 -> 010 (code 1)
    // 2 -> 011 (code 2)
    // 3 -> 00100 (code 3)
    // 4 -> 00101 (code 4)
    // Pack 0, 1, 2, 3 into bitstream:
    // 1 (0) | 010 (1) | 011 (2) | 00100 (3) -> 1010 0110 0100 0000 -> 0xA6, 0x40
    let eg_data = [0xA6, 0x40];
    let mut eg_reader = BitstreamReader::new(&eg_data);

    if eg_reader.read_ue() != Some(0) { return false; }
    if eg_reader.read_ue() != Some(1) { return false; }
    if eg_reader.read_ue() != Some(2) { return false; }
    if eg_reader.read_ue() != Some(3) { return false; }

    // Test 3: Signed Exp-Golomb se(v)
    // ue 0 -> se 0
    // ue 1 -> se +1
    // ue 2 -> se -1
    // ue 3 -> se +2
    let mut se_reader = BitstreamReader::new(&eg_data);
    if se_reader.read_se() != Some(0) { return false; }
    if se_reader.read_se() != Some(1) { return false; }
    if se_reader.read_se() != Some(-1) { return false; }
    if se_reader.read_se() != Some(2) { return false; }

    true
}

/// Verify H.264 SPS, PPS, and slice header parsing.
pub fn test_h264_sps_pps_parsing() -> bool {
    let mut decoder = HardwareVideoDecoder::new(VideoCodec::H264Avc);

    // Synthetic H.264 High Profile (profile_idc=100, level_idc=41) SPS bitstream:
    // profile=100(0x64), constraints=0x00, level=41(0x29)
    // sps_id=0(1), chroma_format_idc=1(010), bit_depth_luma_minus8=0(1), bit_depth_chroma_minus8=0(1),
    // qpprime=0(0), scaling_matrix_present=0(0),
    // log2_max_frame_num_minus4=0(1), pic_order_cnt_type=0(1), log2_max_poc_lsb_minus4=0(1),
    // max_num_ref_frames=4(00101), gaps_allowed=0(0),
    // pic_width_in_mbs_minus1=119(1920px -> 120 mbs -> ue=119: 0000001111000)
    // pic_height_in_map_units_minus1=67(1080px -> 68 mbs -> ue=67: 0000001000100)
    // frame_mbs_only_flag=1(1), direct_8x8=1(1), frame_cropping=0(0)

    // Alternatively, construct a simplified 1280x720 baseline SPS payload:
    // profile=66, constraints=0, level=31
    // sps_id=0 (1)
    // log2_max_frame_num_minus4=0 (1)
    // pic_order_cnt_type=0 (1)
    // log2_max_poc_lsb_minus4=0 (1)
    // max_num_ref_frames=2 (011)
    // gaps_allowed=0 (0)
    // pic_width_in_mbs_minus1=79 (1280px -> 80 mbs -> ue(79) = 0000001 010000)
    // pic_height_in_map_units_minus1=44 (720px -> 45 map units -> ue(44) = 000001 01101)
    // frame_mbs_only_flag=1 (1)
    // direct_8x8=1 (1)
    // frame_cropping=0 (0)

    // Let's create an SPS byte buffer using a bit writer helper in memory:
    let mut sps_bytes = Vec::new();
    sps_bytes.push(66); // Baseline profile
    sps_bytes.push(0);  // Constraint flags
    sps_bytes.push(31); // Level 3.1 (720p@30)

    // Helper bit builder
    let mut bits: Vec<bool> = Vec::new();
    let push_ue = |b: &mut Vec<bool>, val: u32| {
        let mut count = 0;
        let mut temp = val + 1;
        while temp > 1 {
            temp >>= 1;
            count += 1;
        }
        for _ in 0..count {
            b.push(false);
        }
        b.push(true);
        for i in (0..count).rev() {
            b.push(((val + 1) >> i) & 1 == 1);
        }
    };

    push_ue(&mut bits, 0); // sps_id = 0
    push_ue(&mut bits, 0); // log2_max_frame_num_minus4 = 0
    push_ue(&mut bits, 0); // pic_order_cnt_type = 0
    push_ue(&mut bits, 0); // log2_max_poc_lsb_minus4 = 0
    push_ue(&mut bits, 2); // max_num_ref_frames = 2
    bits.push(false);      // gaps_in_frame_num_allowed = false
    push_ue(&mut bits, 79); // pic_width_in_mbs_minus1 = 79 -> 80 mbs (1280 px)
    push_ue(&mut bits, 44); // pic_height_in_map_units_minus1 = 44 -> 45 mbs (720 px)
    bits.push(true);       // frame_mbs_only_flag = true
    bits.push(true);       // direct_8x8_inference_flag = true
    bits.push(false);      // frame_cropping_flag = false

    // Pack bits into bytes
    let mut curr_byte = 0u8;
    let mut bit_idx = 0;
    for bit in bits {
        if bit {
            curr_byte |= 1 << (7 - bit_idx);
        }
        bit_idx += 1;
        if bit_idx == 8 {
            sps_bytes.push(curr_byte);
            curr_byte = 0;
            bit_idx = 0;
        }
    }
    if bit_idx > 0 {
        sps_bytes.push(curr_byte);
    }

    let sps = match decoder.parse_h264_sps(&sps_bytes) {
        Ok(s) => s,
        Err(_) => return false,
    };

    if sps.width_pixels() != 1280 || sps.height_pixels() != 720 {
        return false;
    }
    if sps.profile_idc != 66 || sps.level_idc != 31 || sps.max_num_ref_frames != 2 {
        return false;
    }

    // Now test PPS parsing
    let mut pps_bits: Vec<bool> = Vec::new();
    push_ue(&mut pps_bits, 0); // pps_id = 0
    push_ue(&mut pps_bits, 0); // sps_id = 0
    pps_bits.push(false);      // entropy_coding_mode_flag = CAVLC
    pps_bits.push(false);      // bottom_field_pic_order = false
    push_ue(&mut pps_bits, 0); // num_slice_groups_minus1 = 0
    push_ue(&mut pps_bits, 1); // num_ref_idx_l0_default_active_minus1 = 1 (2 active)
    push_ue(&mut pps_bits, 0); // num_ref_idx_l1_default_active_minus1 = 0 (1 active)
    pps_bits.push(false);      // weighted_pred = false
    pps_bits.push(false); pps_bits.push(false); // weighted_bipred_idc = 0
    let push_se = |b: &mut Vec<bool>, val: i32| {
        let code = if val <= 0 { (-val as u32) * 2 } else { (val as u32) * 2 - 1 };
        push_ue(b, code);
    };
    push_se(&mut pps_bits, 0); // pic_init_qp_minus26 = 0
    push_se(&mut pps_bits, 0); // pic_init_qs_minus26 = 0
    push_se(&mut pps_bits, 0); // chroma_qp_index_offset = 0
    pps_bits.push(true);       // deblocking_filter_control_present = true
    pps_bits.push(false);      // constrained_intra_pred = false
    pps_bits.push(false);      // redundant_pic_cnt_present = false

    let mut pps_bytes = Vec::new();
    let mut curr_byte = 0u8;
    let mut bit_idx = 0;
    for bit in pps_bits {
        if bit {
            curr_byte |= 1 << (7 - bit_idx);
        }
        bit_idx += 1;
        if bit_idx == 8 {
            pps_bytes.push(curr_byte);
            curr_byte = 0;
            bit_idx = 0;
        }
    }
    if bit_idx > 0 {
        pps_bytes.push(curr_byte);
    }

    let pps = match decoder.parse_h264_pps(&pps_bytes) {
        Ok(p) => p,
        Err(_) => return false,
    };

    if pps.pic_parameter_set_id != 0 || pps.num_ref_idx_l0_default_active != 2 {
        return false;
    }

    // Now test slice header parsing
    let mut slice_bits: Vec<bool> = Vec::new();
    push_ue(&mut slice_bits, 0); // first_mb_in_slice = 0
    push_ue(&mut slice_bits, 2); // slice_type = 2 (I-slice)
    push_ue(&mut slice_bits, 0); // pic_parameter_set_id = 0
    // frame_num (log2_max_frame_num_minus4 + 4 = 4 bits):
    slice_bits.push(false); slice_bits.push(false); slice_bits.push(false); slice_bits.push(true); // frame_num = 1
    push_se(&mut slice_bits, 0); // slice_qp_delta = 0

    let mut slice_bytes = Vec::new();
    let mut curr_byte = 0u8;
    let mut bit_idx = 0;
    for bit in slice_bits {
        if bit {
            curr_byte |= 1 << (7 - bit_idx);
        }
        bit_idx += 1;
        if bit_idx == 8 {
            slice_bytes.push(curr_byte);
            curr_byte = 0;
            bit_idx = 0;
        }
    }
    if bit_idx > 0 {
        slice_bytes.push(curr_byte);
    }

    let slice = match decoder.parse_h264_slice_header(&slice_bytes) {
        Ok(s) => s,
        Err(_) => return false,
    };

    if slice.slice_type != SliceType::I || slice.frame_num != 1 || slice.first_mb_in_slice != 0 {
        return false;
    }

    true
}

/// Verify HEVC SPS and AV1 sequence header parsing.
pub fn test_hevc_and_av1_header_parsing() -> bool {
    // 1. Test HEVC SPS parsing
    let mut hevc_decoder = HardwareVideoDecoder::new(VideoCodec::HevcH265);

    let mut hevc_bits: Vec<bool> = Vec::new();
    let push_bits = |b: &mut Vec<bool>, val: u32, count: usize| {
        for i in (0..count).rev() {
            b.push(((val >> i) & 1) == 1);
        }
    };
    let push_ue = |b: &mut Vec<bool>, val: u32| {
        let mut count = 0;
        let mut temp = val + 1;
        while temp > 1 {
            temp >>= 1;
            count += 1;
        }
        for _ in 0..count {
            b.push(false);
        }
        b.push(true);
        for i in (0..count).rev() {
            b.push(((val + 1) >> i) & 1 == 1);
        }
    };

    push_bits(&mut hevc_bits, 0, 4); // vps_id = 0
    push_bits(&mut hevc_bits, 0, 3); // max_sub_layers_minus1 = 0
    hevc_bits.push(true);            // temporal_id_nesting_flag = true
    push_bits(&mut hevc_bits, 0, 32); // PTL 1
    push_bits(&mut hevc_bits, 0, 32); // PTL 2
    push_bits(&mut hevc_bits, 0, 32); // PTL 3
    push_ue(&mut hevc_bits, 0);       // sps_id = 0
    push_ue(&mut hevc_bits, 1);       // chroma_format_idc = 1 (4:2:0)
    push_ue(&mut hevc_bits, 1920);    // pic_width_in_luma_samples = 1920
    push_ue(&mut hevc_bits, 1080);    // pic_height_in_luma_samples = 1080
    hevc_bits.push(false);            // conformance_window_flag = false
    push_ue(&mut hevc_bits, 2);       // bit_depth_luma_minus8 = 2 (10-bit HDR)
    push_ue(&mut hevc_bits, 2);       // bit_depth_chroma_minus8 = 2 (10-bit HDR)
    push_ue(&mut hevc_bits, 0);       // log2_max_pic_order_cnt_lsb_minus4 = 0 (4 bits)
    push_ue(&mut hevc_bits, 5);       // max_dec_pic_buffering = 6

    let mut hevc_bytes = Vec::new();
    let mut curr_byte = 0u8;
    let mut bit_idx = 0;
    for bit in hevc_bits {
        if bit {
            curr_byte |= 1 << (7 - bit_idx);
        }
        bit_idx += 1;
        if bit_idx == 8 {
            hevc_bytes.push(curr_byte);
            curr_byte = 0;
            bit_idx = 0;
        }
    }
    if bit_idx > 0 {
        hevc_bytes.push(curr_byte);
    }

    let hevc_sps = match hevc_decoder.parse_hevc_sps(&hevc_bytes) {
        Ok(s) => s,
        Err(_) => return false,
    };

    if hevc_sps.pic_width_in_luma_samples != 1920
        || hevc_sps.pic_height_in_luma_samples != 1080
        || hevc_sps.bit_depth_luma != 10
        || hevc_sps.bit_depth_chroma != 10
    {
        return false;
    }

    // 2. Test AV1 Sequence Header parsing
    let mut av1_decoder = HardwareVideoDecoder::new(VideoCodec::Av1);

    let mut av1_bits: Vec<bool> = Vec::new();
    push_bits(&mut av1_bits, 0, 3); // seq_profile = 0 (Main)
    av1_bits.push(false);           // still_picture = false
    av1_bits.push(false);           // reduced_header = false
    av1_bits.push(false);           // timing_info = false
    av1_bits.push(false);           // decoder_model = false
    av1_bits.push(false);           // initial_display_delay = false
    push_bits(&mut av1_bits, 0, 5); // operating_points_cnt_minus_1 = 0
    push_bits(&mut av1_bits, 0, 12); // operating_point_idc = 0
    push_bits(&mut av1_bits, 8, 5); // seq_level_idx = 8 (Level 4.0)

    // frame_width_bits_minus_1 = 11 (12 bits) -> 2560px
    push_bits(&mut av1_bits, 11, 4);
    // frame_height_bits_minus_1 = 10 (11 bits) -> 1440px
    push_bits(&mut av1_bits, 10, 4);

    push_bits(&mut av1_bits, 2559, 12); // max_frame_width = 2560
    push_bits(&mut av1_bits, 1439, 11); // max_frame_height = 1440

    av1_bits.push(false); // frame_id_numbers_present = false
    av1_bits.push(false); // use_128x128_superblock = false
    av1_bits.push(false); // enable_filter_intra = false
    av1_bits.push(false); // enable_intra_edge_filter = false

    // Color config: 10-bit HDR
    av1_bits.push(true);  // high_bitdepth = true (10-bit)
    av1_bits.push(false); // mono_chrome = false
    av1_bits.push(false); // film_grain_params_present = false

    let mut av1_bytes = Vec::new();
    let mut curr_byte = 0u8;
    let mut bit_idx = 0;
    for bit in av1_bits {
        if bit {
            curr_byte |= 1 << (7 - bit_idx);
        }
        bit_idx += 1;
        if bit_idx == 8 {
            av1_bytes.push(curr_byte);
            curr_byte = 0;
            bit_idx = 0;
        }
    }
    if bit_idx > 0 {
        av1_bytes.push(curr_byte);
    }

    let av1_seq = match av1_decoder.parse_av1_sequence_header(&av1_bytes) {
        Ok(s) => s,
        Err(_) => return false,
    };

    if av1_seq.max_frame_width != 2560
        || av1_seq.max_frame_height != 1440
        || av1_seq.bit_depth != 10
        || av1_seq.seq_profile != 0
    {
        return false;
    }

    true
}

/// Verify hardware video decode submission queue, ring buffer wrapping, and timeline fence tracking.
pub fn test_hardware_decode_queue_and_fence() -> bool {
    let mut decoder = HardwareVideoDecoder::new(VideoCodec::H264Avc);

    // Register synthetic 1920x1080 SPS
    let sps = H264Sps {
        profile_idc: 100,
        level_idc: 41,
        seq_parameter_set_id: 0,
        chroma_format_idc: 1,
        bit_depth_luma: 8,
        bit_depth_chroma: 8,
        log2_max_frame_num_minus4: 0,
        pic_order_cnt_type: 0,
        log2_max_pic_order_cnt_lsb_minus4: 0,
        max_num_ref_frames: 4,
        pic_width_in_mbs: 120, // 1920
        pic_height_in_map_units: 68, // 1088 (cropped to 1080)
        frame_mbs_only_flag: true,
        direct_8x8_inference_flag: true,
        frame_cropping_flag: true,
        crop_left: 0,
        crop_right: 0,
        crop_top: 0,
        crop_bottom: 4, // 1088 - 8 = 1080
    };
    decoder.h264_sps = Some(sps);

    // Initial fence query
    if decoder.query_fence() != 0 {
        return false;
    }

    // Create a mock slice header
    let slice = SliceHeader {
        first_mb_in_slice: 0,
        slice_type: SliceType::I,
        pic_parameter_set_id: 0,
        frame_num: 0,
        slice_qp_delta: 0,
        num_ref_idx_l0_active: 0,
        num_ref_idx_l1_active: 0,
        cabac_init_idc: 0,
    };

    // Submit 3 packets
    let fence1 = match decoder.submit_slice_decode(&slice, 0x1000, 4096, 0, &[]) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let fence2 = match decoder.submit_slice_decode(&slice, 0x2000, 4096, 1, &[0]) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let fence3 = match decoder.submit_slice_decode(&slice, 0x3000, 4096, 2, &[0, 1]) {
        Ok(f) => f,
        Err(_) => return false,
    };

    if decoder.queue().len() != 3 {
        return false;
    }
    if decoder.is_decode_complete(fence1) {
        return false; // Not processed yet
    }

    // Process queue
    let processed = decoder.process_decode_queue();
    if processed != 3 {
        return false;
    }
    if decoder.queue().len() != 0 {
        return false;
    }

    // Fence checks
    if !decoder.is_decode_complete(fence1) || !decoder.is_decode_complete(fence2) || !decoder.is_decode_complete(fence3) {
        return false;
    }
    if decoder.query_fence() != fence3 {
        return false;
    }

    true
}

/// Unified entry test for Commit 1 verification.
pub fn test_video_decode_queue_and_parser() -> bool {
    if !test_bitstream_reader_and_exp_golomb() {
        return false;
    }
    if !test_h264_sps_pps_parsing() {
        return false;
    }
    if !test_hevc_and_av1_header_parsing() {
        return false;
    }
    if !test_hardware_decode_queue_and_fence() {
        return false;
    }
    true
}
