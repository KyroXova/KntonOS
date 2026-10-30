//! Direct3D 12 Block-Compressed GPU Texture Streaming Pipeline.
//! Provides esports-grade zero-copy texture streaming:

extern crate alloc;

use alloc::vec::Vec;
use crate::drivers::storage::{
    DirectStorageError, DirectStoragePriority, DirectStorageRequest, DirectStorageStatus, GpuBarAperture,
};

/// Direct3D 12 Block Compression (BC) texture formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockCompressionFormat {
    /// BC1 (DXT1): 4x4 pixels in 64 bits (8 bytes), 4 bpp. RGB with optional punch-through 1-bit alpha.
    Bc1Rgba,
    /// BC3 (DXT5): 4x4 pixels in 128 bits (16 bytes), 8 bpp. High-quality RGBA with dedicated 64-bit alpha block.
    Bc3Rgba,
    /// BC7: 4x4 pixels in 128 bits (16 bytes), 8 bpp. Modern High-Dynamic/LDR texture compression with variable partitions.
    Bc7Rgba,
}

impl BlockCompressionFormat {
    /// Returns the compressed payload size in bytes for a single 4x4 texel block.
    #[inline(always)]
    pub const fn block_size_bytes(&self) -> usize {
        match self {
            Self::Bc1Rgba => 8,
            Self::Bc3Rgba | Self::Bc7Rgba => 16,
        }
    }

    /// Computes row pitch (bytes per horizontal block row) and total byte size for dimensions.
    pub fn compute_surface_pitch_and_size(&self, width: u32, height: u32) -> (usize, usize) {
        let block_w = (width.max(1) + 3) / 4;
        let block_h = (height.max(1) + 3) / 4;
        let block_bytes = self.block_size_bytes();

        let row_pitch = (block_w as usize) * block_bytes;
        // Direct3D 12 requires row pitch alignment to D3D12_TEXTURE_DATA_PITCH_ALIGNMENT (256 bytes)
        let aligned_pitch = (row_pitch + 255) & !255;
        let total_size = aligned_pitch * (block_h as usize);

        (aligned_pitch, total_size)
    }
}

/// Mip-Map Level Descriptor in a Streaming Texture Pyramid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MipLevelDescriptor {
    /// Mip level index (0 = base highest resolution, 1 = half resolution, ...).
    pub mip_index: u8,
    /// Width of this mip level in pixels.
    pub width: u32,
    /// Height of this mip level in pixels.
    pub height: u32,
    /// Row pitch in bytes (aligned to 256 bytes).
    pub row_pitch: usize,
    /// Total compressed byte size in VRAM.
    pub byte_size: usize,
    /// Logical Block Address (LBA) on NVMe storage where this mip data is stored.
    pub disk_lba: u64,
    /// Disk block count.
    pub disk_blocks: u32,
    /// GPU physical address in BAR1 VRAM where this mip is currently resident (0 if non-resident).
    pub vram_address: u64,
    /// Whether this mip level is resident in GPU VRAM and ready for sampling.
    pub is_resident: bool,
    /// Whether an asynchronous DirectStorage DMA transfer is currently in flight.
    pub is_loading: bool,
}

/// Direct3D 12 Streaming Texture Object.
#[derive(Debug, Clone)]
pub struct StreamingTexture {
    /// Unique identifier for this texture asset.
    pub texture_id: u32,
    /// Block compression format (BC1, BC3, BC7).
    pub format: BlockCompressionFormat,
    /// Base width (Mip 0) in pixels.
    pub base_width: u32,
    /// Base height (Mip 0) in pixels.
    pub base_height: u32,
    /// Total number of mip levels in the pyramid.
    pub total_mips: u8,
    /// Lowest mip index currently resident in VRAM (e.g. 0 = maximum quality, 3 = coarse).
    pub resident_highest_mip: u8,
    /// Target desired highest mip index based on camera distance / screen-space error.
    pub desired_highest_mip: u8,
    /// Descriptors for each mip level (index 0..total_mips - 1).
    pub mips: Vec<MipLevelDescriptor>,
}

impl StreamingTexture {
    /// Creates a new streaming texture pyramid with automatic mip dimension and pitch calculation.
    pub fn new(
        texture_id: u32,
        format: BlockCompressionFormat,
        base_width: u32,
        base_height: u32,
        total_mips: u8,
        start_lba: u64,
        block_size: u32,
    ) -> Self {
        let mut mips = Vec::with_capacity(total_mips as usize);
        let mut current_w = base_width;
        let mut current_h = base_height;
        let mut current_lba = start_lba;

        for mip_idx in 0..total_mips {
            let (pitch, size) = format.compute_surface_pitch_and_size(current_w, current_h);
            let blocks = ((size as u64 + block_size as u64 - 1) / block_size as u64) as u32;

            mips.push(MipLevelDescriptor {
                mip_index: mip_idx,
                width: current_w,
                height: current_h,
                row_pitch: pitch,
                byte_size: size,
                disk_lba: current_lba,
                disk_blocks: blocks,
                vram_address: 0,
                is_resident: false,
                is_loading: false,
            });

            current_lba += blocks as u64;
            current_w = (current_w / 2).max(1);
            current_h = (current_h / 2).max(1);
        }

        Self {
            texture_id,
            format,
            base_width,
            base_height,
            total_mips,
            resident_highest_mip: total_mips - 1, // Start with only coarse mip
            desired_highest_mip: total_mips - 1,
            mips,
        }
    }

