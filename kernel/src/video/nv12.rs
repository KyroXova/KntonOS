//! NV12 / P010 Tiled Video Frame Buffer Allocator & Decoded Picture Buffer (DPB) Ring
//! and Zero-Copy Direct Scanout Presentation Streamer.
//!
//! Implements GPU video surface memory layout, reference frame tracking, and presentation:
//! - Bi-planar video surface layout: NV12 (8-bit SDR) and P010 (10-bit HDR10).
//! - Hardware pitch alignment (128-byte and 256-byte cache-line row strides).
//! - Linear and micro-tiled (Block-Linear) memory coordinate transformations.
//! - Decoded Picture Buffer (DPB) reference frame ring with short-term / long-term picture marking and LRU eviction.
//! - Fast fixed-point BT.709 (SDR) and BT.2020 (HDR10) color matrix conversion.
//! - Zero-copy direct display scanout overlay pipeline with atomic VSYNC flipping and frame latency metrics.

#![allow(dead_code)]

extern crate alloc;
use alloc::vec::Vec;

// -----------------------------------------------------------------------------
// Video Surface Formats & Memory Layouts
// -----------------------------------------------------------------------------

/// Bi-planar video pixel formats.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoSurfaceFormat {
    /// 8-bit Y plane followed by interleaved UV plane (4:2:0 subsampled).
    Nv12 = 1,
    /// 10-bit Y plane followed by interleaved UV plane (4:2:0 subsampled, 16 bits per sample).
    P010 = 2,
}

impl VideoSurfaceFormat {
    #[inline]
    pub fn bytes_per_sample(self) -> usize {
        match self {
            VideoSurfaceFormat::Nv12 => 1,
            VideoSurfaceFormat::P010 => 2,
        }
    }

    #[inline]
    pub fn bits_per_sample(self) -> usize {
        match self {
            VideoSurfaceFormat::Nv12 => 8,
            VideoSurfaceFormat::P010 => 10,
        }
    }
}

/// GPU memory tiling configurations.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoSurfaceTiling {
    /// Contiguous row-major scanline linear memory.
    Linear = 0,
    /// 16x16 micro-tiled layout (GPU hardware texture sampler friendly).
    Tile16x16 = 1,
}

/// Color matrix standard specifications.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorStandard {
    /// SDTV standard (Rec. ITU-R BT.601).
    Bt601 = 1,
    /// HDTV standard (Rec. ITU-R BT.709).
    Bt709 = 2,
    /// UHDTV / HDR10 standard (Rec. ITU-R BT.2020).
    Bt2020 = 3,
}

/// Color dynamic range quantification.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorRange {
    /// Limited / Studio range (Y: 16-235, UV: 16-240 for 8-bit; Y: 64-940, UV: 64-960 for 10-bit).
    Limited = 1,
    /// Full / PC range (0-255 for 8-bit, 0-1023 for 10-bit).
    Full = 2,
}

// -----------------------------------------------------------------------------
// Video Surface Descriptor & Buffer
// -----------------------------------------------------------------------------

/// Individual decoded video frame buffer holding Y and UV planes.
pub struct VideoSurface {
    pub surface_id: u16,
    pub format: VideoSurfaceFormat,
    pub tiling: VideoSurfaceTiling,
    pub width: usize,
    pub height: usize,
    pub luma_pitch: usize,
    pub chroma_pitch: usize,
    pub luma_offset: usize,
    pub chroma_offset: usize,
    pub total_size: usize,
    pub memory: Vec<u8>,
}

impl VideoSurface {
    /// Allocate a new video frame surface with pitch alignment and proper plane offsets.
    pub fn new(
        surface_id: u16,
        format: VideoSurfaceFormat,
        tiling: VideoSurfaceTiling,
        width: usize,
        height: usize,
        pitch_alignment: usize,
    ) -> Result<Self, &'static str> {
        if width == 0 || height == 0 {
            return Err("Zero dimensions invalid");
        }
        if pitch_alignment == 0 || (pitch_alignment & (pitch_alignment - 1)) != 0 {
            return Err("Pitch alignment must be a power of two");
        }

        let bytes_per_pixel = format.bytes_per_sample();
        let unaligned_luma_pitch = width * bytes_per_pixel;
        let luma_pitch = (unaligned_luma_pitch + pitch_alignment - 1) & !(pitch_alignment - 1);
        let chroma_pitch = luma_pitch; // UV interleaved has same byte width per line

        let luma_height = height;
        let chroma_height = (height + 1) / 2;

        let luma_size = luma_pitch * luma_height;
        let chroma_size = chroma_pitch * chroma_height;

        let luma_offset = 0;
        let chroma_offset = luma_size;
        let total_size = luma_size + chroma_size;

        let mut memory = Vec::new();
        memory.resize(total_size, 0);

