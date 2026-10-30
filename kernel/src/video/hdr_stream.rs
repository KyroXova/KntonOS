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


// -----------------------------------------------------------------------------
// Real-Time Fixed-Point HDR Tonemapper & Gamut Compressor Implementation
// -----------------------------------------------------------------------------

/// Unpack 32-bit packed HDR10 R10G10B10A2 pixel into 10-bit components and 2-bit alpha.
#[inline(always)]
pub fn unpack_r10g10b10a2(packed: u32) -> (u16, u16, u16, u8) {
    let r = (packed & 0x3FF) as u16;
    let g = ((packed >> 10) & 0x3FF) as u16;
    let b = ((packed >> 20) & 0x3FF) as u16;
    let a = ((packed >> 30) & 0x03) as u8;
    (r, g, b, a)
}

/// Pack 10-bit color components and 2-bit alpha into 32-bit R10G10B10A2 pixel.
#[inline(always)]
pub fn pack_r10g10b10a2(r: u16, g: u16, b: u16, a: u8) -> u32 {
    ((r as u32) & 0x3FF)
        | (((g as u32) & 0x3FF) << 10)
        | (((b as u32) & 0x3FF) << 20)
        | (((a as u32) & 0x03) << 30)
}

/// Convert 10-bit HDR10 PQ sample value into normalized linear luminance in 16.16 fixed point.
/// Calibrated so that 512 (reference SDR paper-white) maps to 1.0 (65536),
/// and 1023 (maximum HDR peak) maps to 4.0x (262144).
#[inline]
pub fn hdr10_to_linear_q16(val_10bit: u16) -> u32 {
    let v = val_10bit.min(1023) as u32;
    if v <= 512 {
        // Linear mapping up to 1.0 (65536)
        v * 128
    } else {
        // Non-linear expansion above paper-white up to 4.0x (262144)
        let delta = v - 512;
        65536 + ((delta as u64 * 196608) / 511) as u32
    }
}

/// Convert 16-bit scRGB linear sample into normalized 16.16 fixed-point luminance.
/// Standard scRGB maps 1.0 (paper-white) to 8192.
#[inline]
pub fn scrgb_to_linear_q16(val_16bit: u16) -> u32 {
    let v = val_16bit as u32;
    (v * 65536) / 8192
}

/// Apply extended Reinhard tonemapping curve:
/// $L_{out} = L_{in} (1 + L_{in} / L_{w}^2) / (1 + L_{in})$.
/// Computed entirely in 64-bit integer arithmetic without floating-point instructions.
pub fn apply_tonemap_reinhard(x_q16: u32, white_point_q16: u32) -> u32 {
    if x_q16 == 0 {
        return 0;
    }
    let x = x_q16 as u64;
    let w = white_point_q16.max(65536) as u64;
    let w2 = (w * w) >> 16;
    let w2_safe = if w2 == 0 { 65536 } else { w2 };

    let x2_over_w2 = (x * x) / w2_safe;
    let numerator = (x + x2_over_w2) * 65536;
    let denominator = x + 65536;

    let result = numerator / denominator;
    result.min(65536) as u32
}

/// Apply Academy Color Encoding System (ACES) filmic S-curve approximation:
/// $f(x) = x(2.51x + 0.03) / (x(2.43x + 0.59) + 0.14)$.
/// Computed entirely with fixed-point integer arithmetic in Q16.16.
pub fn apply_tonemap_aces(x_q16: u32) -> u32 {
    if x_q16 == 0 {
        return 0;
    }
    let x = x_q16 as u64;

    // ACES coefficients scaled to Q16.16:
    // a = 2.51 * 65536 = 164495
    // b = 0.03 * 65536 = 1966
    // c = 2.43 * 65536 = 159252
    // d = 0.59 * 65536 = 38666
    // e = 0.14 * 65536 = 9175
    let a: u64 = 164495;
    let b: u64 = 1966;
    let c: u64 = 159252;
    let d: u64 = 38666;
    let e: u64 = 9175;

    let ax_b = ((a * x) >> 16) + b;
    let num = (x * ax_b) >> 16;

    let cx_d = ((c * x) >> 16) + d;
    let den = ((x * cx_d) >> 16) + e;

    if den == 0 {
        return 0;
    }

    let result = (num << 16) / den;
    result.min(65536) as u32
}

