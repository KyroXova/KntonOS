//! NV12 / P010 Tiled Video Frame Buffer Allocator & Decoded Picture Buffer (DPB) Ring.
//!
//! Implements GPU video surface memory layout and reference frame tracking:
//! - Bi-planar video surface layout: NV12 (8-bit SDR) and P010 (10-bit HDR10).
//! - Hardware pitch alignment (128-byte and 256-byte cache-line row strides).
//! - Linear and micro-tiled (Block-Linear) memory coordinate transformations.
//! - Decoded Picture Buffer (DPB) reference frame ring with short-term / long-term picture marking and LRU eviction.

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
// Unit Self-Tests for Video Surface Allocator & DPB Ring
// -----------------------------------------------------------------------------

/// Verify NV12 and P010 surface layout calculations, pitch alignment, and pixel read/write operations.
pub fn test_nv12_p010_allocator_and_dpb() -> bool {
    // 1. Test NV12 allocation with 256-byte pitch alignment (e.g. 1920x1080)
    // 1920 is divisible by 256? 1920 / 256 = 7.5 -> aligned pitch = 2048.
    let mut nv12_surf = match VideoSurface::new(
        0,
        VideoSurfaceFormat::Nv12,
        VideoSurfaceTiling::Linear,
        1920,
        1080,
        256,
    ) {
        Ok(s) => s,
        Err(_) => return false,
    };

    if nv12_surf.luma_pitch != 2048 {
        return false;
    }
    if nv12_surf.chroma_pitch != 2048 {
        return false;
    }
    if nv12_surf.luma_offset != 0 {
        return false;
    }
    if nv12_surf.chroma_offset != 2048 * 1080 {
        return false;
    }
    if nv12_surf.total_size != 2048 * 1080 + 2048 * 540 {
        return false;
    }

    // Write and read luma & chroma pixels
    nv12_surf.write_luma(100, 200, 235); // Video white in limited range
    nv12_surf.write_luma(101, 200, 16);  // Video black
    nv12_surf.write_chroma(50, 100, 128, 128); // Neutral chroma

    if nv12_surf.read_luma(100, 200) != 235 {
        return false;
    }
    if nv12_surf.read_luma(101, 200) != 16 {
        return false;
    }
    if nv12_surf.read_chroma(50, 100) != (128, 128) {
        return false;
    }

    // 2. Test P010 10-bit HDR allocation with 128-byte pitch alignment (e.g. 3840x2160 4K)
    // 3840 * 2 = 7680 bytes per row. 7680 is divisible by 128 (7680 / 128 = 60).
    let mut p010_surf = match VideoSurface::new(
        1,
        VideoSurfaceFormat::P010,
        VideoSurfaceTiling::Linear,
        3840,
        2160,
        128,
    ) {
        Ok(s) => s,
        Err(_) => return false,
    };

    if p010_surf.luma_pitch != 7680 {
        return false;
    }
    if p010_surf.total_size != 7680 * 2160 + 7680 * 1080 {
        return false;
    }

    // Test 10-bit dynamic range (0 to 1023)
    p010_surf.write_luma(500, 800, 940); // 10-bit HDR peak luma
    p010_surf.write_chroma(250, 400, 512, 600);

    if p010_surf.read_luma(500, 800) != 940 {
        return false;
    }
    if p010_surf.read_chroma(250, 400) != (512, 600) {
        return false;
    }

    // 3. Test DPB reference frame ring and LRU eviction
    let mut manager = VideoSurfaceManager::new(4); // Max 4 reference frames

    let id0 = match manager.allocate_surface(1280, 720, VideoSurfaceFormat::Nv12, VideoSurfaceTiling::Linear, 128) {
        Ok(id) => id,
        Err(_) => return false,
    };
    let id1 = match manager.allocate_surface(1280, 720, VideoSurfaceFormat::Nv12, VideoSurfaceTiling::Linear, 128) {
        Ok(id) => id,
        Err(_) => return false,
    };
    let id2 = match manager.allocate_surface(1280, 720, VideoSurfaceFormat::Nv12, VideoSurfaceTiling::Linear, 128) {
        Ok(id) => id,
        Err(_) => return false,
    };
    let id3 = match manager.allocate_surface(1280, 720, VideoSurfaceFormat::Nv12, VideoSurfaceTiling::Linear, 128) {
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
    let id4 = match manager.allocate_surface(1280, 720, VideoSurfaceFormat::Nv12, VideoSurfaceTiling::Linear, 128) {
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