        Ok(Self {
            surface_id,
            format,
            tiling,
            width,
            height,
            luma_pitch,
            chroma_pitch,
            luma_offset,
            chroma_offset,
            total_size,
            memory,
        })
    }

    /// Compute byte offset of luma pixel (x, y).
    #[inline]
    pub fn luma_pixel_offset(&self, x: usize, y: usize) -> usize {
        match self.tiling {
            VideoSurfaceTiling::Linear => {
                let bytes = self.format.bytes_per_sample();
                self.luma_offset + y * self.luma_pitch + x * bytes
            }
            VideoSurfaceTiling::Tile16x16 => {
                let bytes = self.format.bytes_per_sample();
                let tile_x = x / 16;
                let tile_y = y / 16;
                let in_tile_x = x % 16;
                let in_tile_y = y % 16;
                let tiles_per_row = (self.width + 15) / 16;
                let tile_idx = tile_y * tiles_per_row + tile_x;
                let tile_size = 16 * 16 * bytes;
                self.luma_offset + tile_idx * tile_size + (in_tile_y * 16 + in_tile_x) * bytes
            }
        }
    }

    /// Compute byte offset of chroma UV sample pair (x_half, y_half).
    #[inline]
    pub fn chroma_pixel_offset(&self, x_half: usize, y_half: usize) -> (usize, usize) {
        match self.tiling {
            VideoSurfaceTiling::Linear => {
                let bytes = self.format.bytes_per_sample();
                let base = self.chroma_offset + y_half * self.chroma_pitch + x_half * (bytes * 2);
                (base, base + bytes)
            }
            VideoSurfaceTiling::Tile16x16 => {
                let bytes = self.format.bytes_per_sample();
                let uv_bytes = bytes * 2;
                let tile_x = x_half / 8;
                let tile_y = y_half / 8;
                let in_tile_x = x_half % 8;
                let in_tile_y = y_half % 8;
                let tiles_per_row = ((self.width / 2) + 7) / 8;
                let tile_idx = tile_y * tiles_per_row + tile_x;
                let tile_size = 8 * 8 * uv_bytes;
                let base = self.chroma_offset + tile_idx * tile_size + (in_tile_y * 8 + in_tile_x) * uv_bytes;
                (base, base + bytes)
            }
        }
    }

    /// Read an 8-bit or 10-bit Y sample at (x, y).
    pub fn read_luma(&self, x: usize, y: usize) -> u16 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        let offset = self.luma_pixel_offset(x, y);
        match self.format {
            VideoSurfaceFormat::Nv12 => self.memory[offset] as u16,
            VideoSurfaceFormat::P010 => {
                let low = self.memory[offset] as u16;
                let high = self.memory[offset + 1] as u16;
                let sample_16 = (high << 8) | low;
                sample_16 >> 6 // 10-bit MSB aligned
            }
        }
    }

    /// Write an 8-bit or 10-bit Y sample at (x, y).
    pub fn write_luma(&mut self, x: usize, y: usize, val: u16) {
        if x >= self.width || y >= self.height {
            return;
        }
        let offset = self.luma_pixel_offset(x, y);
        match self.format {
            VideoSurfaceFormat::Nv12 => {
                self.memory[offset] = (val & 0xFF) as u8;
            }
            VideoSurfaceFormat::P010 => {
                let sample_16 = (val & 0x03FF) << 6;
                self.memory[offset] = (sample_16 & 0xFF) as u8;
                self.memory[offset + 1] = ((sample_16 >> 8) & 0xFF) as u8;
            }
        }
    }

    /// Read chroma U and V samples for a given 2x2 luma block.
    pub fn read_chroma(&self, x_half: usize, y_half: usize) -> (u16, u16) {
        if x_half >= (self.width + 1) / 2 || y_half >= (self.height + 1) / 2 {
            return (128, 128);
        }
        let (u_off, v_off) = self.chroma_pixel_offset(x_half, y_half);
        match self.format {
            VideoSurfaceFormat::Nv12 => (
                self.memory[u_off] as u16,
                self.memory[v_off] as u16,
            ),
            VideoSurfaceFormat::P010 => {
                let u_raw = (self.memory[u_off] as u16) | ((self.memory[u_off + 1] as u16) << 8);
                let v_raw = (self.memory[v_off] as u16) | ((self.memory[v_off + 1] as u16) << 8);
                (u_raw >> 6, v_raw >> 6)
            }
        }
    }

    /// Write chroma U and V samples for a given 2x2 luma block.
    pub fn write_chroma(&mut self, x_half: usize, y_half: usize, u_val: u16, v_val: u16) {
        if x_half >= (self.width + 1) / 2 || y_half >= (self.height + 1) / 2 {
            return;
        }
        let (u_off, v_off) = self.chroma_pixel_offset(x_half, y_half);
        match self.format {
            VideoSurfaceFormat::Nv12 => {
                self.memory[u_off] = (u_val & 0xFF) as u8;
                self.memory[v_off] = (v_val & 0xFF) as u8;
            }
            VideoSurfaceFormat::P010 => {
                let u_raw = (u_val & 0x03FF) << 6;
                let v_raw = (v_val & 0x03FF) << 6;
                self.memory[u_off] = (u_raw & 0xFF) as u8;
                self.memory[u_off + 1] = ((u_raw >> 8) & 0xFF) as u8;
                self.memory[v_off] = (v_raw & 0xFF) as u8;
                self.memory[v_off + 1] = ((v_raw >> 8) & 0xFF) as u8;
            }
        }
    }
}