/// Convert BT.2020 wide color gamut primaries to BT.709 (sRGB) primaries using 16.16 fixed-point matrix:
/// [R_709]   [ 1.6605  -0.5876  -0.0728 ] [R_2020]
/// [G_709] = [ -0.1246  1.1329  -0.0083 ] [G_2020]
/// [B_709]   [ -0.0182  -0.1006  1.1187 ] [B_2020]
pub fn apply_bt2020_to_bt709_gamut(r_q16: i32, g_q16: i32, b_q16: i32) -> (i32, i32, i32) {
    let r = r_q16 as i64;
    let g = g_q16 as i64;
    let b = b_q16 as i64;

    let r_out = (108823 * r - 38509 * g - 4778 * b) >> 16;
    let g_out = (-8166 * r + 74246 * g - 544 * b) >> 16;
    let b_out = (-1193 * r - 6593 * g + 73322 * b) >> 16;

    (r_out.max(0) as i32, g_out.max(0) as i32, b_out.max(0) as i32)
}

/// Perform highlight desaturation and gamut compression.
/// Prevents out-of-gamut color clipping and edge hue shifts in high-brightness areas.
pub fn apply_highlight_desaturation(
    r_q16: u32,
    g_q16: u32,
    b_q16: u32,
    saturation_q16: u32,
) -> (u32, u32, u32) {
    // Luminance approximation Y = 0.2126 R + 0.7152 G + 0.0722 B in Q8
    let y = ((54 * (r_q16 >> 8) + 183 * (g_q16 >> 8) + 19 * (b_q16 >> 8)) >> 8) << 8;

    // Above 75% of SDR white (49152 in Q16.16), smoothly desaturate towards paper-white
    let desat_threshold = 49152u32;
    let mut sat = saturation_q16 as u64;

    if y > desat_threshold {
        let excess = (y - desat_threshold) as u64;
        let factor = (excess << 16) / 16384; // 0..65536
        let roll_off = 65536 - factor.min(65536);
        sat = (sat * roll_off) >> 16;
    }

    let r_diff = (r_q16 as i64) - (y as i64);
    let g_diff = (g_q16 as i64) - (y as i64);
    let b_diff = (b_q16 as i64) - (y as i64);

    let r_final = ((y as i64) + ((r_diff * (sat as i64)) >> 16)).max(0).min(65536) as u32;
    let g_final = ((y as i64) + ((g_diff * (sat as i64)) >> 16)).max(0).min(65536) as u32;
    let b_final = ((y as i64) + ((b_diff * (sat as i64)) >> 16)).max(0).min(65536) as u32;

    (r_final, g_final, b_final)
}

