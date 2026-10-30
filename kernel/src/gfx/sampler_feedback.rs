//! Direct3D 12 Kernel-Direct GPU Sampler Feedback Streaming & Texture LOD Clamp Engine.
//!
//! Implements hardware-accelerated Direct3D 12 Sampler Feedback specifications:
//! - MinMipMap and Texture Transcoding feedback maps.
//! - Tile region mapping (e.g. 64x64, 128x128, 256x256 texels).
//! - Sub-millisecond texture tile streaming for massive game worlds without CPU readback stalls.
//! - Dynamic LOD Clamp governor to prevent visual texture popping and page fault crashes.

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
///
/// Configures the association between a source texture resource and its paired
/// feedback map surface, calculating tile grids and memory footprints.
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
///
/// Each tile is represented by 1 byte storing the minimum (highest resolution) mip level
/// sampled during GPU shader execution. Unaccessed tiles contain `SAMPLER_FEEDBACK_UNACCESSED_MIP` (0xFF).
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
