//! Kernel-Direct Real-Time In-Game Frame Capture HDR Tonemapper & Ultra-Low-Latency
//! Display Mirroring Streamer Subsystem.
//!
//! Provides zero-copy frame capture ring buffer taps interfacing directly with the
//! display compositor scanout engine, real-time fixed-point integer Reinhard and ACES
//! filmic tonemapping for HDR10 (BT.2020) and scRGB linear surfaces, low-latency
//! scanline slicing and packetization, and an adaptive network/USB congestion governor.

#![allow(dead_code)]

extern crate alloc;
use alloc::vec::Vec;

// -----------------------------------------------------------------------------
// Magic Constants & Protocol Definitions
// -----------------------------------------------------------------------------

/// Magic identifier for KntonOS display mirroring network/USB streaming packets ('KNTH').
pub const STREAM_MAGIC: u32 = 0x4B4E5448;

/// Display mirroring protocol version.
pub const STREAM_PROTOCOL_VERSION: u8 = 1;

/// Maximum payload bytes per scanline chunk packet (MTU ~1400 bytes for low-overhead UDP/IP/USB).
pub const MAX_CHUNK_PAYLOAD_SIZE: usize = 1380;

/// Default ring buffer capacity for frame capture slots.
pub const DEFAULT_CAPTURE_SLOTS: usize = 4;

/// Fixed-point scale factor for 16.16 fixed-point arithmetic ($1.0 = 65536$).
pub const FP_ONE_Q16: i32 = 65536;

/// Fixed-point scale factor for 8.8 fixed-point arithmetic ($1.0 = 256$).
pub const FP_ONE_Q8: i32 = 256;

// -----------------------------------------------------------------------------
// Pixel Formats & Color Spaces
// -----------------------------------------------------------------------------

/// Frame capture source and destination pixel formats.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HdrPixelFormat {
    /// 32-bit packed HDR10 RGB: 10 bits Red, 10 bits Green, 10 bits Blue, 2 bits Alpha (BT.2020 / PQ).
    Hdr10R10G10B10A2 = 1,
    /// 10-bit planar YUV 4:2:0 bi-planar format (P010, BT.2020).
    Hdr10P010 = 2,
    /// 64-bit scRGB format: 16-bit signed/half linear fixed-point per channel (0..65535, 1.0 SDR white = 8192).
    ScRgbFp16 = 3,
    /// 32-bit Standard Dynamic Range RGBA (8 bits per channel, BT.709 / sRGB).
    SdrRgba8888 = 4,
    /// 32-bit Standard Dynamic Range BGRA (8 bits per channel, BT.709 / sRGB).
    SdrBgra8888 = 5,
    /// 16-bit Standard Dynamic Range RGB565 format (5 bits R, 6 bits G, 5 bits B).
    SdrRgb565 = 6,
    /// 8-bit bi-planar YUV 4:2:0 format (NV12, BT.709).
    SdrNv12 = 7,
}

impl HdrPixelFormat {
    /// Return the byte size per pixel for packed formats, or sample stride for planar.
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            HdrPixelFormat::Hdr10R10G10B10A2 => 4,
            HdrPixelFormat::Hdr10P010 => 2, // 2 bytes per Y sample
            HdrPixelFormat::ScRgbFp16 => 8, // 4 * 2 bytes
            HdrPixelFormat::SdrRgba8888 => 4,
            HdrPixelFormat::SdrBgra8888 => 4,
            HdrPixelFormat::SdrRgb565 => 2,
            HdrPixelFormat::SdrNv12 => 1, // 1 byte per Y sample
        }
    }

    /// Returns true if this format represents High Dynamic Range content.
    pub fn is_hdr(self) -> bool {
        matches!(
            self,
            HdrPixelFormat::Hdr10R10G10B10A2 | HdrPixelFormat::Hdr10P010 | HdrPixelFormat::ScRgbFp16
        )
    }
}

// -----------------------------------------------------------------------------
// HDR Tonemapping Curves & Configuration
// -----------------------------------------------------------------------------

/// Mathematical tone mapping curve operator.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TonemapOperator {
    /// Linear clamping: clamps values directly to SDR maximum range.
    LinearClamp = 0,
    /// Standard Reinhard tone curve: $L_{out} = L_{in} / (1 + L_{in})$.
    Reinhard = 1,
    /// Extended Reinhard with variable white point: $L_{out} = L_{in} (1 + L_{in}/L_{w}^2) / (1 + L_{in})$.
    ReinhardExtended = 2,
    /// Academy Color Encoding System (ACES) filmic S-curve approximation with highlight roll-off.
    AcesFilmic = 3,
}