/// Tonemap a single pixel with arbitrary linear input channels into 8-bit SDR RGB.
pub fn tonemap_pixel(
    r_lin_q16: u32,
    g_lin_q16: u32,
    b_lin_q16: u32,
    params: &TonemapParameters,
) -> (u8, u8, u8) {
    // 1. Apply exposure scaling
    let r_exp = ((r_lin_q16 as u64 * params.exposure_q16 as u64) >> 16) as u32;
    let g_exp = ((g_lin_q16 as u64 * params.exposure_q16 as u64) >> 16) as u32;
    let b_exp = ((b_lin_q16 as u64 * params.exposure_q16 as u64) >> 16) as u32;

    // 2. Wide color gamut compression from BT.2020 to BT.709 if enabled
    let (r_gamut, g_gamut, b_gamut) = if params.gamut_compress_enabled {
        let (r709, g709, b709) = apply_bt2020_to_bt709_gamut(r_exp as i32, g_exp as i32, b_exp as i32);
        (r709 as u32, g709 as u32, b709 as u32)
    } else {
        (r_exp, g_exp, b_exp)
    };

    // 3. Apply mathematical tonemapping curve
    let (r_tm, g_tm, b_tm) = match params.operator {
        TonemapOperator::LinearClamp => (
            r_gamut.min(65536),
            g_gamut.min(65536),
            b_gamut.min(65536),
        ),
        TonemapOperator::Reinhard => (
            apply_tonemap_reinhard(r_gamut, 65536),
            apply_tonemap_reinhard(g_gamut, 65536),
            apply_tonemap_reinhard(b_gamut, 65536),
        ),
        TonemapOperator::ReinhardExtended => (
            apply_tonemap_reinhard(r_gamut, params.white_point_q16),
            apply_tonemap_reinhard(g_gamut, params.white_point_q16),
            apply_tonemap_reinhard(b_gamut, params.white_point_q16),
        ),
        TonemapOperator::AcesFilmic => (
            apply_tonemap_aces(r_gamut),
            apply_tonemap_aces(g_gamut),
            apply_tonemap_aces(b_gamut),
        ),
    };

    // 4. Highlight desaturation & saturation modulation
    let (r_desat, g_desat, b_desat) = apply_highlight_desaturation(
        r_tm,
        g_tm,
        b_tm,
        params.saturation_q16,
    );

    // 5. Convert Q16.16 [0..65536] to 8-bit [0..255]
    let r_8 = ((r_desat as u64 * 255 + 32768) >> 16).min(255) as u8;
    let g_8 = ((g_desat as u64 * 255 + 32768) >> 16).min(255) as u8;
    let b_8 = ((b_desat as u64 * 255 + 32768) >> 16).min(255) as u8;

    (r_8, g_8, b_8)
}

/// Real-time kernel HDR tonemapper engine executing scanline-level processing.
pub struct HdrTonemapper {
    pub params: TonemapParameters,
}

impl HdrTonemapper {
    /// Initialize a new real-time tonemapper engine.
    pub fn new(params: TonemapParameters) -> Self {
        Self { params }
    }

    /// Update active tonemapping parameters.
    pub fn update_parameters(&mut self, params: TonemapParameters) {
        self.params = params;
    }

    /// Tonemap a single scanline of packed HDR10 R10G10B10A2 into SDR RGBA8888.
    pub fn tonemap_scanline_hdr10_to_rgba8(
        &self,
        src_hdr10: &[u8],
        dst_rgba8: &mut [u8],
        pixel_count: usize,
    ) {
        let src_bytes = pixel_count * 4;
        let dst_bytes = pixel_count * 4;
        if src_hdr10.len() < src_bytes || dst_rgba8.len() < dst_bytes {
            return;
        }

        for i in 0..pixel_count {
            let src_offset = i * 4;
            let packed = u32::from_le_bytes([
                src_hdr10[src_offset],
                src_hdr10[src_offset + 1],
                src_hdr10[src_offset + 2],
                src_hdr10[src_offset + 3],
            ]);

            let (r10, g10, b10, _a2) = unpack_r10g10b10a2(packed);
            let r_lin = hdr10_to_linear_q16(r10);
            let g_lin = hdr10_to_linear_q16(g10);
            let b_lin = hdr10_to_linear_q16(b10);

            let (r8, g8, b8) = tonemap_pixel(r_lin, g_lin, b_lin, &self.params);

            let dst_offset = i * 4;
            dst_rgba8[dst_offset] = r8;
            dst_rgba8[dst_offset + 1] = g8;
            dst_rgba8[dst_offset + 2] = b8;
            dst_rgba8[dst_offset + 3] = 255;
        }
    }

