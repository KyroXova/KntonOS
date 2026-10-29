//! VirtIO GPU 2D/3D Hardware Acceleration & Scanout Display Driver Protocol.
//!
//! Implements OASIS VirtIO 1.0+ GPU device specifications (Section 5.7):
//! - Control queue (Queue 0) & Cursor queue (Queue 1) packet wire formats
//! - 2D resource allocation and host backing memory attachment
//! - Display scanout configuration, EDID mode discovery, and dirty rect flushing

use core::mem::size_of;

/// PCI Device ID for modern VirtIO GPU controllers.
pub const PCI_DEVICE_ID_VIRTIO_GPU: u16 = 0x1050;

/// VirtIO Subsystem Device ID for GPU.
pub const VIRTIO_ID_GPU: u16 = 16;

/// Queue index constants
pub const VIRTIO_GPU_CONTROL_QUEUE: u16 = 0;
pub const VIRTIO_GPU_CURSOR_QUEUE: u16 = 1;

/// Maximum number of scanouts supported by VirtIO GPU display response.
pub const VIRTIO_GPU_MAX_SCANOUTS: usize = 16;

// -----------------------------------------------------------------------------
// VirtIO GPU Command Types (2D / 3D / Cursor)
// -----------------------------------------------------------------------------

pub const VIRTIO_GPU_CMD_GET_DISPLAY_INFO: u32 = 0x0100;
pub const VIRTIO_GPU_CMD_RESOURCE_CREATE_2D: u32 = 0x0101;
pub const VIRTIO_GPU_CMD_RESOURCE_UNREF: u32 = 0x0102;
pub const VIRTIO_GPU_CMD_SET_SCANOUT: u32 = 0x0103;
pub const VIRTIO_GPU_CMD_RESOURCE_FLUSH: u32 = 0x0104;
pub const VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D: u32 = 0x0105;
pub const VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING: u32 = 0x0106;
pub const VIRTIO_GPU_CMD_RESOURCE_DETACH_BACKING: u32 = 0x0107;
pub const VIRTIO_GPU_CMD_GET_CAPSET_INFO: u32 = 0x0108;
pub const VIRTIO_GPU_CMD_GET_CAPSET: u32 = 0x0109;
pub const VIRTIO_GPU_CMD_GET_EDID: u32 = 0x010A;

pub const VIRTIO_GPU_CMD_UPDATE_CURSOR: u32 = 0x0300;
pub const VIRTIO_GPU_CMD_MOVE_CURSOR: u32 = 0x0301;

// -----------------------------------------------------------------------------
// VirtIO GPU Response Types
// -----------------------------------------------------------------------------

pub const VIRTIO_GPU_RESP_OK_NODATA: u32 = 0x1100;
pub const VIRTIO_GPU_RESP_OK_DISPLAY_INFO: u32 = 0x1101;
pub const VIRTIO_GPU_RESP_OK_CAPSET_INFO: u32 = 0x1102;
pub const VIRTIO_GPU_RESP_OK_CAPSET: u32 = 0x1103;
pub const VIRTIO_GPU_RESP_OK_EDID: u32 = 0x1104;

pub const VIRTIO_GPU_RESP_ERR_UNSPEC: u32 = 0x1200;
pub const VIRTIO_GPU_RESP_ERR_OUT_OF_MEMORY: u32 = 0x1201;
pub const VIRTIO_GPU_RESP_ERR_INVALID_SCANOUT_ID: u32 = 0x1202;
pub const VIRTIO_GPU_RESP_ERR_INVALID_RESOURCE_ID: u32 = 0x1203;
pub const VIRTIO_GPU_RESP_ERR_INVALID_CONTEXT_ID: u32 = 0x1204;
pub const VIRTIO_GPU_RESP_ERR_INVALID_PARAMETER: u32 = 0x1205;

// -----------------------------------------------------------------------------
// VirtIO GPU Formats
// -----------------------------------------------------------------------------

pub const VIRTIO_GPU_FORMAT_B8G8R8A8_UNORM: u32 = 1;
pub const VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM: u32 = 2;
pub const VIRTIO_GPU_FORMAT_A8R8G8B8_UNORM: u32 = 3;
pub const VIRTIO_GPU_FORMAT_X8R8G8B8_UNORM: u32 = 4;
pub const VIRTIO_GPU_FORMAT_R8G8B8A8_UNORM: u32 = 67;
pub const VIRTIO_GPU_FORMAT_X8B8G8R8_UNORM: u32 = 68;