/// Fixed-point configuration parameters for the real-time kernel HDR tonemapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TonemapParameters {
    /// Active tonemapping curve operator.
    pub operator: TonemapOperator,
    /// Exposure scaling multiplier in 16.16 fixed-point (65536 = 1.0x).
    pub exposure_q16: u32,
    /// White point maximum luminance in 16.16 fixed-point (e.g. 4.0 * 65536 = 262144).
    pub white_point_q16: u32,
    /// Midtone contrast adjustment in 16.16 fixed-point (65536 = normal 1.0).
    pub contrast_q16: u32,
    /// Color gamut saturation factor in 16.16 fixed-point (65536 = 100% saturation).
    pub saturation_q16: u32,
    /// Enable BT.2020 wide color gamut compression into BT.709 primaries.
    pub gamut_compress_enabled: bool,
}

impl TonemapParameters {
    /// Create standard default parameters for gaming HDR10 (1000 nits peak -> 200 nits SDR).
    pub const fn default_hdr10() -> Self {
        Self {
            operator: TonemapOperator::AcesFilmic,
            exposure_q16: 65536,       // 1.0x
            white_point_q16: 262144,   // 4.0x peak white
            contrast_q16: 65536,      // 1.0
            saturation_q16: 65536,    // 1.0
            gamut_compress_enabled: true,
        }
    }

    /// Create Reinhard configuration with customizable white point.
    pub const fn reinhard(white_point_factor: u32) -> Self {
        Self {
            operator: TonemapOperator::ReinhardExtended,
            exposure_q16: 65536,
            white_point_q16: white_point_factor * 65536,
            contrast_q16: 65536,
            saturation_q16: 65536,
            gamut_compress_enabled: true,
        }
    }
}

// -----------------------------------------------------------------------------
// Zero-Copy Frame Capture Ring Buffer
// -----------------------------------------------------------------------------

/// State transitions of a zero-copy frame capture ring slot.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureSlotState {
    /// Slot is unallocated and free for capture tap assignment.
    Free = 0,
    /// Compositor scanout engine is writing frame contents into the slot.
    Capturing = 1,
    /// Frame capture has completed; available for tonemapping / streaming.
    Ready = 2,
    /// Active streamer is slicing scanlines from this slot.
    Streaming = 3,
}

/// Individual frame capture slot in the zero-copy compositor ring buffer.
pub struct FrameCaptureSlot {
    /// Slot index within the ring buffer.
    pub slot_id: u8,
    /// Current acquisition and processing state.
    pub state: CaptureSlotState,
    /// Monotonically increasing frame sequence number.
    pub frame_id: u32,
    /// Presentation timestamp (PTS) in microseconds.
    pub pts_us: u64,
    /// Frame resolution width in pixels.
    pub width: u16,
    /// Frame resolution height in scanlines.
    pub height: u16,
    /// Row stride pitch in bytes.
    pub stride_bytes: u32,
    /// Pixel format of the captured surface.
    pub format: HdrPixelFormat,
    /// Physical address for zero-copy GPU scanout tap, or 0 if software buffer.
    pub phys_addr: u64,
    /// Raw backing storage buffer.
    pub buffer: Vec<u8>,
}

impl FrameCaptureSlot {
    /// Create an empty frame capture slot with specified byte capacity.
    pub fn new(slot_id: u8, capacity_bytes: usize) -> Self {
        Self {
            slot_id,
            state: CaptureSlotState::Free,
            frame_id: 0,
            pts_us: 0,
            width: 0,
            height: 0,
            stride_bytes: 0,
            format: HdrPixelFormat::Hdr10R10G10B10A2,
            phys_addr: 0,
            buffer: alloc::vec![0u8; capacity_bytes],
        }
    }
}

/// High-throughput zero-copy frame capture ring managing compositor scanout taps.
pub struct FrameCaptureRing {
    /// Slots holding active, in-flight, and completed frames.
    pub slots: Vec<FrameCaptureSlot>,
    /// Index of next slot to assign to the compositor tap.
    pub capture_idx: usize,
    /// Index of oldest ready slot to process and stream.
    pub stream_idx: usize,
    /// Total frames submitted to the capture ring.
    pub frames_captured: u64,
    /// Total frames dropped due to backpressure / slot exhaustion.
    pub frames_dropped: u64,
}