    /// Tonemap a single scanline of 64-bit scRGB FP16 linear pixels into SDR RGBA8888.
    pub fn tonemap_scanline_scrgb_to_rgba8(
        &self,
        src_scrgb: &[u8],
        dst_rgba8: &mut [u8],
        pixel_count: usize,
    ) {
        let src_bytes = pixel_count * 8;
        let dst_bytes = pixel_count * 4;
        if src_scrgb.len() < src_bytes || dst_rgba8.len() < dst_bytes {
            return;
        }

        for i in 0..pixel_count {
            let src_offset = i * 8;
            let r_half = u16::from_le_bytes([src_scrgb[src_offset], src_scrgb[src_offset + 1]]);
            let g_half = u16::from_le_bytes([src_scrgb[src_offset + 2], src_scrgb[src_offset + 3]]);
            let b_half = u16::from_le_bytes([src_scrgb[src_offset + 4], src_scrgb[src_offset + 5]]);

            let r_lin = scrgb_to_linear_q16(r_half);
            let g_lin = scrgb_to_linear_q16(g_half);
            let b_lin = scrgb_to_linear_q16(b_half);

            let (r8, g8, b8) = tonemap_pixel(r_lin, g_lin, b_lin, &self.params);

            let dst_offset = i * 4;
            dst_rgba8[dst_offset] = r8;
            dst_rgba8[dst_offset + 1] = g8;
            dst_rgba8[dst_offset + 2] = b8;
            dst_rgba8[dst_offset + 3] = 255;
        }
    }

    /// Tonemap a single scanline of HDR10 into compact 16-bit RGB565 (for bandwidth constrained mirrors).
    pub fn tonemap_scanline_hdr10_to_rgb565(
        &self,
        src_hdr10: &[u8],
        dst_rgb565: &mut [u8],
        pixel_count: usize,
    ) {
        let src_bytes = pixel_count * 4;
        let dst_bytes = pixel_count * 2;
        if src_hdr10.len() < src_bytes || dst_rgb565.len() < dst_bytes {
            return;
        }

        for i in 0..pixel_count {
            let src_offset = i * 4;
            let packed = u32::from_le_bytes([
                src_hdr10[src_offset],
                src_hdr10[src_offset + 1],
                src_hdr10[src_offset + 2],
                src_hdr10[src_offset + 3],
            ]);

            let (r10, g10, b10, _) = unpack_r10g10b10a2(packed);
            let (r8, g8, b8) = tonemap_pixel(
                hdr10_to_linear_q16(r10),
                hdr10_to_linear_q16(g10),
                hdr10_to_linear_q16(b10),
                &self.params,
            );

            let rgb565: u16 = (((r8 as u16) >> 3) << 11)
                | (((g8 as u16) >> 2) << 5)
                | ((b8 as u16) >> 3);

            let dst_offset = i * 2;
            let bytes = rgb565.to_le_bytes();
            dst_rgb565[dst_offset] = bytes[0];
            dst_rgb565[dst_offset + 1] = bytes[1];
        }
    }

    /// Tonemap a horizontal slice of scanlines between source surface and destination surface.
    pub fn tonemap_slice(
        &self,
        src: &[u8],
        dst: &mut [u8],
        src_stride: usize,
        dst_stride: usize,
        width: usize,
        start_line: usize,
        line_count: usize,
        src_format: HdrPixelFormat,
        dst_format: HdrPixelFormat,
    ) -> Result<usize, &'static str> {
        let mut bytes_written = 0;
        for line in 0..line_count {
            let row = start_line + line;
            let src_offset = row * src_stride;
            let dst_offset = line * dst_stride;

            if src_offset + src_stride > src.len() || dst_offset + dst_stride > dst.len() {
                return Err("Slice bounds exceed buffer size");
            }

            let src_row = &src[src_offset..src_offset + src_stride];
            let dst_row = &mut dst[dst_offset..dst_offset + dst_stride];

            match (src_format, dst_format) {
                (HdrPixelFormat::Hdr10R10G10B10A2, HdrPixelFormat::SdrRgba8888) => {
                    self.tonemap_scanline_hdr10_to_rgba8(src_row, dst_row, width);
                    bytes_written += width * 4;
                }
                (HdrPixelFormat::Hdr10R10G10B10A2, HdrPixelFormat::SdrRgb565) => {
                    self.tonemap_scanline_hdr10_to_rgb565(src_row, dst_row, width);
                    bytes_written += width * 2;
                }
                (HdrPixelFormat::ScRgbFp16, HdrPixelFormat::SdrRgba8888) => {
                    self.tonemap_scanline_scrgb_to_rgba8(src_row, dst_row, width);
                    bytes_written += width * 4;
                }
                _ => return Err("Unsupported format combination for tonemapper"),
            }
        }
        Ok(bytes_written)
    }
}