// -----------------------------------------------------------------------------
// Fixed-Point Color Matrix Conversion (BT.709 SDR & BT.2020 HDR)
// -----------------------------------------------------------------------------

/// Convert 8-bit NV12 Y, U, V values to 8-bit RGB `(R, G, B)`.
/// Uses 16.16 fixed-point arithmetic for zero-copy kernel execution without floating point.
pub fn nv12_to_rgb(y: u8, u: u8, v: u8, standard: ColorStandard, range: ColorRange) -> (u8, u8, u8) {
    let (c, d, e) = match range {
        ColorRange::Limited => {
            let y_adj = (y as i32) - 16;
            let u_adj = (u as i32) - 128;
            let v_adj = (v as i32) - 128;
            (y_adj, u_adj, v_adj)
        }
        ColorRange::Full => {
            let y_adj = y as i32;
            let u_adj = (u as i32) - 128;
            let v_adj = (v as i32) - 128;
            (y_adj, u_adj, v_adj)
        }
    };

    let (r_val, g_val, b_val) = match standard {
        ColorStandard::Bt709 => {
            // Rec.709 coefficients in 16.16 fixed-point:
            // R = 1.164383 * C + 1.792741 * E
            // G = 1.164383 * C - 0.213249 * D - 0.532909 * E
            // B = 1.164383 * C + 2.112402 * D
            let c_fixed = c * 76309;
            let r = (c_fixed + 117489 * e + 32768) >> 16;
            let g = (c_fixed - 13975 * d - 34925 * e + 32768) >> 16;
            let b = (c_fixed + 138438 * d + 32768) >> 16;
            (r, g, b)
        }
        ColorStandard::Bt601 => {
            // Rec.601 coefficients in 16.16 fixed-point:
            // R = 1.164383 * C + 1.596027 * E
            // G = 1.164383 * C - 0.391762 * D - 0.812968 * E
            // B = 1.164383 * C + 2.017232 * D
            let c_fixed = c * 76309;
            let r = (c_fixed + 104597 * e + 32768) >> 16;
            let g = (c_fixed - 25675 * d - 53279 * e + 32768) >> 16;
            let b = (c_fixed + 132201 * d + 32768) >> 16;
            (r, g, b)
        }
        ColorStandard::Bt2020 => {
            // Rec.2020 SDR coefficients in 16.16 fixed-point:
            // R = 1.164383 * C + 1.678670 * E
            // G = 1.164383 * C - 0.187326 * D - 0.650420 * E
            // B = 1.164383 * C + 2.141770 * D
            let c_fixed = c * 76309;
            let r = (c_fixed + 110013 * e + 32768) >> 16;
            let g = (c_fixed - 12277 * d - 42626 * e + 32768) >> 16;
            let b = (c_fixed + 140363 * d + 32768) >> 16;
            (r, g, b)
        }
    };

    let r_clamped = r_val.clamp(0, 255) as u8;
    let g_clamped = g_val.clamp(0, 255) as u8;
    let b_clamped = b_val.clamp(0, 255) as u8;

    (r_clamped, g_clamped, b_clamped)
}

/// Convert 10-bit P010 Y, U, V values (0 to 1023) to 8-bit RGB `(R, G, B)` with tone-curve clamping.
pub fn p010_to_rgb(y: u16, u: u16, v: u16, standard: ColorStandard, range: ColorRange) -> (u8, u8, u8) {
    let (c, d, e) = match range {
        ColorRange::Limited => {
            let y_adj = (y as i32) - 64;   // 10-bit limited black level
            let u_adj = (u as i32) - 512;  // 10-bit neutral chroma
            let v_adj = (v as i32) - 512;
            (y_adj, u_adj, v_adj)
        }
        ColorRange::Full => {
            let y_adj = y as i32;
            let u_adj = (u as i32) - 512;
            let v_adj = (v as i32) - 512;
            (y_adj, u_adj, v_adj)
        }
    };

    let (r_10, g_10, b_10) = match standard {
        ColorStandard::Bt2020 => {
            // Rec.2020 in 16.16 fixed point:
            let c_fixed = c * 76309;
            let r = (c_fixed + 110013 * e + 32768) >> 16;
            let g = (c_fixed - 12277 * d - 42626 * e + 32768) >> 16;
            let b = (c_fixed + 140363 * d + 32768) >> 16;
            (r, g, b)
        }
        _ => {
            // BT.709 fallback
            let c_fixed = c * 76309;
            let r = (c_fixed + 117489 * e + 32768) >> 16;
            let g = (c_fixed - 13975 * d - 34925 * e + 32768) >> 16;
            let b = (c_fixed + 138438 * d + 32768) >> 16;
            (r, g, b)
        }
    };

    // Scale 10-bit (0-1020 limited peak) down to 8-bit (0-255)
    let r_scaled = (r_10 * 255 + 510) / 1020;
    let g_scaled = (g_10 * 255 + 510) / 1020;
    let b_scaled = (b_10 * 255 + 510) / 1020;

    let r_out = r_scaled.clamp(0, 255) as u8;
    let g_out = g_scaled.clamp(0, 255) as u8;
    let b_out = b_scaled.clamp(0, 255) as u8;

    (r_out, g_out, b_out)
}

