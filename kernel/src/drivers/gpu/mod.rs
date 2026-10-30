//! GPU and display acceleration subsystem for KntonOS.

pub mod drm;
pub mod modeset;
pub mod timing;
pub mod virtio_gpu;
pub mod panel;
pub mod rotation;
pub mod vrr;
pub mod tdr;

pub use drm::*;
pub use modeset::*;
pub use panel::*;
pub use rotation::*;
pub use timing::*;
pub use virtio_gpu::{
    test_gpu_command_submit_and_resource_allocation, test_scanout_flush_and_zero_copy_handover,
    test_virtio_gpu_packet_structures, test_virtio_gpu_subsystem, VirtioGpuCtrlHdr,
    VirtioGpuCursorPos, VirtioGpuDevice, VirtioGpuDisplayOne, VirtioGpuError, VirtioGpuGetEdid,
    VirtioGpuMemEntry, VirtioGpuRect, VirtioGpuResource2D, VirtioGpuResourceAttachBackingHeader,
    VirtioGpuResourceCreate2D, VirtioGpuResourceDetachBacking, VirtioGpuResourceFlush,
    VirtioGpuResourceUnref, VirtioGpuRespDisplayInfo, VirtioGpuRespEdid, VirtioGpuSetScanout,
    VirtioGpuTransferToHost2D, VirtioGpuUpdateCursor, PCI_DEVICE_ID_VIRTIO_GPU,
    VIRTIO_GPU_CMD_GET_CAPSET, VIRTIO_GPU_CMD_GET_CAPSET_INFO, VIRTIO_GPU_CMD_GET_DISPLAY_INFO,
    VIRTIO_GPU_CMD_GET_EDID, VIRTIO_GPU_CMD_MOVE_CURSOR, VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING,
    VIRTIO_GPU_CMD_RESOURCE_CREATE_2D, VIRTIO_GPU_CMD_RESOURCE_DETACH_BACKING,
    VIRTIO_GPU_CMD_RESOURCE_FLUSH, VIRTIO_GPU_CMD_RESOURCE_UNREF, VIRTIO_GPU_CMD_SET_SCANOUT,
    VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D, VIRTIO_GPU_CMD_UPDATE_CURSOR, VIRTIO_GPU_CONTROL_QUEUE,
    VIRTIO_GPU_CURSOR_QUEUE, VIRTIO_GPU_FLAG_FENCE, VIRTIO_GPU_FORMAT_A8R8G8B8_UNORM,
    VIRTIO_GPU_FORMAT_B8G8R8A8_UNORM, VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM,
    VIRTIO_GPU_FORMAT_R8G8B8A8_UNORM, VIRTIO_GPU_FORMAT_X8B8G8R8_UNORM,
    VIRTIO_GPU_FORMAT_X8R8G8B8_UNORM, VIRTIO_GPU_MAX_SCANOUTS, VIRTIO_GPU_RESP_ERR_INVALID_CONTEXT_ID,
    VIRTIO_GPU_RESP_ERR_INVALID_PARAMETER, VIRTIO_GPU_RESP_ERR_INVALID_RESOURCE_ID,
    VIRTIO_GPU_RESP_ERR_INVALID_SCANOUT_ID, VIRTIO_GPU_RESP_ERR_OUT_OF_MEMORY,
    VIRTIO_GPU_RESP_ERR_UNSPEC, VIRTIO_GPU_RESP_OK_CAPSET, VIRTIO_GPU_RESP_OK_CAPSET_INFO,
    VIRTIO_GPU_RESP_OK_DISPLAY_INFO, VIRTIO_GPU_RESP_OK_EDID, VIRTIO_GPU_RESP_OK_NODATA,
    VIRTIO_ID_GPU,
};
pub use vrr::*;
pub use tdr::*;

use crate::serial_write_str;