// -----------------------------------------------------------------------------
// Display Mirroring Network Packet Streamer & Scanline Chunker
// -----------------------------------------------------------------------------

/// Individual packet slice containing binary header and network payload.
#[derive(Debug, Clone)]
pub struct StreamChunk {
    /// 32-byte standardized packet header.
    pub header: MirrorStreamPacketHeader,
    /// Chunk payload bytes (up to `MAX_CHUNK_PAYLOAD_SIZE`).
    pub payload: Vec<u8>,
}

/// Congestion levels governing adaptive display mirroring streaming.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CongestionLevel {
    /// Nominal link performance (<25% queue depth, low latency). Full quality & frame rate.
    Nominal = 0,
    /// Mild network/USB buffering (25-50% queue depth). Modulate bitrate or color format.
    Mild = 1,
    /// Severe congestion (50-80% queue depth). Drop non-key frames and halve frame rate.
    Severe = 2,
    /// Critical backlog (>80% queue depth). Drop all delta frames; emit low-res recovery keyframes.
    Critical = 3,
}

/// Adaptive congestion and bitrate governor monitoring link backpressure.
pub struct AdaptiveCongestionGovernor {
    /// Current evaluated congestion state.
    pub level: CongestionLevel,
    /// Active transmission queue depth.
    pub tx_queue_depth: usize,
    /// Maximum transmission queue capacity.
    pub tx_queue_capacity: usize,
    /// Smoothed round-trip time in microseconds.
    pub smoothed_rtt_us: u32,
    /// Packet loss count reported by receiver feedback.
    pub packet_loss_count: u32,
    /// Total frames evaluated by the governor.
    pub frames_evaluated: u64,
    /// Total frames successfully transmitted.
    pub frames_transmitted: u64,
    /// Total frames dropped due to backpressure.
    pub frames_dropped: u64,
    /// Current target streaming bitrate in kilobits per second.
    pub target_bitrate_kbps: u32,
}

impl AdaptiveCongestionGovernor {
    /// Create a new adaptive congestion governor.
    pub fn new(tx_queue_capacity: usize, base_bitrate_kbps: u32) -> Self {
        Self {
            level: CongestionLevel::Nominal,
            tx_queue_depth: 0,
            tx_queue_capacity: if tx_queue_capacity == 0 { 64 } else { tx_queue_capacity },
            smoothed_rtt_us: 1000, // 1 ms initial estimate
            packet_loss_count: 0,
            frames_evaluated: 0,
            frames_transmitted: 0,
            frames_dropped: 0,
            target_bitrate_kbps: base_bitrate_kbps,
        }
    }

    /// Update network feedback statistics and modulate congestion state.
    pub fn update_network_stats(&mut self, queue_depth: usize, rtt_us: u32, packets_lost: u32) {
        self.tx_queue_depth = queue_depth;
        self.packet_loss_count = packets_lost;

        // Exponential moving average for RTT
        self.smoothed_rtt_us = (self.smoothed_rtt_us * 7 + rtt_us) / 8;

        let queue_ratio_pct = (queue_depth * 100) / self.tx_queue_capacity;

        if queue_ratio_pct > 80 || packets_lost > 10 || self.smoothed_rtt_us > 35_000 {
            self.level = CongestionLevel::Critical;
            self.target_bitrate_kbps = self.target_bitrate_kbps.saturating_mul(50) / 100;
        } else if queue_ratio_pct > 50 || packets_lost > 3 || self.smoothed_rtt_us > 20_000 {
            self.level = CongestionLevel::Severe;
            self.target_bitrate_kbps = self.target_bitrate_kbps.saturating_mul(75) / 100;
        } else if queue_ratio_pct > 25 || packets_lost > 0 || self.smoothed_rtt_us > 10_000 {
            self.level = CongestionLevel::Mild;
            self.target_bitrate_kbps = self.target_bitrate_kbps.saturating_mul(90) / 100;
        } else {
            self.level = CongestionLevel::Nominal;
            // Gradually recover bitrate up to maximum
            self.target_bitrate_kbps = (self.target_bitrate_kbps + 1000).min(50_000);
        }
    }