// -----------------------------------------------------------------------------
// Direct Scanout Overlay Plane & Presentation Pipeline
// -----------------------------------------------------------------------------

/// Hardware display overlay plane configuration for zero-copy scanout.
#[derive(Debug, Clone, Copy)]
pub struct VideoOverlayPlane {
    pub dst_x: usize,
    pub dst_y: usize,
    pub dst_width: usize,
    pub dst_height: usize,
    pub src_x: usize,
    pub src_y: usize,
    pub src_width: usize,
    pub src_height: usize,
    pub standard: ColorStandard,
    pub range: ColorRange,
    pub is_enabled: bool,
    pub active_surface_id: Option<u16>,
}

impl VideoOverlayPlane {
    pub const fn new() -> Self {
        Self {
            dst_x: 0,
            dst_y: 0,
            dst_width: 1920,
            dst_height: 1080,
            src_x: 0,
            src_y: 0,
            src_width: 1920,
            src_height: 1080,
            standard: ColorStandard::Bt709,
            range: ColorRange::Limited,
            is_enabled: false,
            active_surface_id: None,
        }
    }

    /// Configure viewport destination and source scaling bounds.
    pub fn configure_viewport(
        &mut self,
        dst_x: usize,
        dst_y: usize,
        dst_w: usize,
        dst_h: usize,
        src_w: usize,
        src_h: usize,
    ) {
        self.dst_x = dst_x;
        self.dst_y = dst_y;
        self.dst_width = dst_w;
        self.dst_height = dst_h;
        self.src_x = 0;
        self.src_y = 0;
        self.src_width = src_w;
        self.src_height = src_h;
        self.is_enabled = true;
    }
}

/// Queued frame entry awaiting presentation at next VSYNC.
#[derive(Debug, Clone, Copy)]
pub struct QueuedPresentation {
    pub surface_id: u16,
    pub pts_us: u64,
    pub enqueue_time_us: u64,
}

pub const MAX_PRESENTATION_QUEUE: usize = 8;

/// Presentation engine managing zero-copy hardware flips and compositor fallback rendering.
pub struct PresentationPipeline {
    overlay: VideoOverlayPlane,
    queue: [Option<QueuedPresentation>; MAX_PRESENTATION_QUEUE],
    queue_head: usize,
    queue_tail: usize,
    queue_count: usize,
    presented_frames: u64,
    dropped_frames: u64,
    last_pts_us: u64,
    last_flip_timestamp_us: u64,
    total_latency_accum_us: u64,
}

impl PresentationPipeline {
    pub const fn new() -> Self {
        Self {
            overlay: VideoOverlayPlane::new(),
            queue: [None; MAX_PRESENTATION_QUEUE],
            queue_head: 0,
            queue_tail: 0,
            queue_count: 0,
            presented_frames: 0,
            dropped_frames: 0,
            last_pts_us: 0,
            last_flip_timestamp_us: 0,
            total_latency_accum_us: 0,
        }
    }

    #[inline]
    pub fn overlay(&self) -> &VideoOverlayPlane {
        &self.overlay
    }

    #[inline]
    pub fn overlay_mut(&mut self) -> &mut VideoOverlayPlane {
        &mut self.overlay
    }

    #[inline]
    pub fn presented_frames(&self) -> u64 {
        self.presented_frames
    }

    #[inline]
    pub fn dropped_frames(&self) -> u64 {
        self.dropped_frames
    }

    #[inline]
    pub fn last_pts(&self) -> u64 {
        self.last_pts_us
    }

    #[inline]
    pub fn active_surface_id(&self) -> Option<u16> {
        self.overlay.active_surface_id
    }

    /// Average presentation latency from decode queue insertion to VSYNC presentation.
    pub fn average_latency_us(&self) -> u32 {
        if self.presented_frames == 0 {
            0
        } else {
            (self.total_latency_accum_us / self.presented_frames) as u32
        }
    }

    /// Queue decoded surface for presentation on next display scanout refresh.
    pub fn queue_surface(&mut self, surface_id: u16, pts_us: u64, current_time_us: u64) -> Result<(), &'static str> {
        if self.queue_count >= MAX_PRESENTATION_QUEUE {
            // Queue overrun: drop oldest frame to maintain real-time pacing
            self.queue[self.queue_head] = None;
            self.queue_head = (self.queue_head + 1) % MAX_PRESENTATION_QUEUE;
            self.queue_count -= 1;
            self.dropped_frames += 1;
        }