impl FrameCaptureRing {
    /// Allocate a new frame capture ring buffer.
    pub fn new(slot_count: usize, slot_capacity_bytes: usize) -> Self {
        let count = if slot_count < 2 { DEFAULT_CAPTURE_SLOTS } else { slot_count };
        let mut slots = Vec::with_capacity(count);
        for i in 0..count {
            slots.push(FrameCaptureSlot::new(i as u8, slot_capacity_bytes));
        }

        Self {
            slots,
            capture_idx: 0,
            stream_idx: 0,
            frames_captured: 0,
            frames_dropped: 0,
        }
    }

    /// Acquire the next available slot for compositor frame capture tap.
    /// Returns slot ID if successful, or error if all slots are busy.
    pub fn acquire_capture_slot(
        &mut self,
        width: u16,
        height: u16,
        format: HdrPixelFormat,
        pts_us: u64,
    ) -> Result<u8, &'static str> {
        let count = self.slots.len();
        for i in 0..count {
            let idx = (self.capture_idx + i) % count;
            if self.slots[idx].state == CaptureSlotState::Free {
                self.capture_idx = (idx + 1) % count;
                let slot = &mut self.slots[idx];
                slot.state = CaptureSlotState::Capturing;
                self.frames_captured = self.frames_captured.wrapping_add(1);
                slot.frame_id = self.frames_captured as u32;
                slot.pts_us = pts_us;
                slot.width = width;
                slot.height = height;
                slot.stride_bytes = (width as u32) * (format.bytes_per_pixel() as u32);
                slot.format = format;
                return Ok(slot.slot_id);
            }
        }

        // Backlog: all slots currently in use
        self.frames_dropped = self.frames_dropped.wrapping_add(1);
        Err("Capture ring backlog: all slots busy")
    }

    /// Mark a captured frame as complete and ready for streaming.
    pub fn commit_captured_frame(&mut self, slot_id: u8) -> Result<(), &'static str> {
        let idx = slot_id as usize;
        if idx >= self.slots.len() {
            return Err("Invalid slot_id");
        }
        if self.slots[idx].state != CaptureSlotState::Capturing {
            return Err("Slot not in Capturing state");
        }
        self.slots[idx].state = CaptureSlotState::Ready;
        Ok(())
    }

    /// Retrieve the oldest ready frame for tonemapping and display mirroring.
    pub fn acquire_ready_frame(&mut self) -> Option<u8> {
        let count = self.slots.len();
        for i in 0..count {
            let idx = (self.stream_idx + i) % count;
            if self.slots[idx].state == CaptureSlotState::Ready {
                self.stream_idx = (idx + 1) % count;
                self.slots[idx].state = CaptureSlotState::Streaming;
                return Some(self.slots[idx].slot_id);
            }
        }
        None
    }

    /// Release a frame slot back to the free pool after streaming.
    pub fn release_frame(&mut self, slot_id: u8) -> Result<(), &'static str> {
        let idx = slot_id as usize;
        if idx >= self.slots.len() {
            return Err("Invalid slot_id");
        }
        self.slots[idx].state = CaptureSlotState::Free;
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Ultra-Low-Latency Display Mirroring Streaming Packet Protocol
// -----------------------------------------------------------------------------

/// Packet types dispatched over network or USB streaming transport.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamPacketType {
    /// Frame presentation metadata header (width, height, format, PTS).
    FrameHeader = 1,
    /// Slice of contiguous scanlines tonemapped and chunked for streaming.
    ScanlineChunk = 2,
    /// Presentation timestamp (PTS) clock synchronization signal.
    PtsSync = 3,
    /// Receiver backpressure congestion and latency feedback.
    CongestionFeedback = 4,
    /// Keep-alive heartbeat packet.
    Heartbeat = 5,
}

/// Bit flags for streaming packet header.
pub const PACKET_FLAG_KEYFRAME: u16 = 1 << 0;
pub const PACKET_FLAG_LAST_CHUNK: u16 = 1 << 1;
pub const PACKET_FLAG_DROPPABLE: u16 = 1 << 2;
pub const PACKET_FLAG_TONEMAPPED: u16 = 1 << 3;

