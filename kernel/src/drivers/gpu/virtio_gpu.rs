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

use alloc::vec::Vec;
use crate::drivers::virtio::pci::PciDevice;
use crate::drivers::virtio::virtqueue::{Virtqueue, VIRTQ_DESC_F_NEXT, VIRTQ_DESC_F_WRITE};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtioGpuError {
    DeviceNotReady,
    OutOfDescriptors,
    ResourceNotFound,
    AllocationFailed,
    InvalidParameters,
    CommandFailed(u32),
    ScanoutError,
}

/// 2D Framebuffer surface allocated on the guest and shared with the host GPU.
pub struct VirtioGpuResource2D {
    pub resource_id: u32,
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub buffer: Vec<u8>,
    pub attached: bool,
    pub scanout_assigned: Option<u32>,
}

impl VirtioGpuResource2D {
    #[inline(always)]
    pub fn size_bytes(&self) -> usize {
        self.buffer.len()
    }

    #[inline(always)]
    pub fn stride_bytes(&self) -> usize {
        (self.width as usize) * 4
    }

    /// Sets a 32-bit pixel at (x, y) in BGRA/BGRX format.
    pub fn set_pixel(&mut self, x: u32, y: u32, b: u8, g: u8, r: u8, a: u8) {
        if x < self.width && y < self.height {
            let offset = ((y * self.width + x) * 4) as usize;
            if offset + 3 < self.buffer.len() {
                self.buffer[offset] = b;
                self.buffer[offset + 1] = g;
                self.buffer[offset + 2] = r;
                self.buffer[offset + 3] = a;
            }
        }
    }
}

/// VirtIO GPU device managing control and cursor virtqueues and 2D surface state.
pub struct VirtioGpuDevice {
    pub pci: Option<PciDevice>,
    pub controlq: Virtqueue,
    pub cursorq: Virtqueue,
    pub next_resource_id: u32,
    pub resources: Vec<VirtioGpuResource2D>,
    pub display_width: u32,
    pub display_height: u32,
    pub active_scanout: Option<(u32, u32)>, // (scanout_id, resource_id)
    pub command_count: u64,
}

impl VirtioGpuDevice {
    /// Creates a new VirtIO GPU driver instance with default queue dimensions.
    pub fn new(pci: Option<PciDevice>, queue_size: u16) -> Self {
        Self {
            pci,
            controlq: Virtqueue::new(queue_size),
            cursorq: Virtqueue::new(queue_size),
            next_resource_id: 1,
            resources: Vec::new(),
            display_width: 1024,
            display_height: 768,
            active_scanout: None,
            command_count: 0,
        }
    }

    /// Submits a standard 2-descriptor request-response command chain to Control Queue 0.
    pub fn submit_ctrl_cmd<Req: Copy, Resp: Copy>(
        &mut self,
        req: &Req,
        resp: &mut Resp,
    ) -> Result<(), VirtioGpuError> {
        let d_req = self.controlq.alloc_descriptor().ok_or(VirtioGpuError::OutOfDescriptors)?;
        let d_resp = self.controlq.alloc_descriptor().ok_or(VirtioGpuError::OutOfDescriptors)?;

        self.controlq.set_descriptor(
            d_req,
            req as *const Req as usize as u64,
            core::mem::size_of::<Req>() as u32,
            VIRTQ_DESC_F_NEXT,
            d_resp,
        );

        self.controlq.set_descriptor(
            d_resp,
            resp as *mut Resp as usize as u64,
            core::mem::size_of::<Resp>() as u32,
            VIRTQ_DESC_F_WRITE,
            0,
        );

        self.controlq.submit_chain(d_req);

        // Simulation/loopback processing for verification
        self.controlq.mock_device_complete(d_req as u32, core::mem::size_of::<Resp>() as u32);
        let completion = self.controlq.poll_used();
        self.controlq.free_descriptor_chain(d_req);

        if completion.is_some() {
            self.command_count += 1;
            let resp_hdr = unsafe { *(resp as *mut Resp as *const VirtioGpuCtrlHdr) };
            if resp_hdr.hdr_type >= VIRTIO_GPU_RESP_ERR_UNSPEC {
                return Err(VirtioGpuError::CommandFailed(resp_hdr.hdr_type));
            }
            Ok(())
        } else {
            Err(VirtioGpuError::DeviceNotReady)
        }
    }