        self.queue[self.queue_tail] = Some(QueuedPresentation {
            surface_id,
            pts_us,
            enqueue_time_us: current_time_us,
        });
        self.queue_tail = (self.queue_tail + 1) % MAX_PRESENTATION_QUEUE;
        self.queue_count += 1;

        Ok(())
    }

    /// Trigger hardware zero-copy display flip upon VSYNC interrupt.
    /// Updates the scanout hardware surface address register atomically without copying.
    pub fn on_vsync_flip(&mut self, vsync_timestamp_us: u64) -> Option<u16> {
        if self.queue_count == 0 {
            return self.overlay.active_surface_id;
        }

        let queued = self.queue[self.queue_head].take()?;
        self.queue_head = (self.queue_head + 1) % MAX_PRESENTATION_QUEUE;
        self.queue_count -= 1;

        // Atomic presentation flip
        self.overlay.active_surface_id = Some(queued.surface_id);
        self.last_pts_us = queued.pts_us;
        self.presented_frames += 1;

        let latency = vsync_timestamp_us.saturating_sub(queued.enqueue_time_us);
        self.total_latency_accum_us += latency;
        self.last_flip_timestamp_us = vsync_timestamp_us;

        Some(queued.surface_id)
    }

    /// Software direct scanout rasterizer: renders video surface into linear framebuffer
    /// using nearest-neighbor scaling and hardware color matrix conversion.
    pub fn rasterize_to_framebuffer(
        &self,
        surface: &VideoSurface,
        fb_base: *mut u32,
        fb_stride: usize,
        fb_width: usize,
        fb_height: usize,
    ) {
        if !self.overlay.is_enabled || fb_base.is_null() {
            return;
        }

        let dst_x = self.overlay.dst_x;
        let dst_y = self.overlay.dst_y;
        let dst_w = self.overlay.dst_width.min(fb_width.saturating_sub(dst_x));
        let dst_h = self.overlay.dst_height.min(fb_height.saturating_sub(dst_y));

        let src_w = self.overlay.src_width;
        let src_h = self.overlay.src_height;

        if dst_w == 0 || dst_h == 0 || src_w == 0 || src_h == 0 {
            return;
        }

        for dy in 0..dst_h {
            let sy = (dy * src_h) / dst_h;
            let row_ptr = unsafe { fb_base.add((dst_y + dy) * fb_stride + dst_x) };

            for dx in 0..dst_w {
                let sx = (dx * src_w) / dst_w;

                let (r, g, b) = match surface.format {
                    VideoSurfaceFormat::Nv12 => {
                        let y_val = surface.read_luma(sx, sy) as u8;
                        let (u_val, v_val) = surface.read_chroma(sx / 2, sy / 2);
                        nv12_to_rgb(y_val, u_val as u8, v_val as u8, self.overlay.standard, self.overlay.range)
                    }
                    VideoSurfaceFormat::P010 => {
                        let y_val = surface.read_luma(sx, sy);
                        let (u_val, v_val) = surface.read_chroma(sx / 2, sy / 2);
                        p010_to_rgb(y_val, u_val, v_val, self.overlay.standard, self.overlay.range)
                    }
                };

                // Format as 32-bit ARGB/XRGB (0xFF_RR_GG_BB)
                let pixel = (0xFF << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
                unsafe {
                    row_ptr.add(dx).write_volatile(pixel);
                }
            }
        }
    }
}

// -----------------------------------------------------------------------------
// Decoded Picture Buffer (DPB) Reference Frame Ring
// -----------------------------------------------------------------------------

/// Reference frame status within the Decoded Picture Buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceState {
    Unused,
    InDecode,
    ShortTermReference { poc: i32, frame_num: u32 },
    LongTermReference { long_term_idx: u32 },
    DisplayPending,
}

/// Slot in the DPB holding surface reference metadata.
#[derive(Debug, Clone, Copy)]
pub struct DpbSlot {
    pub surface_id: u16,
    pub state: ReferenceState,
    pub poc: i32,
    pub age: u64,
}

impl DpbSlot {
    pub const fn empty() -> Self {
        Self {
            surface_id: 0,
            state: ReferenceState::Unused,
            poc: 0,
            age: 0,
        }
    }
}

pub const MAX_DPB_SURFACES: usize = 16;

/// Decoded video surface manager and DPB reference frame ring.
pub struct VideoSurfaceManager {
    surfaces: [Option<VideoSurface>; MAX_DPB_SURFACES],
    dpb_slots: [DpbSlot; MAX_DPB_SURFACES],
    max_active_references: usize,
    global_age_counter: u64,
}

impl VideoSurfaceManager {
    pub const fn new(max_active_references: usize) -> Self {
        const EMPTY_SLOT: DpbSlot = DpbSlot::empty();
        Self {
            surfaces: [
                None, None, None, None,
                None, None, None, None,
                None, None, None, None,
                None, None, None, None,
            ],
            dpb_slots: [EMPTY_SLOT; MAX_DPB_SURFACES],
            max_active_references,
            global_age_counter: 1,
        }
    }