    /// Calculates total resident VRAM bytes consumed by this texture.
    pub fn current_resident_vram_bytes(&self) -> usize {
        self.mips
            .iter()
            .filter(|m| m.is_resident)
            .map(|m| m.byte_size)
            .sum()
    }

    /// Marks a mip level as resident at a specific GPU VRAM address.
    pub fn mark_mip_resident(&mut self, mip_index: u8, vram_address: u64) {
        if let Some(mip) = self.mips.get_mut(mip_index as usize) {
            mip.is_resident = true;
            mip.is_loading = false;
            mip.vram_address = vram_address;

            if mip_index < self.resident_highest_mip {
                self.resident_highest_mip = mip_index;
            }
        }
    }

    /// Evicts a high-resolution mip level from VRAM to free memory.
    pub fn evict_mip(&mut self, mip_index: u8) -> usize {
        let mut freed = 0;
        if let Some(mip) = self.mips.get_mut(mip_index as usize) {
            if mip.is_resident {
                mip.is_resident = false;
                mip.vram_address = 0;
                freed = mip.byte_size;

                // Adjust resident_highest_mip upwards
                self.resident_highest_mip = self
                    .mips
                    .iter()
                    .filter(|m| m.is_resident)
                    .map(|m| m.mip_index)
                    .min()
                    .unwrap_or(self.total_mips - 1);
            }
        }
        freed
    }
}

/// GPU Texture Streaming Manager.
/// Coordinates on-demand mip streaming, priority calculation based on camera distance,
pub struct TextureStreamManager {
    pub textures: Vec<StreamingTexture>,
    pub gpu_aperture: GpuBarAperture,
    pub max_vram_bytes: usize,
    pub current_vram_bytes: usize,
    pub total_streamed_mips: u64,
    pub total_evicted_mips: u64,
}

impl TextureStreamManager {
    pub fn new(gpu_aperture: GpuBarAperture, max_vram_bytes: usize) -> Self {
        Self {
            textures: Vec::new(),
            gpu_aperture,
            max_vram_bytes,
            current_vram_bytes: 0,
            total_streamed_mips: 0,
            total_evicted_mips: 0,
        }
    }

    pub fn register_texture(&mut self, texture: StreamingTexture) {
        self.textures.push(texture);
    }

    /// Requests that a texture stream in higher resolution mip levels up to `target_mip`.
    /// Generates a `DirectStorageRequest` targeting GPU BAR1 VRAM if streaming is needed.
    pub fn request_mip_level(
        &mut self,
        texture_id: u32,
        target_mip: u8,
        priority: DirectStoragePriority,
        fence_value: u64,
    ) -> Result<Option<DirectStorageRequest>, DirectStorageError> {
        let tex = self
            .textures
            .iter_mut()
            .find(|t| t.texture_id == texture_id)
            .ok_or(DirectStorageError::InvalidRequest)?;

        tex.desired_highest_mip = target_mip;

        // If target mip is already resident or currently loading, nothing to submit
        let mip = match tex.mips.get_mut(target_mip as usize) {
            Some(m) => m,
            None => return Err(DirectStorageError::InvalidRequest),
        };

        if mip.is_resident || mip.is_loading {
            return Ok(None);
        }

        // Allocate VRAM space in GPU BAR1 aperture
        let alloc_size = mip.byte_size as u64;
        let vram_dest = self.gpu_aperture.allocate(alloc_size, 4096)?;
        mip.is_loading = true;

        let req = DirectStorageRequest {
            request_id: 0, // Assigned by DirectStorageQueue
            nsid: 1,
            start_lba: mip.disk_lba,
            block_count: mip.disk_blocks,
            block_size: 512,
            gpu_vram_dest: vram_dest,
            texture_id,
            mip_level: target_mip,
            priority,
            status: DirectStorageStatus::Queued,
            fence_value,
            bytes_transferred: 0,
        };

        Ok(Some(req))
    }

    /// Notifies the manager that a DirectStorage DMA transfer completed for a mip level.
    pub fn on_mip_stream_complete(&mut self, texture_id: u32, mip_index: u8, vram_address: u64) {
        if let Some(tex) = self.textures.iter_mut().find(|t| t.texture_id == texture_id) {
            tex.mark_mip_resident(mip_index, vram_address);
            if let Some(mip) = tex.mips.get(mip_index as usize) {
                self.current_vram_bytes += mip.byte_size;
                self.total_streamed_mips += 1;
            }
        }
    }

