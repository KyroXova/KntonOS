//! Kernel-Direct Hardware Video Decode & Presentation Streamer Subsystem.
//!
//! Provides hardware-accelerated video decoding (H.264 / HEVC / AV1),
//! tiled NV12/P010 decoded surface management, reference frame buffering,
//! and zero-copy direct scanout presentation pipelines.

#![allow(dead_code)]

pub mod decode;
pub mod nv12;

/// Comprehensive verification self-test suite for Kernel-Direct Hardware Video Decode
/// & Zero-Copy Presentation Streamer subsystem.
///
/// Emits sentinel [KNT-VIDEO-DECODE-PASS] upon successful validation.
pub fn test_video_decode_subsystem() -> bool {
    crate::serial_write_str("[KNT-VIDEO-DECODE-TEST] Running hardware video decode & zero-copy presentation verification suite...\r\n");

    // 1. Verify Exp-Golomb bitstream extraction, SPS/PPS/AV1 sequence header parsing, and slice headers
    if !decode::test_bitstream_reader_and_exp_golomb() {
        crate::serial_write_str("[KNT-PANIC] Bitstream reader and Exp-Golomb decoder verification failed!\r\n");
        return false;
    }
    if !decode::test_h264_sps_pps_parsing() {
        crate::serial_write_str("[KNT-PANIC] H.264 SPS, PPS, and slice header parser verification failed!\r\n");
        return false;
    }
    if !decode::test_hevc_and_av1_header_parsing() {
        crate::serial_write_str("[KNT-PANIC] HEVC SPS and AV1 sequence header parser verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * Bitstream reader (ue/se) and multi-codec picture parameter sets (H.264/HEVC/AV1) verified\r\n");

    // 2. Verify hardware video decode submission queue, ring wrapping, and 64-bit timeline completion fence
    if !decode::test_hardware_decode_queue_and_fence() {
        crate::serial_write_str("[KNT-PANIC] Hardware video decode queue and timeline fence verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * Hardware video decode ring queue submission and 64-bit timeline fence tracking verified\r\n");

    // 3. Verify NV12 and P010 bi-planar tiled frame buffer allocator and DPB reference frame ring
    if !nv12::test_nv12_p010_allocator_and_dpb() {
        crate::serial_write_str("[KNT-PANIC] NV12/P010 surface allocator and DPB ring verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * NV12 (8-bit) & P010 (10-bit HDR) pitch alignment, tiled memory layout, and DPB LRU ring verified\r\n");

    // 4. Verify fixed-point color matrices (BT.709 & BT.2020) and zero-copy direct scanout presentation pipeline
    if !nv12::test_zero_copy_presentation_and_color_matrices() {
        crate::serial_write_str("[KNT-PANIC] Color matrix conversion and zero-copy presentation verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * Fixed-point BT.709 (SDR) / BT.2020 (HDR10) matrices and zero-copy VSYNC direct scanout flip verified\r\n");

    // 5. End-to-end integration test: decode slice -> DPB surface -> zero-copy presentation handoff
    let mut decoder = decode::HardwareVideoDecoder::new(decode::VideoCodec::H264Avc);
    let mut surfaces = nv12::VideoSurfaceManager::new(4);
    let mut presenter = nv12::PresentationPipeline::new();

    // Register 160x90 test SPS
    let sps = decode::H264Sps {
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
        pic_width_in_mbs: 10,
        pic_height_in_map_units: 6,
        frame_mbs_only_flag: true,
        direct_8x8_inference_flag: true,
        frame_cropping_flag: true,
        crop_left: 0,
        crop_right: 0,
        crop_top: 0,
        crop_bottom: 3,
    };
    // Safety check on SPS dimensions
    if sps.width_pixels() != 160 || sps.height_pixels() != 90 {
        return false;
    }
    decoder.h264_sps = Some(sps);

    let surf_id = match surfaces.allocate_surface(160, 90, nv12::VideoSurfaceFormat::Nv12, nv12::VideoSurfaceTiling::Linear, 128) {
        Ok(id) => id,
        Err(_) => return false,
    };

    let slice = decode::SliceHeader {
        first_mb_in_slice: 0,
        slice_type: decode::SliceType::I,
        pic_parameter_set_id: 0,
        frame_num: 0,
        slice_qp_delta: 0,
        num_ref_idx_l0_active: 0,
        num_ref_idx_l1_active: 0,
        cabac_init_idc: 0,
    };

    let fence_val = match decoder.submit_slice_decode(&slice, 0x10000, 65536, surf_id, &[]) {
        Ok(f) => f,
        Err(_) => return false,
    };

    decoder.process_decode_queue();
    if !decoder.is_decode_complete(fence_val) {
        return false;
    }

    if surfaces.mark_display_pending(surf_id).is_err() {
        return false;
    }

    presenter.overlay_mut().configure_viewport(0, 0, 1920, 1080, 1920, 1080);
    if presenter.queue_surface(surf_id, 16_666, 1_000).is_err() {
        return false;
    }

    let active_surf = presenter.on_vsync_flip(16_666);
    if active_surf != Some(surf_id) {
        return false;
    }

    crate::serial_write_str("  * End-to-end hardware video decode queue to direct scanout presentation pipeline verified\r\n");
    crate::serial_write_str("[KNT-VIDEO-DECODE-PASS] Kernel-direct hardware video decode and zero-copy presentation streamer operational\r\n");

    true
}