    /// Determine whether an incoming frame should be dropped due to backpressure.
    pub fn should_drop_frame(&mut self, is_keyframe: bool) -> bool {
        self.frames_evaluated = self.frames_evaluated.wrapping_add(1);

        let drop = match self.level {
            CongestionLevel::Nominal => false,
            CongestionLevel::Mild => {
                // Drop every 4th non-keyframe to relieve mild pressure
                !is_keyframe && (self.frames_evaluated % 4 == 0)
            }
            CongestionLevel::Severe => {
                // Drop every other non-keyframe (halve framerate)
                !is_keyframe && (self.frames_evaluated % 2 != 0)
            }
            CongestionLevel::Critical => {
                // In critical backlog, drop all non-keyframes
                !is_keyframe
            }
        };

        if drop {
            self.frames_dropped = self.frames_dropped.wrapping_add(1);
        } else {
            self.frames_transmitted = self.frames_transmitted.wrapping_add(1);
        }

        drop
    }

    /// Return the optimal pixel format recommended under current network congestion.
    pub fn recommended_pixel_format(&self) -> HdrPixelFormat {
        match self.level {
            CongestionLevel::Nominal => HdrPixelFormat::SdrRgba8888,
            CongestionLevel::Mild => HdrPixelFormat::SdrRgba8888,
            CongestionLevel::Severe | CongestionLevel::Critical => HdrPixelFormat::SdrRgb565,
        }
    }
}

/// Presentation Timestamp (PTS) synchronization engine for zero-stutter display mirrors.
pub struct PtsSyncGovernor {
    /// Host local clock baseline in microseconds.
    pub source_clock_base_us: u64,
    /// Clock offset between host and receiver in microseconds.
    pub target_clock_offset_us: i64,
    /// Smoothed RTT in microseconds.
    pub smoothed_rtt_us: u32,
    /// Maximum acceptable pipeline latency before a frame is considered stale.
    pub max_acceptable_latency_us: u32,
    /// PTS of the most recent presented frame.
    pub last_presented_pts_us: u64,
}

impl PtsSyncGovernor {
    /// Create a new PTS synchronization governor with maximum latency threshold.
    pub fn new(max_acceptable_latency_us: u32) -> Self {
        Self {
            source_clock_base_us: 0,
            target_clock_offset_us: 0,
            smoothed_rtt_us: 1000,
            max_acceptable_latency_us: if max_acceptable_latency_us == 0 {
                33_333 // 33.3 ms default threshold (~2 frames at 60 Hz)
            } else {
                max_acceptable_latency_us
            },
            last_presented_pts_us: 0,
        }
    }

    /// Process round-trip time and clock synchronization feedback packet from the receiver.
    pub fn record_clock_sync(&mut self, source_pts_us: u64, receiver_pts_us: u64, rtt_us: u32) {
        self.smoothed_rtt_us = (self.smoothed_rtt_us * 3 + rtt_us) / 4;
        let one_way_delay = (self.smoothed_rtt_us / 2) as i64;
        let estimated_remote_at_source = (receiver_pts_us as i64) - one_way_delay;
        self.target_clock_offset_us = estimated_remote_at_source - (source_pts_us as i64);
    }

    /// Check if a captured frame's PTS is already stale relative to current presentation clock.
    pub fn is_frame_stale(&self, frame_pts_us: u64, current_clock_us: u64) -> bool {
        if frame_pts_us < self.last_presented_pts_us {
            return true;
        }
        if current_clock_us > frame_pts_us {
            let latency = current_clock_us - frame_pts_us;
            if latency > (self.max_acceptable_latency_us as u64) {
                return true;
            }
        }
        false
    }