// -----------------------------------------------------------------------------
// VirtIO GPU Flags
// -----------------------------------------------------------------------------

pub const VIRTIO_GPU_FLAG_FENCE: u32 = 1 << 0;

// -----------------------------------------------------------------------------
// Wire Protocol Packet Structures
// -----------------------------------------------------------------------------

/// Standard 24-byte control header present on all request and response packets.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VirtioGpuCtrlHdr {
    pub hdr_type: u32,
    pub flags: u32,
    pub fence_id: u64,
    pub ctx_id: u32,
    pub padding: u32,
}

impl VirtioGpuCtrlHdr {
    pub const fn new(hdr_type: u32) -> Self {
        Self {
            hdr_type,
            flags: 0,
            fence_id: 0,
            ctx_id: 0,
            padding: 0,
        }
    }
}

/// 16-byte 2D rectangle specifying screen/buffer boundaries.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VirtioGpuRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl VirtioGpuRect {
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }
}

/// Single display scanout descriptor in GET_DISPLAY_INFO response.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VirtioGpuDisplayOne {
    pub r: VirtioGpuRect,
    pub enabled: u32,
    pub flags: u32,
}

/// Complete response packet for VIRTIO_GPU_CMD_GET_DISPLAY_INFO.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VirtioGpuRespDisplayInfo {
    pub hdr: VirtioGpuCtrlHdr,
    pub pmodes: [VirtioGpuDisplayOne; VIRTIO_GPU_MAX_SCANOUTS],
}

impl Default for VirtioGpuRespDisplayInfo {
    fn default() -> Self {
        Self {
            hdr: VirtioGpuCtrlHdr::new(VIRTIO_GPU_RESP_OK_DISPLAY_INFO),
            pmodes: [VirtioGpuDisplayOne::default(); VIRTIO_GPU_MAX_SCANOUTS],
        }
    }
}

/// Request packet for VIRTIO_GPU_CMD_RESOURCE_CREATE_2D.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtioGpuResourceCreate2D {
    pub hdr: VirtioGpuCtrlHdr,
    pub resource_id: u32,
    pub format: u32,
    pub width: u32,
    pub height: u32,
}

impl VirtioGpuResourceCreate2D {
    pub const fn new(resource_id: u32, format: u32, width: u32, height: u32) -> Self {
        Self {
            hdr: VirtioGpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_CREATE_2D),
            resource_id,
            format,
            width,
            height,
        }
    }
}

/// Request packet for VIRTIO_GPU_CMD_RESOURCE_UNREF.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtioGpuResourceUnref {
    pub hdr: VirtioGpuCtrlHdr,
    pub resource_id: u32,
    pub padding: u32,
}

impl VirtioGpuResourceUnref {
    pub const fn new(resource_id: u32) -> Self {
        Self {
            hdr: VirtioGpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_UNREF),
            resource_id,
            padding: 0,
        }
    }
}

/// Request packet for VIRTIO_GPU_CMD_SET_SCANOUT.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtioGpuSetScanout {
    pub hdr: VirtioGpuCtrlHdr,
    pub r: VirtioGpuRect,
    pub scanout_id: u32,
    pub resource_id: u32,
}

impl VirtioGpuSetScanout {
    pub const fn new(scanout_id: u32, resource_id: u32, r: VirtioGpuRect) -> Self {
        Self {
            hdr: VirtioGpuCtrlHdr::new(VIRTIO_GPU_CMD_SET_SCANOUT),
            r,
            scanout_id,
            resource_id,
        }
    }
}

/// Request packet for VIRTIO_GPU_CMD_RESOURCE_FLUSH.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtioGpuResourceFlush {
    pub hdr: VirtioGpuCtrlHdr,
    pub r: VirtioGpuRect,
    pub resource_id: u32,
    pub padding: u32,
}

impl VirtioGpuResourceFlush {
    pub const fn new(resource_id: u32, r: VirtioGpuRect) -> Self {
        Self {
            hdr: VirtioGpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_FLUSH),
            r,
            resource_id,
            padding: 0,
        }
    }
}

/// Request packet for VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtioGpuTransferToHost2D {
    pub hdr: VirtioGpuCtrlHdr,
    pub r: VirtioGpuRect,
    pub offset: u64,
    pub resource_id: u32,
    pub padding: u32,
}

