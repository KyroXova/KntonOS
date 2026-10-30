//! Direct3D 12 Kernel-Direct GPU Sampler Feedback Streaming & Texture LOD Clamp Engine.
//! Implements hardware-accelerated Direct3D 12 Sampler Feedback specifications:

use alloc::vec;
use alloc::vec::Vec;

/// Value indicating no mip level was sampled in a feedback tile (0xFF).
pub const SAMPLER_FEEDBACK_UNACCESSED_MIP: u8 = 0xFF;

/// Maximum number of mip levels supported per texture resource.
pub const MAX_TEXTURE_MIP_LEVELS: usize = 16;

/// Default feedback tile dimension in texels (128x128).
pub const DEFAULT_FEEDBACK_TILE_SIZE: u32 = 128;

/// Errors returned by the sampler feedback subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SamplerFeedbackError {
    /// Texture width or height is zero or invalid.
    InvalidTextureDimensions,
    /// Tile coordinate or texel coordinate is out of bounds.
    OutOfBounds,
    /// Mip level index exceeds texture's total mip count.
    InvalidMipLevel,
    /// Tile streaming request queue is full.
    QueueFull,
    /// Unsupported or invalid sampler feedback type.
    InvalidFeedbackType,
}

/// Direct3D 12 Sampler Feedback map format archetype.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SamplerFeedbackType {
    /// MinMipMap: Records the finest (numerically lowest) mip level accessed in each tile region.
    MinMipMap,
    /// TextureTranscoding: Records accessed tiles requiring on-demand format decompression or transcoding.
    TextureTranscoding,
}

/// Sampler Feedback tile dimension granularity in texels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum FeedbackTileGranularity {
    /// 32x32 texels per feedback tile.
    Tile32x32 = 32,
    /// 64x64 texels per feedback tile.
    Tile64x64 = 64,
    /// 128x128 texels per feedback tile.
    Tile128x128 = 128,
    /// 256x256 texels per feedback tile.
    Tile256x256 = 256,
}

impl FeedbackTileGranularity {
    /// Returns tile size in texels along one dimension.
    #[inline]
    pub const fn texel_size(self) -> u32 {
        self as u32
    }
}

/// Discrete rectangular region of tiles within a feedback map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FeedbackTileRegion {
    /// Starting tile coordinate X.
    pub tile_x: u32,
    /// Starting tile coordinate Y.
    pub tile_y: u32,
    /// Number of tiles horizontally.
    pub width_tiles: u32,
    /// Number of tiles vertically.
    pub height_tiles: u32,
}

impl FeedbackTileRegion {
    /// Creates a new tile region.
    pub const fn new(tile_x: u32, tile_y: u32, width_tiles: u32, height_tiles: u32) -> Self {
        Self {
            tile_x,
            tile_y,
            width_tiles,
            height_tiles,
        }
    }

    /// Evaluates if a given tile coordinate `(tx, ty)` lies within this region.
    pub fn contains(&self, tx: u32, ty: u32) -> bool {
        tx >= self.tile_x
            && tx < self.tile_x.saturating_add(self.width_tiles)
            && ty >= self.tile_y
            && ty < self.tile_y.saturating_add(self.height_tiles)
    }
}

/// Sampler Feedback Map Descriptor.
/// Configures the association between a source texture resource and its paired
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeedbackMapDescriptor {
    /// Feedback map archetype (MinMipMap or TextureTranscoding).
    pub feedback_type: SamplerFeedbackType,
    /// Width of the paired texture in texels.
    pub texture_width: u32,
    /// Height of the paired texture in texels.
    pub texture_height: u32,
    /// Total number of mip levels in the paired texture.
    pub mip_levels: u8,
    /// Tile granularity in texels.
    pub tile_granularity: FeedbackTileGranularity,
    /// Number of horizontal feedback tiles.
    pub tiles_x: u32,
    /// Number of vertical feedback tiles.
    pub tiles_y: u32,
}