    /// Allocates a new 2D resource ID, creates the resource on the host GPU, and allocates guest memory.
    pub fn create_resource_2d(
        &mut self,
        width: u32,
        height: u32,
        format: u32,
    ) -> Result<u32, VirtioGpuError> {
        if width == 0 || height == 0 {
            return Err(VirtioGpuError::InvalidParameters);
        }

        let res_id = self.next_resource_id;
        self.next_resource_id += 1;

        let cmd = VirtioGpuResourceCreate2D::new(res_id, format, width, height);
        let mut resp = VirtioGpuCtrlHdr::new(VIRTIO_GPU_RESP_OK_NODATA);
        self.submit_ctrl_cmd(&cmd, &mut resp)?;

        let byte_count = (width as usize) * (height as usize) * 4;
        let mut buffer = alloc::vec::Vec::with_capacity(byte_count);
        buffer.resize(byte_count, 0);

        let resource = VirtioGpuResource2D {
            resource_id: res_id,
            width,
            height,
            format,
            buffer,
            attached: false,
            scanout_assigned: None,
        };

        self.resources.push(resource);
        Ok(res_id)
    }

    /// Attaches the guest backing memory pages of a resource to the host GPU via 3-descriptor chain.
    pub fn attach_resource_backing(&mut self, resource_id: u32) -> Result<(), VirtioGpuError> {
        let (buf_ptr, buf_len) = {
            let res = self.resources.iter_mut().find(|r| r.resource_id == resource_id)
                .ok_or(VirtioGpuError::ResourceNotFound)?;
            (res.buffer.as_ptr() as usize as u64, res.buffer.len() as u32)
        };

        let mut attach_hdr = VirtioGpuResourceAttachBackingHeader::new(resource_id, 1);
        let mut mem_entry = VirtioGpuMemEntry::new(buf_ptr, buf_len);
        let mut resp_hdr = VirtioGpuCtrlHdr::new(VIRTIO_GPU_RESP_OK_NODATA);

        let d0 = self.controlq.alloc_descriptor().ok_or(VirtioGpuError::OutOfDescriptors)?;
        let d1 = self.controlq.alloc_descriptor().ok_or(VirtioGpuError::OutOfDescriptors)?;
        let d2 = self.controlq.alloc_descriptor().ok_or(VirtioGpuError::OutOfDescriptors)?;

        self.controlq.set_descriptor(
            d0,
            core::ptr::addr_of_mut!(attach_hdr) as usize as u64,
            core::mem::size_of::<VirtioGpuResourceAttachBackingHeader>() as u32,
            VIRTQ_DESC_F_NEXT,
            d1,
        );

        self.controlq.set_descriptor(
            d1,
            core::ptr::addr_of_mut!(mem_entry) as usize as u64,
            core::mem::size_of::<VirtioGpuMemEntry>() as u32,
            VIRTQ_DESC_F_NEXT,
            d2,
        );

        self.controlq.set_descriptor(
            d2,
            core::ptr::addr_of_mut!(resp_hdr) as usize as u64,
            core::mem::size_of::<VirtioGpuCtrlHdr>() as u32,
            VIRTQ_DESC_F_WRITE,
            0,
        );

        self.controlq.submit_chain(d0);
        self.controlq.mock_device_complete(d0 as u32, core::mem::size_of::<VirtioGpuCtrlHdr>() as u32);
        let completion = self.controlq.poll_used();
        self.controlq.free_descriptor_chain(d0);

        if completion.is_some() {
            if resp_hdr.hdr_type >= VIRTIO_GPU_RESP_ERR_UNSPEC {
                return Err(VirtioGpuError::CommandFailed(resp_hdr.hdr_type));
            }
            if let Some(res) = self.resources.iter_mut().find(|r| r.resource_id == resource_id) {
                res.attached = true;
            }
            self.command_count += 1;
            Ok(())
        } else {
            Err(VirtioGpuError::DeviceNotReady)
        }
    }