impl VirtioGpuTransferToHost2D {
    pub const fn new(resource_id: u32, offset: u64, r: VirtioGpuRect) -> Self {
        Self {
            hdr: VirtioGpuCtrlHdr::new(VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D),
            r,
            offset,
            resource_id,
            padding: 0,
        }
    }
}

/// Memory entry descriptor describing physical guest pages backing a GPU resource.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VirtioGpuMemEntry {
    pub addr: u64,
    pub length: u32,
    pub padding: u32,
}

impl VirtioGpuMemEntry {
    pub const fn new(addr: u64, length: u32) -> Self {
        Self {
            addr,
            length,
            padding: 0,
        }
    }
}

/// Header for VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtioGpuResourceAttachBackingHeader {
    pub hdr: VirtioGpuCtrlHdr,
    pub resource_id: u32,
    pub nr_entries: u32,
}

impl VirtioGpuResourceAttachBackingHeader {
    pub const fn new(resource_id: u32, nr_entries: u32) -> Self {
        Self {
            hdr: VirtioGpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING),
            resource_id,
            nr_entries,
        }
    }
}

/// Request packet for VIRTIO_GPU_CMD_RESOURCE_DETACH_BACKING.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtioGpuResourceDetachBacking {
    pub hdr: VirtioGpuCtrlHdr,
    pub resource_id: u32,
    pub padding: u32,
}

impl VirtioGpuResourceDetachBacking {
    pub const fn new(resource_id: u32) -> Self {
        Self {
            hdr: VirtioGpuCtrlHdr::new(VIRTIO_GPU_CMD_RESOURCE_DETACH_BACKING),
            resource_id,
            padding: 0,
        }
    }
}

/// Cursor position parameters.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VirtioGpuCursorPos {
    pub scanout_id: u32,
    pub x: u32,
    pub y: u32,
    pub padding: u32,
}

/// Request packet for cursor updates on Cursor Queue (Queue 1).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtioGpuUpdateCursor {
    pub hdr: VirtioGpuCtrlHdr,
    pub pos: VirtioGpuCursorPos,
    pub resource_id: u32,
    pub hot_x: u32,
    pub hot_y: u32,
    pub padding: u32,
}

/// Request packet for VIRTIO_GPU_CMD_GET_EDID.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtioGpuGetEdid {
    pub hdr: VirtioGpuCtrlHdr,
    pub scanout: u32,
    pub padding: u32,
}

impl VirtioGpuGetEdid {
    pub const fn new(scanout: u32) -> Self {
        Self {
            hdr: VirtioGpuCtrlHdr::new(VIRTIO_GPU_CMD_GET_EDID),
            scanout,
            padding: 0,
        }
    }
}

/// Response packet for VIRTIO_GPU_CMD_GET_EDID.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VirtioGpuRespEdid {
    pub hdr: VirtioGpuCtrlHdr,
    pub size: u32,
    pub padding: u32,
    pub edid: [u8; 1024],
}

// -----------------------------------------------------------------------------
// Unit Tests for Protocol Structures
// -----------------------------------------------------------------------------