    /// Allocate or retrieve a free surface slot and initialize the underlying frame buffer.
    pub fn allocate_surface(
        &mut self,
        width: usize,
        height: usize,
        format: VideoSurfaceFormat,
        tiling: VideoSurfaceTiling,
        pitch_alignment: usize,
    ) -> Result<u16, &'static str> {
        // Find an empty or unused slot
        let mut target_idx = None;
        for i in 0..MAX_DPB_SURFACES {
            if self.dpb_slots[i].state == ReferenceState::Unused {
                target_idx = Some(i);
                break;
            }
        }

        // If no slot is unused, evict the oldest short-term reference
        let slot_idx = match target_idx {
            Some(idx) => idx,
            None => self.evict_oldest_short_term_reference().ok_or("DPB full: no recyclable surfaces")?,
        };

        let surface_id = slot_idx as u16;
        let surface = VideoSurface::new(
            surface_id,
            format,
            tiling,
            width,
            height,
            pitch_alignment,
        )?;

        self.surfaces[slot_idx] = Some(surface);
        self.dpb_slots[slot_idx] = DpbSlot {
            surface_id,
            state: ReferenceState::InDecode,
            poc: 0,
            age: self.global_age_counter,
        };
        self.global_age_counter += 1;

        Ok(surface_id)
    }

    /// Get immutable reference to video surface.
    #[inline]
    pub fn get_surface(&self, surface_id: u16) -> Option<&VideoSurface> {
        let idx = surface_id as usize;
        if idx < MAX_DPB_SURFACES {
            self.surfaces[idx].as_ref()
        } else {
            None
        }
    }

    /// Get mutable reference to video surface.
    #[inline]
    pub fn get_surface_mut(&mut self, surface_id: u16) -> Option<&mut VideoSurface> {
        let idx = surface_id as usize;
        if idx < MAX_DPB_SURFACES {
            self.surfaces[idx].as_mut()
        } else {
            None
        }
    }

    /// Mark decoded picture as a short-term reference frame.
    pub fn mark_short_term_reference(&mut self, surface_id: u16, poc: i32, frame_num: u32) -> Result<(), &'static str> {
        let idx = surface_id as usize;
        if idx >= MAX_DPB_SURFACES || self.surfaces[idx].is_none() {
            return Err("Invalid surface id");
        }

        self.dpb_slots[idx].state = ReferenceState::ShortTermReference { poc, frame_num };
        self.dpb_slots[idx].poc = poc;
        self.dpb_slots[idx].age = self.global_age_counter;
        self.global_age_counter += 1;

        // Enforce max active reference frames limit
        let active_refs = self.count_active_references();
        if active_refs > self.max_active_references {
            let _ = self.evict_oldest_short_term_reference();
        }

        Ok(())
    }

    /// Mark decoded picture as a long-term reference frame.
    pub fn mark_long_term_reference(&mut self, surface_id: u16, long_term_idx: u32) -> Result<(), &'static str> {
        let idx = surface_id as usize;
        if idx >= MAX_DPB_SURFACES || self.surfaces[idx].is_none() {
            return Err("Invalid surface id");
        }

        self.dpb_slots[idx].state = ReferenceState::LongTermReference { long_term_idx };
        self.dpb_slots[idx].age = self.global_age_counter;
        self.global_age_counter += 1;

        Ok(())
    }

    /// Mark a surface for direct display presentation.
    pub fn mark_display_pending(&mut self, surface_id: u16) -> Result<(), &'static str> {
        let idx = surface_id as usize;
        if idx >= MAX_DPB_SURFACES || self.surfaces[idx].is_none() {
            return Err("Invalid surface id");
        }

        self.dpb_slots[idx].state = ReferenceState::DisplayPending;
        Ok(())
    }

    /// Release or unmark a surface slot from the DPB.
    pub fn release_surface(&mut self, surface_id: u16) {
        let idx = surface_id as usize;
        if idx < MAX_DPB_SURFACES {
            self.dpb_slots[idx].state = ReferenceState::Unused;
        }
    }

    /// Count total active reference frames (short-term + long-term).
    pub fn count_active_references(&self) -> usize {
        let mut count = 0;
        for slot in &self.dpb_slots {
            match slot.state {
                ReferenceState::ShortTermReference { .. } | ReferenceState::LongTermReference { .. } => {
                    count += 1;
                }
                _ => {}
            }
        }
        count
    }

    /// Evict the oldest short-term reference frame according to LRU age.
    pub fn evict_oldest_short_term_reference(&mut self) -> Option<usize> {
        let mut oldest_idx = None;
        let mut min_age = u64::MAX;

        for (i, slot) in self.dpb_slots.iter().enumerate() {
            if let ReferenceState::ShortTermReference { .. } = slot.state {
                if slot.age < min_age {
                    min_age = slot.age;
                    oldest_idx = Some(i);
                }
            }
        }

        if let Some(idx) = oldest_idx {
            self.dpb_slots[idx].state = ReferenceState::Unused;
        }

        oldest_idx
    }

    /// Find surface by POC for motion compensation reference.
    pub fn find_by_poc(&self, target_poc: i32) -> Option<u16> {
        for slot in &self.dpb_slots {
            if let ReferenceState::ShortTermReference { poc, .. } = slot.state {
                if poc == target_poc {
                    return Some(slot.surface_id);
                }
            }
        }
        None
    }
}