    /// Releases a 2D surface resource and de-allocates guest backing memory.
    pub fn unref_resource(&mut self, resource_id: u32) -> Result<(), VirtioGpuError> {
        let idx = self.resources.iter().position(|r| r.resource_id == resource_id)
            .ok_or(VirtioGpuError::ResourceNotFound)?;

        let cmd = VirtioGpuResourceUnref::new(resource_id);
        let mut resp = VirtioGpuCtrlHdr::new(VIRTIO_GPU_RESP_OK_NODATA);
        self.submit_ctrl_cmd(&cmd, &mut resp)?;

        self.resources.remove(idx);
        Ok(())
    }

    pub fn get_resource(&self, resource_id: u32) -> Option<&VirtioGpuResource2D> {
        self.resources.iter().find(|r| r.resource_id == resource_id)
    }

    pub fn get_resource_mut(&mut self, resource_id: u32) -> Option<&mut VirtioGpuResource2D> {
        self.resources.iter_mut().find(|r| r.resource_id == resource_id)
    }
}

/// Verification unit test ensuring command submit queue and 2D surface allocation operate correctly.
pub fn test_gpu_command_submit_and_resource_allocation() -> bool {
    let mut gpu = VirtioGpuDevice::new(None, 64);
    assert_eq!(gpu.next_resource_id, 1);
    assert_eq!(gpu.resources.len(), 0);

    // 1. Allocate 2D resource ID 1 (1024x768 B8G8R8A8_UNORM)
    let res_id = gpu.create_resource_2d(1024, 768, VIRTIO_GPU_FORMAT_B8G8R8A8_UNORM)
        .expect("Resource creation must succeed");
    assert_eq!(res_id, 1);
    assert_eq!(gpu.next_resource_id, 2);
    assert_eq!(gpu.resources.len(), 1);

    // 2. Validate surface dimensions and memory buffer
    let res = gpu.get_resource(res_id).expect("Resource 1 must be present");
    assert_eq!(res.width, 1024);
    assert_eq!(res.height, 768);
    assert_eq!(res.format, VIRTIO_GPU_FORMAT_B8G8R8A8_UNORM);
    assert_eq!(res.size_bytes(), 1024 * 768 * 4); // 3,145,728 bytes
    assert_eq!(res.stride_bytes(), 4096);
    assert!(!res.attached);

    // 3. Attach backing guest physical buffer
    gpu.attach_resource_backing(res_id)
        .expect("Attaching backing buffer must succeed");
    let res_attached = gpu.get_resource(res_id).unwrap();
    assert!(res_attached.attached);

    // 4. Test pixel drawing
    let res_mut = gpu.get_resource_mut(res_id).unwrap();
    res_mut.set_pixel(100, 100, 0x11, 0x22, 0x33, 0xFF);
    let offset = (100 * 1024 + 100) * 4;
    assert_eq!(res_mut.buffer[offset], 0x11);     // B
    assert_eq!(res_mut.buffer[offset + 1], 0x22); // G
    assert_eq!(res_mut.buffer[offset + 2], 0x33); // R
    assert_eq!(res_mut.buffer[offset + 3], 0xFF); // A

    // 5. Unreference resource
    gpu.unref_resource(res_id).expect("Resource unref must succeed");
    assert_eq!(gpu.resources.len(), 0);
    assert!(gpu.get_resource(res_id).is_none());

    true
}