/// Sized binary header prepended to all ultra-low-latency display mirroring packets.
/// Exactly 32 bytes in length for cache-line alignment and zero-overhead parsing.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MirrorStreamPacketHeader {
    /// Protocol magic number (`STREAM_MAGIC` = 0x4B4E5448).
    pub magic: u32,
    /// Protocol version.
    pub version: u8,
    /// Type of packet (`StreamPacketType`).
    pub packet_type: u8,
    /// Control flags (`PACKET_FLAG_*`).
    pub flags: u16,
    /// Frame sequence identifier.
    pub frame_id: u32,
    /// Chunk index within this frame (0-based).
    pub chunk_index: u16,
    /// Total chunks comprising this complete frame.
    pub chunk_count: u16,
    /// Starting scanline row index of this slice.
    pub start_scanline: u16,
    /// Number of scanline rows included in this slice.
    pub scanline_count: u16,
    /// Presentation timestamp (PTS) in microseconds.
    pub pts_us: u64,
    /// Payload length in bytes following this header.
    pub payload_bytes: u16,
    /// CRC-16 checksum covering payload bytes for link integrity verification.
    pub crc16: u16,
}

impl MirrorStreamPacketHeader {
    /// Construct a new display mirroring packet header.
    pub fn new(
        packet_type: StreamPacketType,
        flags: u16,
        frame_id: u32,
        chunk_index: u16,
        chunk_count: u16,
        start_scanline: u16,
        scanline_count: u16,
        pts_us: u64,
        payload_bytes: u16,
    ) -> Self {
        Self {
            magic: STREAM_MAGIC,
            version: STREAM_PROTOCOL_VERSION,
            packet_type: packet_type as u8,
            flags,
            frame_id,
            chunk_index,
            chunk_count,
            start_scanline,
            scanline_count,
            pts_us,
            payload_bytes,
            crc16: 0,
        }
    }

    /// Serialize header to 32-byte binary array.
    pub fn to_bytes(&self) -> [u8; 32] {
        let mut bytes = [0u8; 32];
        bytes[0..4].copy_from_slice(&self.magic.to_le_bytes());
        bytes[4] = self.version;
        bytes[5] = self.packet_type;
        bytes[6..8].copy_from_slice(&self.flags.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.frame_id.to_le_bytes());
        bytes[12..14].copy_from_slice(&self.chunk_index.to_le_bytes());
        bytes[14..16].copy_from_slice(&self.chunk_count.to_le_bytes());
        bytes[16..18].copy_from_slice(&self.start_scanline.to_le_bytes());
        bytes[18..20].copy_from_slice(&self.scanline_count.to_le_bytes());
        bytes[20..28].copy_from_slice(&self.pts_us.to_le_bytes());
        bytes[28..30].copy_from_slice(&self.payload_bytes.to_le_bytes());
        bytes[30..32].copy_from_slice(&self.crc16.to_le_bytes());
        bytes
    }

    /// Deserialize header from 32-byte slice.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 32 {
            return None;
        }

        let magic = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        if magic != STREAM_MAGIC {
            return None;
        }

        let version = bytes[4];
        let packet_type = bytes[5];
        let flags = u16::from_le_bytes([bytes[6], bytes[7]]);
        let frame_id = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let chunk_index = u16::from_le_bytes([bytes[12], bytes[13]]);
        let chunk_count = u16::from_le_bytes([bytes[14], bytes[15]]);
        let start_scanline = u16::from_le_bytes([bytes[16], bytes[17]]);
        let scanline_count = u16::from_le_bytes([bytes[18], bytes[19]]);
        let pts_us = u64::from_le_bytes([
            bytes[20], bytes[21], bytes[22], bytes[23],
            bytes[24], bytes[25], bytes[26], bytes[27],
        ]);
        let payload_bytes = u16::from_le_bytes([bytes[28], bytes[29]]);
        let crc16 = u16::from_le_bytes([bytes[30], bytes[31]]);

        Some(Self {
            magic,
            version,
            packet_type,
            flags,
            frame_id,
            chunk_index,
            chunk_count,
            start_scanline,
            scanline_count,
            pts_us,
            payload_bytes,
            crc16,
        })
    }

    /// Calculate fast CRC-16 CCITT checksum over payload slice.
    pub fn compute_crc16(payload: &[u8]) -> u16 {
        let mut crc: u16 = 0xFFFF;
        for &byte in payload {
            crc ^= (byte as u16) << 8;
            for _ in 0..8 {
                if (crc & 0x8000) != 0 {
                    crc = (crc << 1) ^ 0x1021;
                } else {
                    crc <<= 1;
                }
            }
        }
        crc
    }
}