/// Verification unit test for VirtIO GPU protocol structures, layout, and commands.
pub fn test_virtio_gpu_packet_structures() -> bool {
    // 1. Verify byte sizes match OASIS VirtIO specification
    assert_eq!(size_of::<VirtioGpuCtrlHdr>(), 24);
    assert_eq!(size_of::<VirtioGpuRect>(), 16);
    assert_eq!(size_of::<VirtioGpuDisplayOne>(), 24);
    assert_eq!(size_of::<VirtioGpuRespDisplayInfo>(), 24 + 16 * 24); // 408
    assert_eq!(size_of::<VirtioGpuResourceCreate2D>(), 40);
    assert_eq!(size_of::<VirtioGpuResourceUnref>(), 32);
    assert_eq!(size_of::<VirtioGpuSetScanout>(), 48);
    assert_eq!(size_of::<VirtioGpuResourceFlush>(), 48);
    assert_eq!(size_of::<VirtioGpuTransferToHost2D>(), 56);
    assert_eq!(size_of::<VirtioGpuMemEntry>(), 16);
    assert_eq!(size_of::<VirtioGpuResourceAttachBackingHeader>(), 32);
    assert_eq!(size_of::<VirtioGpuResourceDetachBacking>(), 32);
    assert_eq!(size_of::<VirtioGpuCursorPos>(), 16);
    assert_eq!(size_of::<VirtioGpuUpdateCursor>(), 56);
    assert_eq!(size_of::<VirtioGpuGetEdid>(), 32);
    assert_eq!(size_of::<VirtioGpuRespEdid>(), 24 + 8 + 1024); // 1056

    // 2. Verify command codes
    assert_eq!(VIRTIO_GPU_CMD_GET_DISPLAY_INFO, 0x0100);
    assert_eq!(VIRTIO_GPU_CMD_RESOURCE_CREATE_2D, 0x0101);
    assert_eq!(VIRTIO_GPU_CMD_RESOURCE_UNREF, 0x0102);
    assert_eq!(VIRTIO_GPU_CMD_SET_SCANOUT, 0x0103);
    assert_eq!(VIRTIO_GPU_CMD_RESOURCE_FLUSH, 0x0104);
    assert_eq!(VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D, 0x0105);
    assert_eq!(VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING, 0x0106);
    assert_eq!(VIRTIO_GPU_CMD_RESOURCE_DETACH_BACKING, 0x0107);
    assert_eq!(VIRTIO_GPU_CMD_GET_CAPSET_INFO, 0x0108);
    assert_eq!(VIRTIO_GPU_CMD_GET_CAPSET, 0x0109);
    assert_eq!(VIRTIO_GPU_CMD_GET_EDID, 0x010A);

    // 3. Verify response codes
    assert_eq!(VIRTIO_GPU_RESP_OK_NODATA, 0x1100);
    assert_eq!(VIRTIO_GPU_RESP_OK_DISPLAY_INFO, 0x1101);
    assert_eq!(VIRTIO_GPU_RESP_OK_CAPSET_INFO, 0x1102);
    assert_eq!(VIRTIO_GPU_RESP_OK_CAPSET, 0x1103);
    assert_eq!(VIRTIO_GPU_RESP_OK_EDID, 0x1104);

    // 4. Verify 2D packet constructors
    let rect = VirtioGpuRect::new(10, 20, 1024, 768);
    assert_eq!(rect.x, 10);
    assert_eq!(rect.y, 20);
    assert_eq!(rect.width, 1024);
    assert_eq!(rect.height, 768);

    let create_cmd = VirtioGpuResourceCreate2D::new(1, VIRTIO_GPU_FORMAT_B8G8R8A8_UNORM, 1024, 768);
    assert_eq!(create_cmd.hdr.hdr_type, VIRTIO_GPU_CMD_RESOURCE_CREATE_2D);
    assert_eq!(create_cmd.resource_id, 1);
    assert_eq!(create_cmd.format, VIRTIO_GPU_FORMAT_B8G8R8A8_UNORM);
    assert_eq!(create_cmd.width, 1024);
    assert_eq!(create_cmd.height, 768);

    let set_scanout = VirtioGpuSetScanout::new(0, 1, rect);
    assert_eq!(set_scanout.hdr.hdr_type, VIRTIO_GPU_CMD_SET_SCANOUT);
    assert_eq!(set_scanout.scanout_id, 0);
    assert_eq!(set_scanout.resource_id, 1);
    assert_eq!(set_scanout.r, rect);

    let flush_cmd = VirtioGpuResourceFlush::new(1, rect);
    assert_eq!(flush_cmd.hdr.hdr_type, VIRTIO_GPU_CMD_RESOURCE_FLUSH);
    assert_eq!(flush_cmd.resource_id, 1);
    assert_eq!(flush_cmd.r, rect);

    let transfer_cmd = VirtioGpuTransferToHost2D::new(1, 0, rect);
    assert_eq!(transfer_cmd.hdr.hdr_type, VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D);
    assert_eq!(transfer_cmd.resource_id, 1);
    assert_eq!(transfer_cmd.offset, 0);
    assert_eq!(transfer_cmd.r, rect);

    let attach_hdr = VirtioGpuResourceAttachBackingHeader::new(1, 4);
    assert_eq!(attach_hdr.hdr.hdr_type, VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING);
    assert_eq!(attach_hdr.resource_id, 1);
    assert_eq!(attach_hdr.nr_entries, 4);

    true
}