    /// Compute synchronized target presentation timestamp for receiver playback.
    pub fn compute_target_pts(&mut self, frame_pts_us: u64) -> u64 {
        self.last_presented_pts_us = frame_pts_us;
        let offset = self.target_clock_offset_us;
        let target = (frame_pts_us as i64) + offset;
        if target > 0 { target as u64 } else { frame_pts_us }
    }
}

/// Scanline Chunker breaking tonemapped scanlines into MTU-sized streaming packets.
pub struct ScanlineChunker {
    /// Maximum payload bytes per chunk packet.
    pub max_payload_size: usize,
}

impl ScanlineChunker {
    /// Construct a scanline chunker with specified MTU payload boundary.
    pub fn new(max_payload_size: usize) -> Self {
        Self {
            max_payload_size: if max_payload_size == 0 {
                MAX_CHUNK_PAYLOAD_SIZE
            } else {
                max_payload_size
            },
        }
    }

    /// Slice a tonemapped pixel buffer into an array of streaming chunks.
    pub fn chunk_frame(
        &self,
        frame_id: u32,
        pts_us: u64,
        is_keyframe: bool,
        tonemapped_buffer: &[u8],
        width: usize,
        height: usize,
        format: HdrPixelFormat,
    ) -> Vec<StreamChunk> {
        let bpp = format.bytes_per_pixel();
        let stride = width * bpp;
        if stride == 0 || height == 0 || tonemapped_buffer.is_empty() {
            return Vec::new();
        }

        let total_bytes = tonemapped_buffer.len();
        let chunk_count = (total_bytes + self.max_payload_size - 1) / self.max_payload_size;
        let mut chunks = Vec::with_capacity(chunk_count);

        for chunk_idx in 0..chunk_count {
            let offset = chunk_idx * self.max_payload_size;
            let end = (offset + self.max_payload_size).min(total_bytes);
            let payload = tonemapped_buffer[offset..end].to_vec();

            let start_line = (offset / stride).min(height - 1) as u16;
            let end_line = ((end + stride - 1) / stride).min(height) as u16;
            let line_count = (end_line - start_line).max(1);

            let mut flags = PACKET_FLAG_TONEMAPPED;
            if is_keyframe {
                flags |= PACKET_FLAG_KEYFRAME;
            }
            if chunk_idx + 1 == chunk_count {
                flags |= PACKET_FLAG_LAST_CHUNK;
            }

            let mut header = MirrorStreamPacketHeader::new(
                StreamPacketType::ScanlineChunk,
                flags,
                frame_id,
                chunk_idx as u16,
                chunk_count as u16,
                start_line,
                line_count,
                pts_us,
                payload.len() as u16,
            );
            header.crc16 = MirrorStreamPacketHeader::compute_crc16(&payload);

            chunks.push(StreamChunk { header, payload });
        }

        chunks
    }
}

/// End-to-end Display Mirroring Streamer Pipeline coordinating capture, tonemapping, chunking, and transmission.
pub struct DisplayMirrorStreamer {
    /// Zero-copy compositor frame capture ring buffer.
    pub capture_ring: FrameCaptureRing,
    /// Fixed-point HDR tonemapper and gamut compressor.
    pub tonemapper: HdrTonemapper,
    /// Presentation timestamp synchronizer.
    pub pts_governor: PtsSyncGovernor,
    /// Adaptive network/USB congestion governor.
    pub congestion_governor: AdaptiveCongestionGovernor,
    /// Scanline slicing and packetization engine.
    pub chunker: ScanlineChunker,
    /// Transmission output ring queue.
    pub tx_queue: Vec<StreamChunk>,
    /// Capacity limit for transmission queue.
    pub tx_queue_capacity: usize,
    /// Keyframe interval counter.
    pub keyframe_interval: u32,
}