// -----------------------------------------------------------------------------
// Unit Self-Tests for Video Surface Allocator, DPB, and Presentation Streamer
// -----------------------------------------------------------------------------

/// Verify NV12 and P010 surface layout calculations, pitch alignment, and pixel read/write operations.
pub fn test_nv12_p010_allocator_and_dpb() -> bool {
    // 1. Test NV12 allocation with 256-byte pitch alignment (e.g. 160x90 test resolution)
    let mut nv12_surf = match VideoSurface::new(
        0,
        VideoSurfaceFormat::Nv12,
        VideoSurfaceTiling::Linear,
        160,
        90,
        256,
    ) {
        Ok(s) => s,
        Err(_) => return false,
    };

    if nv12_surf.luma_pitch != 256 {
        return false;
    }
    if nv12_surf.chroma_pitch != 256 {
        return false;
    }
    if nv12_surf.luma_offset != 0 {
        return false;
    }
    if nv12_surf.chroma_offset != 256 * 90 {
        return false;
    }
    if nv12_surf.total_size != 256 * 90 + 256 * 45 {
        return false;
    }

    // Write and read luma & chroma pixels
    nv12_surf.write_luma(50, 40, 235); // Video white in limited range
    nv12_surf.write_luma(51, 40, 16);  // Video black
    nv12_surf.write_chroma(25, 20, 128, 128); // Neutral chroma

    if nv12_surf.read_luma(50, 40) != 235 {
        return false;
    }
    if nv12_surf.read_luma(51, 40) != 16 {
        return false;
    }
    if nv12_surf.read_chroma(25, 20) != (128, 128) {
        return false;
    }

    // 2. Test P010 10-bit HDR allocation with 128-byte pitch alignment (e.g. 160x90)
    // 160 * 2 = 320 bytes -> aligned to 128-byte boundary = 384 bytes pitch.
    let mut p010_surf = match VideoSurface::new(
        1,
        VideoSurfaceFormat::P010,
        VideoSurfaceTiling::Linear,
        160,
        90,
        128,
    ) {
        Ok(s) => s,
        Err(_) => return false,
    };

    if p010_surf.luma_pitch != 384 {
        return false;
    }
    if p010_surf.total_size != 384 * 90 + 384 * 45 {
        return false;
    }

    // Test 10-bit dynamic range (0 to 1023)
    p010_surf.write_luma(80, 50, 940); // 10-bit HDR peak luma
    p010_surf.write_chroma(40, 25, 512, 600);

    if p010_surf.read_luma(80, 50) != 940 {
        return false;
    }
    if p010_surf.read_chroma(40, 25) != (512, 600) {
        return false;
    }

    // 3. Test DPB reference frame ring and LRU eviction
    let mut manager = VideoSurfaceManager::new(4); // Max 4 reference frames

    let id0 = match manager.allocate_surface(64, 32, VideoSurfaceFormat::Nv12, VideoSurfaceTiling::Linear, 128) {
        Ok(id) => id,
        Err(_) => return false,
    };
    let id1 = match manager.allocate_surface(64, 32, VideoSurfaceFormat::Nv12, VideoSurfaceTiling::Linear, 128) {
        Ok(id) => id,
        Err(_) => return false,
    };
    let id2 = match manager.allocate_surface(64, 32, VideoSurfaceFormat::Nv12, VideoSurfaceTiling::Linear, 128) {
        Ok(id) => id,
        Err(_) => return false,
    };
    let id3 = match manager.allocate_surface(64, 32, VideoSurfaceFormat::Nv12, VideoSurfaceTiling::Linear, 128) {
        Ok(id) => id,
        Err(_) => return false,
    };

    // Mark all 4 as short-term references
    if manager.mark_short_term_reference(id0, 0, 0).is_err() { return false; }
    if manager.mark_short_term_reference(id1, 2, 1).is_err() { return false; }
    if manager.mark_short_term_reference(id2, 4, 2).is_err() { return false; }
    if manager.mark_short_term_reference(id3, 6, 3).is_err() { return false; }

    if manager.count_active_references() != 4 {
        return false;
    }

    // Query by POC
    if manager.find_by_poc(2) != Some(id1) {
        return false;
    }
    if manager.find_by_poc(6) != Some(id3) {
        return false;
    }

    // Allocate 5th surface: should trigger eviction of oldest reference (id0)
    let id4 = match manager.allocate_surface(64, 32, VideoSurfaceFormat::Nv12, VideoSurfaceTiling::Linear, 128) {
        Ok(id) => id,
        Err(_) => return false,
    };
    if manager.mark_short_term_reference(id4, 8, 4).is_err() { return false; }

    // id0 should have been evicted
    if manager.find_by_poc(0) != None {
        return false;
    }
    // id1, id2, id3, id4 should be present
    if manager.find_by_poc(8) != Some(id4) {
        return false;
    }

    true
}