/// Verification self-test suite for display panel orientation detection,
/// zero-overhead scanout rotation, and touchscreen coordinate normalization.
pub fn test_display_rotation_and_panel_subsystem() -> bool {
    serial_write_str("[KNT-GFX-ROT] Testing panel orientation detection & zero-overhead scanout rotation...\r\n");

    // 1. Test DMI Chassis & Handheld Mobile Panel Quirk Detection
    {
        // Valve Steam Deck LCD
        let deck_dmi = DmiSystemInfo {
            sys_vendor: "Valve",
            product_name: "Jupiter",
            product_version: "1",
            board_vendor: "Valve",
            board_name: "Jupiter",
            chassis_type: ChassisType::Handheld,
        };
        let deck_panel = match PanelDetector::detect_from_dmi(&deck_dmi) {
            Some(p) => p,
            None => {
                serial_write_str("[KNT-PANIC] Steam Deck DMI panel quirk detection failed!\r\n");
                return false;
            }
        };

        if deck_panel.physical_width != 800
            || deck_panel.physical_height != 1280
            || deck_panel.logical_width != 1280
            || deck_panel.logical_height != 800
            || deck_panel.orientation != PanelOrientation::RightUp
            || !deck_panel.is_portrait_native
        {
            serial_write_str("[KNT-PANIC] Steam Deck panel descriptor mismatch!\r\n");
            return false;
        }

        // ASUS ROG Ally
        let ally_dmi = DmiSystemInfo {
            sys_vendor: "ASUSTeK COMPUTER INC.",
            product_name: "RC71L",
            product_version: "1.0",
            board_vendor: "ASUSTeK COMPUTER INC.",
            board_name: "RC71L",
            chassis_type: ChassisType::Handheld,
        };
        let ally_panel = match PanelDetector::detect_from_dmi(&ally_dmi) {
            Some(p) => p,
            None => {
                serial_write_str("[KNT-PANIC] ASUS ROG Ally DMI panel quirk detection failed!\r\n");
                return false;
            }
        };

        if ally_panel.physical_width != 1080
            || ally_panel.physical_height != 1920
            || ally_panel.logical_width != 1920
            || ally_panel.logical_height != 1080
            || ally_panel.orientation != PanelOrientation::RightUp
        {
            serial_write_str("[KNT-PANIC] ASUS ROG Ally panel descriptor mismatch!\r\n");
            return false;
        }

        // Generic Desktop Fallback
        let desktop_dmi = DmiSystemInfo {
            sys_vendor: "Dell Inc.",
            product_name: "OptiPlex 7090",
            product_version: "01",
            board_vendor: "Dell Inc.",
            board_name: "0N8G5V",
            chassis_type: ChassisType::Desktop,
        };
        let desk_panel = PanelDetector::detect_or_fallback(Some(&desktop_dmi), 1920, 1080, 60);
        if desk_panel.orientation != PanelOrientation::Normal
            || desk_panel.logical_width != 1920
            || desk_panel.logical_height != 1080
            || desk_panel.is_portrait_native
        {
            serial_write_str("[KNT-PANIC] Desktop fallback panel detection mismatch!\r\n");
            return false;
        }

        // Unidentified Portrait Mobile Panel Fallback (e.g. 1080x2400)
        let unk_panel = PanelDetector::detect_or_fallback(None, 1080, 2400, 60);
        if unk_panel.orientation != PanelOrientation::RightUp
            || unk_panel.logical_width != 2400
            || unk_panel.logical_height != 1080
            || !unk_panel.is_portrait_native
        {
            serial_write_str("[KNT-PANIC] Unidentified portrait panel fallback failed!\r\n");
            return false;
        }
    }

    // 2. Test Scanout Affine Transformation Matrix
    {
        let matrix_90 = ScanoutTransformMatrix::for_orientation(PanelOrientation::RightUp, 1280, 800);

        // Physical (0, 0) -> logical (0, 799)
        let (lx0, ly0) = matrix_90.map_panel_to_logical_u32(0, 0);
        if lx0 != 0 || ly0 != 799 {
            serial_write_str("[KNT-PANIC] Matrix 90 deg corner (0, 0) mismatch!\r\n");
            return false;
        }

        // Physical (799, 0) -> logical (0, 0)
        let (lx1, ly1) = matrix_90.map_panel_to_logical_u32(799, 0);
        if lx1 != 0 || ly1 != 0 {
            serial_write_str("[KNT-PANIC] Matrix 90 deg corner (799, 0) mismatch!\r\n");
            return false;
        }

        // Physical (0, 1279) -> logical (1279, 799)
        let (lx2, ly2) = matrix_90.map_panel_to_logical_u32(0, 1279);
        if lx2 != 1279 || ly2 != 799 {
            serial_write_str("[KNT-PANIC] Matrix 90 deg corner (0, 1279) mismatch!\r\n");
            return false;
        }

        // Physical (799, 1279) -> logical (1279, 0)
        let (lx3, ly3) = matrix_90.map_panel_to_logical_u32(799, 1279);
        if lx3 != 1279 || ly3 != 0 {
            serial_write_str("[KNT-PANIC] Matrix 90 deg corner (799, 1279) mismatch!\r\n");
            return false;
        }
    }

    // 3. Test Zero-Overhead Hardware Scanout DMA Configuration & Direct Pixel Fetch
    {
        // 1. Verify 1280x800 full-scale scanout configuration and strides
        let log_w = 1280u32;
        let log_h = 800u32;
        let bpp = 4u32;
        let pitch = log_w * bpp; // 5120 bytes
        let total_size = (log_h * pitch) as usize; // 4,096,000 bytes

        let controller = ScanoutRotationController::new(
            0x1000_0000,
            log_w,
            log_h,
            pitch,
            bpp,
            PanelOrientation::RightUp,
        );

        if controller.config.pixel_stride_bytes != -(pitch as i32) {
            serial_write_str("[KNT-PANIC] Hardware pixel stride mismatch!\r\n");
            return false;
        }
        if controller.config.line_stride_bytes != (bpp as i32) {
            serial_write_str("[KNT-PANIC] Hardware line stride mismatch!\r\n");
            return false;
        }
        if !controller.config.verify_bounds(total_size) {
            serial_write_str("[KNT-PANIC] Hardware scanout bounds verification failed!\r\n");
            return false;
        }

        // 2. Test direct zero-copy scanout pixel retrieval on a compact 64x40 framebuffer (10 KB)
        let test_w = 64u32;
        let test_h = 40u32;
        let test_pitch = test_w * bpp; // 256 bytes
        let test_total = (test_h * test_pitch) as usize; // 10,240 bytes

        let test_controller = ScanoutRotationController::new(
            0x1000_0000,
            test_w,
            test_h,
            test_pitch,
            bpp,
            PanelOrientation::RightUp,
        );

        let mut fb = alloc::vec![0u8; test_total];

        let set_pixel = |buf: &mut [u8], lx: u32, ly: u32, val: u32| {
            let offset = (ly * test_pitch + lx * bpp) as usize;
            let bytes = val.to_ne_bytes();
            buf[offset..offset + 4].copy_from_slice(&bytes);
        };

        let marker_top_left = 0xAA00_0001u32;     // Logical (0, 0)
        let marker_top_right = 0xBB00_0002u32;    // Logical (63, 0)
        let marker_bottom_left = 0xCC00_0003u32;  // Logical (0, 39)
        let marker_bottom_right = 0xDD00_0004u32; // Logical (63, 39)

        set_pixel(&mut fb, 0, 0, marker_top_left);
        set_pixel(&mut fb, 63, 0, marker_top_right);
        set_pixel(&mut fb, 0, 39, marker_bottom_left);
        set_pixel(&mut fb, 63, 39, marker_bottom_right);

        // Fetch via physical panel coordinates using hardware scanout DMA addressing:
        // Physical (39, 0) corresponds to logical (0, 0)
        let p_top_left = test_controller.fetch_pixel_u32(&fb, 39, 0);
        if p_top_left != Some(marker_top_left) {
            serial_write_str("[KNT-PANIC] Scanout fetch for logical (0, 0) failed!\r\n");
            return false;
        }

        // Physical (39, 63) corresponds to logical (63, 0)
        let p_top_right = test_controller.fetch_pixel_u32(&fb, 39, 63);
        if p_top_right != Some(marker_top_right) {
            serial_write_str("[KNT-PANIC] Scanout fetch for logical (63, 0) failed!\r\n");
            return false;
        }

        // Physical (0, 0) corresponds to logical (0, 39)
        let p_bottom_left = test_controller.fetch_pixel_u32(&fb, 0, 0);
        if p_bottom_left != Some(marker_bottom_left) {
            serial_write_str("[KNT-PANIC] Scanout fetch for logical (0, 39) failed!\r\n");
            return false;
        }

        // Physical (0, 63) corresponds to logical (63, 39)
        let p_bottom_right = test_controller.fetch_pixel_u32(&fb, 0, 63);
        if p_bottom_right != Some(marker_bottom_right) {
            serial_write_str("[KNT-PANIC] Scanout fetch for logical (63, 39) failed!\r\n");
            return false;
        }
    }

    // 4. Test Touchscreen Coordinate Normalization & Orientation Transform
    {
        let touch_pipeline = TouchTransformPipeline::new(
            PanelOrientation::RightUp,
            800,  // Raw digitizer X max
            1280, // Raw digitizer Y max
            1280, // Desktop screen W
            800,  // Desktop screen H
        );

        // Tap physical (800, 0) -> top-right of portrait panel = top-left (0, 0) of landscape screen
        let t0 = touch_pipeline.transform(&RawTouchInput {
            contact_id: 1,
            raw_x: 800,
            raw_y: 0,
            pressure: 1024,
            touch_down: true,
        });
        if t0.screen_x != 0 || t0.screen_y != 0 {
            serial_write_str("[KNT-PANIC] Touch point transform to (0, 0) failed!\r\n");
            return false;
        }

        // Tap physical (800, 1280) -> bottom-right of portrait panel = top-right (1279, 0) of landscape screen
        let t1 = touch_pipeline.transform(&RawTouchInput {
            contact_id: 1,
            raw_x: 800,
            raw_y: 1280,
            pressure: 1024,
            touch_down: true,
        });
        if t1.screen_x != 1279 || t1.screen_y != 0 {
            serial_write_str("[KNT-PANIC] Touch point transform to (1279, 0) failed!\r\n");
            return false;
        }

        // Tap physical (0, 0) -> top-left of portrait panel = bottom-left (0, 799) of landscape screen
        let t2 = touch_pipeline.transform(&RawTouchInput {
            contact_id: 2,
            raw_x: 0,
            raw_y: 0,
            pressure: 512,
            touch_down: true,
        });
        if t2.screen_x != 0 || t2.screen_y != 799 {
            serial_write_str("[KNT-PANIC] Touch point transform to (0, 799) failed!\r\n");
            return false;
        }

        // Tap physical center (400, 640) -> desktop center (640, 400)
        let t_center = touch_pipeline.transform(&RawTouchInput {
            contact_id: 3,
            raw_x: 400,
            raw_y: 640,
            pressure: 2048,
            touch_down: true,
        });
        if (t_center.screen_x as i32 - 640).abs() > 1 || (t_center.screen_y as i32 - 400).abs() > 1 {
            serial_write_str("[KNT-PANIC] Touch point center transform failed!\r\n");
            return false;
        }
    }

    serial_write_str("[KNT-GFX-ROT-PASS] Display panel orientation detection and zero-overhead scanout rotation verified\r\n");
    true
}