impl FeedbackMapDescriptor {
    /// Creates a new FeedbackMapDescriptor with validated dimensions.
    pub fn new(
        feedback_type: SamplerFeedbackType,
        texture_width: u32,
        texture_height: u32,
        mip_levels: u8,
        tile_granularity: FeedbackTileGranularity,
    ) -> Result<Self, SamplerFeedbackError> {
        if texture_width == 0 || texture_height == 0 {
            return Err(SamplerFeedbackError::InvalidTextureDimensions);
        }
        if mip_levels == 0 || mip_levels as usize > MAX_TEXTURE_MIP_LEVELS {
            return Err(SamplerFeedbackError::InvalidMipLevel);
        }

        let tile_size = tile_granularity.texel_size();
        let tiles_x = (texture_width + tile_size - 1) / tile_size;
        let tiles_y = (texture_height + tile_size - 1) / tile_size;

        Ok(Self {
            feedback_type,
            texture_width,
            texture_height,
            mip_levels,
            tile_granularity,
            tiles_x,
            tiles_y,
        })
    }

    /// Total number of feedback tiles in this map.
    #[inline]
    pub fn total_tiles(&self) -> usize {
        (self.tiles_x as usize) * (self.tiles_y as usize)
    }

    /// Maps a 2D texel coordinate `(x, y)` to its corresponding feedback tile coordinate `(tile_x, tile_y)`.
    pub fn texel_to_tile(&self, x: u32, y: u32) -> (u32, u32) {
        let tile_size = self.tile_granularity.texel_size();
        let tx = core::cmp::min(x / tile_size, self.tiles_x.saturating_sub(1));
        let ty = core::cmp::min(y / tile_size, self.tiles_y.saturating_sub(1));
        (tx, ty)
    }
}

/// Underlying storage for a MinMipMap sampler feedback surface.
/// Each tile is represented by 1 byte storing the minimum (highest resolution) mip level
#[derive(Debug, Clone)]
pub struct MinMipMapStorage {
    tiles_x: u32,
    tiles_y: u32,
    data: Vec<u8>,
}

impl MinMipMapStorage {
    /// Creates a new storage buffer initialized to unaccessed state (0xFF).
    pub fn new(tiles_x: u32, tiles_y: u32) -> Self {
        let total = (tiles_x as usize) * (tiles_y as usize);
        Self {
            tiles_x,
            tiles_y,
            data: vec![SAMPLER_FEEDBACK_UNACCESSED_MIP; total],
        }
    }

    /// Clears the feedback map back to 0xFF (GPU clear operation).
    pub fn clear(&mut self) {
        for byte in self.data.iter_mut() {
            *byte = SAMPLER_FEEDBACK_UNACCESSED_MIP;
        }
    }

    /// Retrieves the recorded minimum mip level for tile `(tile_x, tile_y)`.
    pub fn get(&self, tile_x: u32, tile_y: u32) -> Option<u8> {
        if tile_x >= self.tiles_x || tile_y >= self.tiles_y {
            return None;
        }
        let index = (tile_y as usize) * (self.tiles_x as usize) + (tile_x as usize);
        self.data.get(index).copied()
    }

    /// Records a texture sample operation into the feedback map (atomic minimum update).
    pub fn record_sample(&mut self, tile_x: u32, tile_y: u32, sampled_mip: u8) -> bool {
        if tile_x >= self.tiles_x || tile_y >= self.tiles_y {
            return false;
        }
        let index = (tile_y as usize) * (self.tiles_x as usize) + (tile_x as usize);
        if let Some(slot) = self.data.get_mut(index) {
            // Atomic Min: record coarsest or finest based on MinMip semantics (smaller number = higher res)
            if *slot == SAMPLER_FEEDBACK_UNACCESSED_MIP || sampled_mip < *slot {
                *slot = sampled_mip;
            }
            true
        } else {
            false
        }
    }

    /// Whether a tile has been accessed by any shader invocation since last clear.
    #[inline]
    pub fn is_accessed(&self, tile_x: u32, tile_y: u32) -> bool {
        self.get(tile_x, tile_y)
            .map(|val| val != SAMPLER_FEEDBACK_UNACCESSED_MIP)
            .unwrap_or(false)
    }

    /// Returns a slice over the raw 1-byte-per-tile buffer.
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }

    /// Total tiles in the storage buffer.
    #[inline]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Whether the storage buffer is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

/// Flag indicating a tile requires on-demand format decompression or transcoding.
pub const TRANSCODE_FLAG_REQUIRED: u8 = 0x01;
/// Flag indicating a tile has completed transcoding and is ready for sampling.
pub const TRANSCODE_FLAG_READY: u8 = 0x02;