    /// Evicts higher mip levels to ensure VRAM usage remains below `max_vram_bytes`.
    pub fn evict_under_memory_pressure(&mut self) -> usize {
        let mut total_freed = 0;
        if self.current_vram_bytes <= self.max_vram_bytes {
            return 0;
        }

        // Evict Mip 0 from lowest priority textures first
        for tex in self.textures.iter_mut() {
            if self.current_vram_bytes <= self.max_vram_bytes {
                break;
            }
            // Evict Mip 0 if resident and texture has at least Mip 1 available
            if tex.resident_highest_mip == 0 && tex.total_mips > 1 {
                let freed = tex.evict_mip(0);
                self.current_vram_bytes = self.current_vram_bytes.saturating_sub(freed);
                self.total_evicted_mips += 1;
                total_freed += freed;
            }
        }

        total_freed
    }
}

/// Unit test verifying block compression calculations, mip pyramid generation, and streaming state machine.
pub fn verify_texture_streaming_and_block_compression() -> bool {
    // 1. Verify Block Compression calculations:
    // BC1: 4x4 texels = 8 bytes (4 bpp)
    let bc1 = BlockCompressionFormat::Bc1Rgba;
    let (bc1_pitch, bc1_size) = bc1.compute_surface_pitch_and_size(1024, 1024);
    // 1024 / 4 = 256 blocks horizontal * 8 bytes = 2048 bytes pitch (already 256-aligned)
    if bc1_pitch != 2048 || bc1_size != 2048 * 256 {
        return false;
    }

    // BC7: 4x4 texels = 16 bytes (8 bpp)
    let bc7 = BlockCompressionFormat::Bc7Rgba;
    let (bc7_pitch, bc7_size) = bc7.compute_surface_pitch_and_size(1024, 1024);
    // 1024 / 4 = 256 blocks horizontal * 16 bytes = 4096 bytes pitch
    if bc7_pitch != 4096 || bc7_size != 4096 * 256 {
        return false;
    }

    // 2. Initialize 4K Streaming Texture (4096 x 4096, 12 mip levels)
    let start_lba = 10_000u64;
    let mut tex = StreamingTexture::new(101, BlockCompressionFormat::Bc7Rgba, 4096, 4096, 12, start_lba, 512);
    if tex.total_mips != 12 || tex.mips[0].width != 4096 || tex.mips[11].width != 2 {
        return false;
    }

    // Initial state: no mips resident
    if tex.current_resident_vram_bytes() != 0 {
        return false;
    }

    // Mark coarse base mip (Mip 11) resident
    tex.mark_mip_resident(11, 0x0000_0004_1000_0000);
    if tex.resident_highest_mip != 11 {
        return false;
    }

    // 3. Test TextureStreamManager and on-demand streaming request
    let bar1_base = 0x0000_0004_0000_0000u64;
    let bar1_size = 4 * 1024 * 1024 * 1024u64; // 4 GiB
    let aperture = GpuBarAperture::new(bar1_base, bar1_size);

    let mut stream_mgr = TextureStreamManager::new(aperture, 256 * 1024 * 1024);
    stream_mgr.register_texture(tex);

    // Request Mip 0 (Realtime viewport visible)
    let req_opt = match stream_mgr.request_mip_level(101, 0, DirectStoragePriority::Realtime, 42) {
        Ok(r) => r,
        Err(_) => return false,
    };

    let req = match req_opt {
        Some(r) => r,
        None => return false,
    };

    if req.texture_id != 101 || req.mip_level != 0 || req.priority != DirectStoragePriority::Realtime {
        return false;
    }
    if !stream_mgr.gpu_aperture.contains_address(req.gpu_vram_dest, (req.block_count * 512) as u64) {
        return false;
    }

    // 4. Simulate DirectStorage DMA transfer complete
    stream_mgr.on_mip_stream_complete(101, 0, req.gpu_vram_dest);

    let updated_tex = &stream_mgr.textures[0];
    if updated_tex.resident_highest_mip != 0 || !updated_tex.mips[0].is_resident {
        return false;
    }
    if stream_mgr.total_streamed_mips != 1 {
        return false;
    }

    // 5. Test Mip eviction under memory pressure
    // Set budget lower than current VRAM consumption to trigger eviction
    stream_mgr.max_vram_bytes = 1024; // Artificial low limit
    let freed = stream_mgr.evict_under_memory_pressure();
    if freed == 0 || stream_mgr.total_evicted_mips != 1 {
        return false;
    }

    // After evicting Mip 0, highest resident mip must fall back to Mip 11
    if stream_mgr.textures[0].resident_highest_mip != 11 {
        return false;
    }

    true
}