/// Verify BT.709 and BT.2020 color matrix conversions and zero-copy direct scanout presentation pipeline.
pub fn test_zero_copy_presentation_and_color_matrices() -> bool {
    // 1. Verify BT.709 Limited Range color conversion
    // Black: Y=16, U=128, V=128 -> RGB (0, 0, 0)
    let (r_blk, g_blk, b_blk) = nv12_to_rgb(16, 128, 128, ColorStandard::Bt709, ColorRange::Limited);
    if r_blk != 0 || g_blk != 0 || b_blk != 0 {
        return false;
    }

    // White: Y=235, U=128, V=128 -> RGB (255, 255, 255)
    let (r_wht, g_wht, b_wht) = nv12_to_rgb(235, 128, 128, ColorStandard::Bt709, ColorRange::Limited);
    if r_wht != 255 || g_wht != 255 || b_wht != 255 {
        return false;
    }

    // Red: Y=63, U=102, V=240 -> R near 255, G near 0, B near 0
    let (r_red, g_red, b_red) = nv12_to_rgb(63, 102, 240, ColorStandard::Bt709, ColorRange::Limited);
    if r_red < 250 || g_red > 10 || b_red > 10 {
        return false;
    }

    // 2. Verify BT.2020 10-bit HDR conversion
    // 10-bit Black: Y=64, U=512, V=512 -> RGB (0, 0, 0)
    let (r_hdr_blk, g_hdr_blk, b_hdr_blk) = p010_to_rgb(64, 512, 512, ColorStandard::Bt2020, ColorRange::Limited);
    if r_hdr_blk != 0 || g_hdr_blk != 0 || b_hdr_blk != 0 {
        return false;
    }

    // 10-bit White peak: Y=940, U=512, V=512 -> RGB (255, 255, 255)
    let (r_hdr_wht, g_hdr_wht, b_hdr_wht) = p010_to_rgb(940, 512, 512, ColorStandard::Bt2020, ColorRange::Limited);
    if r_hdr_wht != 255 || g_hdr_wht != 255 || b_hdr_wht != 255 {
        return false;
    }

    // 3. Verify PresentationPipeline zero-copy flips and queueing
    let mut pipeline = PresentationPipeline::new();
    pipeline.overlay_mut().configure_viewport(0, 0, 1920, 1080, 1920, 1080);

    // Initial state
    if pipeline.active_surface_id() != None {
        return false;
    }
    if pipeline.presented_frames() != 0 {
        return false;
    }

    // Queue 2 frames with presentation timestamps (PTS)
    if pipeline.queue_surface(1, 16_666, 10_000).is_err() {
        return false;
    }
    if pipeline.queue_surface(2, 33_333, 20_000).is_err() {
        return false;
    }

    // Simulate VSYNC 1 at 16,666 us
    let flipped_id1 = pipeline.on_vsync_flip(16_666);
    if flipped_id1 != Some(1) {
        return false;
    }
    if pipeline.active_surface_id() != Some(1) {
        return false;
    }
    if pipeline.presented_frames() != 1 {
        return false;
    }
    if pipeline.last_pts() != 16_666 {
        return false;
    }

    // Simulate VSYNC 2 at 33,333 us
    let flipped_id2 = pipeline.on_vsync_flip(33_333);
    if flipped_id2 != Some(2) {
        return false;
    }
    if pipeline.active_surface_id() != Some(2) {
        return false;
    }
    if pipeline.presented_frames() != 2 {
        return false;
    }
    if pipeline.last_pts() != 33_333 {
        return false;
    }

    // Average latency should be calculated correctly
    // Flip 1: 16666 - 10000 = 6666
    // Flip 2: 33333 - 20000 = 13333
    // Total latency = 19999, avg = 19999 / 2 = 9999
    if pipeline.average_latency_us() != 9999 {
        return false;
    }

    // 4. Test rasterization to synthetic framebuffer memory
    let mut fb_memory = [0u32; 64 * 64];
    let mut test_surface = match VideoSurface::new(0, VideoSurfaceFormat::Nv12, VideoSurfaceTiling::Linear, 16, 16, 64) {
        Ok(s) => s,
        Err(_) => return false,
    };
    // Fill surface with white (Y=235, U=128, V=128)
    for y in 0..16 {
        for x in 0..16 {
            test_surface.write_luma(x, y, 235);
        }
    }
    for cy in 0..8 {
        for cx in 0..8 {
            test_surface.write_chroma(cx, cy, 128, 128);
        }
    }

    let mut render_pipeline = PresentationPipeline::new();
    render_pipeline.overlay_mut().configure_viewport(0, 0, 16, 16, 16, 16);
    render_pipeline.rasterize_to_framebuffer(
        &test_surface,
        fb_memory.as_mut_ptr(),
        64,
        64,
        64,
    );

    // Pixel (0, 0) should be ARGB white: 0xFFFFFFFF
    if fb_memory[0] != 0xFF_FF_FF_FF {
        return false;
    }

    true
}