/// Underlying storage for a Texture Transcoding sampler feedback surface.
/// Records which texture tiles were accessed by shaders to drive asynchronous
#[derive(Debug, Clone)]
pub struct TextureTranscodingStorage {
    tiles_x: u32,
    tiles_y: u32,
    flags: Vec<u8>,
}

impl TextureTranscodingStorage {
    /// Creates a new transcoding feedback storage buffer.
    pub fn new(tiles_x: u32, tiles_y: u32) -> Self {
        let total = (tiles_x as usize) * (tiles_y as usize);
        Self {
            tiles_x,
            tiles_y,
            flags: vec![0u8; total],
        }
    }

    /// Clears all transcoding flags back to zero.
    pub fn clear(&mut self) {
        for slot in self.flags.iter_mut() {
            *slot = 0;
        }
    }

    /// Marks a tile as accessed and requiring transcoding.
    pub fn mark_transcode_needed(&mut self, tile_x: u32, tile_y: u32) -> bool {
        if tile_x >= self.tiles_x || tile_y >= self.tiles_y {
            return false;
        }
        let index = (tile_y as usize) * (self.tiles_x as usize) + (tile_x as usize);
        if let Some(slot) = self.flags.get_mut(index) {
            *slot |= TRANSCODE_FLAG_REQUIRED;
            true
        } else {
            false
        }
    }

    /// Marks a tile as transcoded and ready.
    pub fn mark_transcode_ready(&mut self, tile_x: u32, tile_y: u32) -> bool {
        if tile_x >= self.tiles_x || tile_y >= self.tiles_y {
            return false;
        }
        let index = (tile_y as usize) * (self.tiles_x as usize) + (tile_x as usize);
        if let Some(slot) = self.flags.get_mut(index) {
            *slot = (*slot & !TRANSCODE_FLAG_REQUIRED) | TRANSCODE_FLAG_READY;
            true
        } else {
            false
        }
    }

    /// Checks if a tile is flagged as requiring transcoding.
    pub fn is_transcode_needed(&self, tile_x: u32, tile_y: u32) -> bool {
        if tile_x >= self.tiles_x || tile_y >= self.tiles_y {
            return false;
        }
        let index = (tile_y as usize) * (self.tiles_x as usize) + (tile_x as usize);
        self.flags.get(index).map(|&f| (f & TRANSCODE_FLAG_REQUIRED) != 0).unwrap_or(false)
    }

    /// Checks if a tile is marked as transcoded and ready.
    pub fn is_transcode_ready(&self, tile_x: u32, tile_y: u32) -> bool {
        if tile_x >= self.tiles_x || tile_y >= self.tiles_y {
            return false;
        }
        let index = (tile_y as usize) * (self.tiles_x as usize) + (tile_x as usize);
        self.flags.get(index).map(|&f| (f & TRANSCODE_FLAG_READY) != 0).unwrap_or(false)
    }
}

/// Tracks per-tile mip level residency for tiled / sparse textures in VRAM.
#[derive(Debug, Clone)]
pub struct TextureMipResidency {
    tiles_x: u32,
    tiles_y: u32,
    base_resident_mip: u8,
    resident_mips: Vec<u8>,
}

impl TextureMipResidency {
    /// Creates a new residency map initialized to `base_resident_mip` for all tiles.
    pub fn new(tiles_x: u32, tiles_y: u32, base_resident_mip: u8) -> Self {
        let total = (tiles_x as usize) * (tiles_y as usize);
        Self {
            tiles_x,
            tiles_y,
            base_resident_mip,
            resident_mips: vec![base_resident_mip; total],
        }
    }

    /// Returns the currently resident finest mip level for tile `(tile_x, tile_y)`.
    pub fn get_resident_mip(&self, tile_x: u32, tile_y: u32) -> u8 {
        if tile_x >= self.tiles_x || tile_y >= self.tiles_y {
            return self.base_resident_mip;
        }
        let index = (tile_y as usize) * (self.tiles_x as usize) + (tile_x as usize);
        self.resident_mips.get(index).copied().unwrap_or(self.base_resident_mip)
    }