impl DisplayMirrorStreamer {
    /// Construct a new end-to-end display mirroring streamer.
    pub fn new(
        slot_count: usize,
        slot_bytes: usize,
        tonemap_params: TonemapParameters,
        tx_queue_capacity: usize,
    ) -> Self {
        Self {
            capture_ring: FrameCaptureRing::new(slot_count, slot_bytes),
            tonemapper: HdrTonemapper::new(tonemap_params),
            pts_governor: PtsSyncGovernor::new(33_333),
            congestion_governor: AdaptiveCongestionGovernor::new(tx_queue_capacity, 25_000),
            chunker: ScanlineChunker::new(MAX_CHUNK_PAYLOAD_SIZE),
            tx_queue: Vec::with_capacity(tx_queue_capacity),
            tx_queue_capacity,
            keyframe_interval: 30, // Every 30 frames
        }
    }

    /// Process a completed capture slot: tonemap HDR to SDR, chunk scanlines, and enqueue streaming packets.
    pub fn process_capture_slot(
        &mut self,
        slot_id: u8,
        current_clock_us: u64,
    ) -> Result<usize, &'static str> {
        let idx = slot_id as usize;
        if idx >= self.capture_ring.slots.len() {
            return Err("Invalid slot_id");
        }

        let slot = &self.capture_ring.slots[idx];
        if slot.state != CaptureSlotState::Ready {
            return Err("Capture slot is not in Ready state");
        }

        let frame_id = slot.frame_id;
        let pts_us = slot.pts_us;
        let width = slot.width as usize;
        let height = slot.height as usize;
        let src_format = slot.format;
        let is_keyframe = (frame_id % self.keyframe_interval) == 0;

        // 1. Check if frame is stale based on presentation timeline
        if self.pts_governor.is_frame_stale(pts_us, current_clock_us) {
            let _ = self.capture_ring.release_frame(slot_id);
            self.congestion_governor.frames_dropped += 1;
            return Err("Frame dropped: PTS is stale");
        }

        // 2. Query congestion governor for frame dropping
        if self.congestion_governor.should_drop_frame(is_keyframe) {
            let _ = self.capture_ring.release_frame(slot_id);
            return Ok(0);
        }

        // 3. Determine target SDR format based on link congestion
        let dst_format = self.congestion_governor.recommended_pixel_format();
        let dst_stride = width * dst_format.bytes_per_pixel();
        let dst_total_bytes = dst_stride * height;
        let mut tonemapped_buf = alloc::vec![0u8; dst_total_bytes];

        // 4. Execute fixed-point tonemapping across scanlines
        let src_stride = slot.stride_bytes as usize;
        self.tonemapper.tonemap_slice(
            &slot.buffer,
            &mut tonemapped_buf,
            src_stride,
            dst_stride,
            width,
            0,
            height,
            src_format,
            dst_format,
        )?;

        // 5. Compute target presentation timestamp with clock sync
        let target_pts = self.pts_governor.compute_target_pts(pts_us);

        // 6. Slice tonemapped scanlines into packet chunks
        let chunks = self.chunker.chunk_frame(
            frame_id,
            target_pts,
            is_keyframe,
            &tonemapped_buf,
            width,
            height,
            dst_format,
        );

        let chunks_count = chunks.len();

        // 7. Enqueue chunks into transmission queue
        for chunk in chunks {
            if self.tx_queue.len() < self.tx_queue_capacity {
                self.tx_queue.push(chunk);
            } else {
                // Queue overflow: drop packet
                break;
            }
        }

        // 8. Release capture slot back to ring
        self.capture_ring.release_frame(slot_id)?;

        Ok(chunks_count)
    }

    /// Poll up to `max_packets` from the transmission queue for hardware MAC or USB submission.
    pub fn poll_transmit_packets(&mut self, max_packets: usize) -> Vec<StreamChunk> {
        let count = max_packets.min(self.tx_queue.len());
        self.tx_queue.drain(0..count).collect()
    }

    /// Handle congestion feedback packet returned from the receiver.
    pub fn receive_feedback(
        &mut self,
        source_pts_us: u64,
        receiver_pts_us: u64,
        rtt_us: u32,
        packets_lost: u32,
    ) {
        self.pts_governor.record_clock_sync(source_pts_us, receiver_pts_us, rtt_us);
        let q_depth = self.tx_queue.len();
        self.congestion_governor.update_network_stats(q_depth, rtt_us, packets_lost);
    }
}