    /// Promotes a tile's resident mip level to a finer (higher-resolution) mip level.
    pub fn promote_tile_mip(&mut self, tile_x: u32, tile_y: u32, new_mip: u8) -> bool {
        if tile_x >= self.tiles_x || tile_y >= self.tiles_y {
            return false;
        }
        let index = (tile_y as usize) * (self.tiles_x as usize) + (tile_x as usize);
        if let Some(slot) = self.resident_mips.get_mut(index) {
            if new_mip < *slot {
                *slot = new_mip;
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    /// Demotes a tile's resident mip level (coarser resolution, e.g. for memory eviction).
    pub fn demote_tile_mip(&mut self, tile_x: u32, tile_y: u32, new_mip: u8) -> bool {
        if tile_x >= self.tiles_x || tile_y >= self.tiles_y {
            return false;
        }
        let index = (tile_y as usize) * (self.tiles_x as usize) + (tile_x as usize);
        if let Some(slot) = self.resident_mips.get_mut(index) {
            if new_mip > *slot {
                *slot = new_mip;
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    /// Evaluates the coarsest resident mip across the entire texture.
    pub fn coarsest_resident_mip(&self) -> u8 {
        self.resident_mips.iter().copied().max().unwrap_or(self.base_resident_mip)
    }

    /// Evaluates the finest resident mip across the entire texture.
    pub fn finest_resident_mip(&self) -> u8 {
        self.resident_mips.iter().copied().min().unwrap_or(self.base_resident_mip)
    }

    /// Resets all tiles back to the base resident mip.
    pub fn reset_to_base(&mut self) {
        for slot in self.resident_mips.iter_mut() {
            *slot = self.base_resident_mip;
        }
    }
}

/// A prioritized tile streaming request generated from feedback analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileStreamingRequest {
    /// Texture resource identifier.
    pub texture_id: u32,
    /// Tile coordinate X.
    pub tile_x: u32,
    /// Tile coordinate Y.
    pub tile_y: u32,
    /// Mip level requested by the GPU shader.
    pub requested_mip: u8,
    /// Streaming urgency priority (higher value = higher priority).
    pub priority: u32,
}

impl TileStreamingRequest {
    /// Creates a new streaming request.
    pub const fn new(
        texture_id: u32,
        tile_x: u32,
        tile_y: u32,
        requested_mip: u8,
        priority: u32,
    ) -> Self {
        Self {
            texture_id,
            tile_x,
            tile_y,
            requested_mip,
            priority,
        }
    }
}

/// Request queue prioritizing texture tile streaming for DirectStorage and asynchronous I/O.
#[derive(Debug, Clone)]
pub struct TileRequestQueue {
    requests: Vec<TileStreamingRequest>,
    capacity: usize,
}

impl TileRequestQueue {
    /// Creates a new request queue with the specified maximum capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            requests: Vec::with_capacity(capacity),
            capacity,
        }
    }

    /// Pushes a tile streaming request into the queue with deduplication and priority ordering.
    /// Returns `Ok(true)` if newly added, `Ok(false)` if merged/deduplicated, or `Err(QueueFull)`.
    pub fn push(&mut self, request: TileStreamingRequest) -> Result<bool, SamplerFeedbackError> {
        // Check for duplicate request targeting the same tile
        if let Some(existing) = self.requests.iter_mut().find(|r| {
            r.texture_id == request.texture_id && r.tile_x == request.tile_x && r.tile_y == request.tile_y
        }) {
            // Update to higher priority and finer mip level if new request is more urgent
            if request.requested_mip < existing.requested_mip {
                existing.requested_mip = request.requested_mip;
            }
            if request.priority > existing.priority {
                existing.priority = request.priority;
            }
            return Ok(false);
        }

        if self.requests.len() >= self.capacity {
            return Err(SamplerFeedbackError::QueueFull);
        }

        // Insert maintaining descending priority order
        let insert_pos = self
            .requests
            .iter()
            .position(|r| r.priority < request.priority)
            .unwrap_or(self.requests.len());

        self.requests.insert(insert_pos, request);
        Ok(true)
    }

    /// Pops the highest-priority pending tile request.
    pub fn pop(&mut self) -> Option<TileStreamingRequest> {
        if self.requests.is_empty() {
            None
        } else {
            Some(self.requests.remove(0))
        }
    }

    /// Peeks at the highest-priority pending tile request without removing it.
    #[inline]
    pub fn peek(&self) -> Option<&TileStreamingRequest> {
        self.requests.first()
    }

    /// Number of pending requests currently queued.
    #[inline]
    pub fn len(&self) -> usize {
        self.requests.len()
    }

    /// Whether the queue is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }

    /// Maximum capacity of the queue.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Clears all pending requests.
    pub fn clear(&mut self) {
        self.requests.clear();
    }
}

/// Telemetry statistics for the sampler feedback and texture streaming engine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SamplerFeedbackStats {
    /// Total simulated GPU texture sample operations recorded into feedback map.
    pub total_samples_recorded: u64,
    /// Total evaluations performed across the feedback map.
    pub evaluations_performed: u64,
    /// Total tile streaming I/O requests generated for missing high-res mips.
    pub tile_requests_generated: u64,
    /// Total tile streaming requests fulfilled and uploaded to VRAM.
    pub tile_requests_fulfilled: u64,
}

/// Dynamic Texture LOD Clamp Governor.
/// Dynamically calculates hardware sampler MinLOD clamps to prevent GPU shaders from
#[derive(Debug, Clone)]
pub struct TextureLodClampGovernor {
    /// Active hardware sampler minimum LOD clamp (finest allowed mip index).
    active_min_lod_clamp: f32,
    /// Baseline fallback resident mip level (always present in VRAM).
    base_resident_mip: u8,
    /// Target ideal mip level requested by shaders.
    target_mip: u8,
}

impl TextureLodClampGovernor {
    /// Creates a new LOD clamp governor initialized to the base resident mip level.
    pub fn new(base_resident_mip: u8) -> Self {
        Self {
            active_min_lod_clamp: base_resident_mip as f32,
            base_resident_mip,
            target_mip: 0,
        }
    }

    /// Active minimum LOD clamp value (in fractional mip level units).
    #[inline]
    pub fn active_min_lod_clamp(&self) -> f32 {
        self.active_min_lod_clamp
    }

    /// Base resident fallback mip level.
    #[inline]
    pub fn base_resident_mip(&self) -> u8 {
        self.base_resident_mip
    }

    /// Target ideal mip level requested by GPU shaders.
    #[inline]
    pub fn target_mip(&self) -> u8 {
        self.target_mip
    }

    /// Updates the hardware sampler clamp based on current per-tile residency.
    pub fn update_clamp_from_residency(&mut self, residency: &TextureMipResidency) {
        let finest_resident = residency.finest_resident_mip();
        self.active_min_lod_clamp = finest_resident as f32;
    }

    /// Sets the target ideal mip level.
    pub fn set_target_mip(&mut self, target: u8) {
        self.target_mip = target;
    }
}

/// Kernel-Direct GPU Sampler Feedback Streaming & Texture LOD Clamp Engine.
/// Orchestrates paired sampler feedback maps (MinMipMap and TextureTranscoding),
#[derive(Debug)]
pub struct SamplerFeedbackEngine {
    /// Paired texture and feedback map descriptor.
    descriptor: FeedbackMapDescriptor,
    /// MinMipMap feedback storage surface.
    min_mip_map: MinMipMapStorage,
    /// Texture transcoding feedback storage surface.
    transcoding_map: TextureTranscodingStorage,
    /// Per-tile mip level residency tracker.
    residency: TextureMipResidency,
    /// Priority request queue for asynchronous tile streaming.
    request_queue: TileRequestQueue,
    /// Dynamic hardware sampler LOD clamp governor.
    lod_governor: TextureLodClampGovernor,
    /// Profiling and streaming telemetry.
    stats: SamplerFeedbackStats,
}

impl SamplerFeedbackEngine {
    /// Creates a new SamplerFeedbackEngine paired with a texture resource.
    pub fn new(
        descriptor: FeedbackMapDescriptor,
        base_resident_mip: u8,
        queue_capacity: usize,
    ) -> Self {
        let tx = descriptor.tiles_x;
        let ty = descriptor.tiles_y;

        Self {
            descriptor,
            min_mip_map: MinMipMapStorage::new(tx, ty),
            transcoding_map: TextureTranscodingStorage::new(tx, ty),
            residency: TextureMipResidency::new(tx, ty, base_resident_mip),
            request_queue: TileRequestQueue::new(queue_capacity),
            lod_governor: TextureLodClampGovernor::new(base_resident_mip),
            stats: SamplerFeedbackStats::default(),
        }
    }

    /// Paired feedback map descriptor.
    #[inline]
    pub fn descriptor(&self) -> &FeedbackMapDescriptor {
        &self.descriptor
    }

    /// Reference to MinMipMap storage.
    #[inline]
    pub fn min_mip_map(&self) -> &MinMipMapStorage {
        &self.min_mip_map
    }

    /// Mutably access MinMipMap storage.
    #[inline]
    pub fn min_mip_map_mut(&mut self) -> &mut MinMipMapStorage {
        &mut self.min_mip_map
    }

    /// Reference to Transcoding map storage.
    #[inline]
    pub fn transcoding_map(&self) -> &TextureTranscodingStorage {
        &self.transcoding_map
    }

    /// Reference to texture mip residency tracker.
    #[inline]
    pub fn residency(&self) -> &TextureMipResidency {
        &self.residency
    }

    /// Mutably access residency tracker.
    #[inline]
    pub fn residency_mut(&mut self) -> &mut TextureMipResidency {
        &mut self.residency
    }

    /// Reference to the tile request queue.
    #[inline]
    pub fn request_queue(&self) -> &TileRequestQueue {
        &self.request_queue
    }

    /// Mutably access tile request queue.
    #[inline]
    pub fn request_queue_mut(&mut self) -> &mut TileRequestQueue {
        &mut self.request_queue
    }

    /// Reference to the LOD clamp governor.
    #[inline]
    pub fn lod_governor(&self) -> &TextureLodClampGovernor {
        &self.lod_governor
    }

    /// Engine telemetry statistics.
    #[inline]
    pub fn stats(&self) -> &SamplerFeedbackStats {
        &self.stats
    }

    /// Records a simulated GPU shader texture sample operation into the feedback map.
    pub fn record_sample(&mut self, texel_x: u32, texel_y: u32, sampled_mip: u8) {
        let (tx, ty) = self.descriptor.texel_to_tile(texel_x, texel_y);
        self.min_mip_map.record_sample(tx, ty, sampled_mip);
        self.transcoding_map.mark_transcode_needed(tx, ty);
        self.stats.total_samples_recorded += 1;
    }

    /// Evaluates the feedback map against current VRAM residency.
    /// Identifies all tiles where `sampled_mip < resident_mip` (missing higher-resolution detail),
    pub fn evaluate_feedback(&mut self, texture_id: u32) -> Result<usize, SamplerFeedbackError> {
        let mut new_requests_count = 0;

        for ty in 0..self.descriptor.tiles_y {
            for tx in 0..self.descriptor.tiles_x {
                if let Some(sampled_mip) = self.min_mip_map.get(tx, ty) {
                    if sampled_mip != SAMPLER_FEEDBACK_UNACCESSED_MIP {
                        let resident_mip = self.residency.get_resident_mip(tx, ty);
                        if sampled_mip < resident_mip {
                            // Missing higher-resolution mip tile!
                            // Priority calculation: urgency scales with mip distance
                            let urgency = (resident_mip - sampled_mip) as u32 * 100;

                            let req = TileStreamingRequest::new(
                                texture_id,
                                tx,
                                ty,
                                sampled_mip,
                                urgency,
                            );

                            if self.request_queue.push(req)? {
                                new_requests_count += 1;
                                self.stats.tile_requests_generated += 1;
                            }
                        }
                    }
                }
            }
        }

        self.lod_governor.update_clamp_from_residency(&self.residency);
        self.stats.evaluations_performed += 1;

        Ok(new_requests_count)
    }

    /// Fulfills a streaming tile request by promoting tile residency in VRAM
    /// and updating the active sampler LOD clamp.
    pub fn fulfill_tile_request(&mut self, req: &TileStreamingRequest) -> bool {
        let promoted = self.residency.promote_tile_mip(req.tile_x, req.tile_y, req.requested_mip);
        if promoted {
            self.transcoding_map.mark_transcode_ready(req.tile_x, req.tile_y);
            self.lod_governor.update_clamp_from_residency(&self.residency);
            self.stats.tile_requests_fulfilled += 1;
        }
        promoted
    }

    /// Clears both MinMipMap and Transcoding feedback surfaces for the next frame.
    pub fn clear_feedback(&mut self) {
        self.min_mip_map.clear();
        self.transcoding_map.clear();
    }
}
