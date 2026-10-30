//! Direct3D 12 Dynamic Tile Pool Memory Compactor & Sparse Virtual Texture Eviction Engine.
//!
//! Implements Direct3D 12 Tier 2/3 Tiled Resources specification:
//! - Standard 64KB physical tile pool allocations (`D3D12_TILED_RESOURCE_TILE_SIZE_IN_BYTES`).
//! - Sparse virtual texture address spaces, multi-resolution mip tiling, and sub-resource coordinate mapping.
//! - Priority-driven and LRU-ordered tile residency tracking and zero-overhead NULL tile unmapping.
//! - Dynamic physical tile pool compaction and transparent page-table remapping.
//! - Memory pressure governor with automated unreferenced/low-LOD tile eviction and Min-LOD clamping.

extern crate alloc;

use alloc::vec::Vec;

/// Standard Direct3D 12 tile size in bytes (64 KiB).
pub const D3D12_TILED_RESOURCE_TILE_SIZE_IN_BYTES: usize = 65536;

/// Default tile width in texels for standard 128-bit / block-compressed textures.
pub const D3D12_STANDARD_TILE_WIDTH_TEXELS: u32 = 64;

/// Default tile height in texels for standard 128-bit / block-compressed textures.
pub const D3D12_STANDARD_TILE_HEIGHT_TEXELS: u32 = 64;

/// Sentinel ID indicating a NULL / unmapped physical tile.
pub const NULL_PHYSICAL_TILE_ID: u32 = u32::MAX;

/// Direct3D 12 Tiled Resource Hardware Support Tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TiledResourceTier {
    /// Tier 1: 2D textures only, no mip tails, no MinLOD clamping, unmapped reads undefined.
    Tier1,
    /// Tier 2: 2D textures, packed mip tail, MinLOD clamping, unmapped reads guaranteed zero.
    Tier2,
    /// Tier 3: 3D volumetric tiled resources, standard 3D tiling shapes.
    Tier3,
}

/// Residency state of a sparse virtual texture tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileResidencyState {
    /// Tile is unmapped (NULL tile). Shader reads return zero/black, writes are discarded.
    Unmapped,
    /// Tile is uniquely mapped to a dedicated 64KB physical tile in the pool.
    Resident { physical_tile_id: u32 },
    /// Tile shares a 64KB physical tile with one or more other virtual tiles (aliasing).
    Aliased { physical_tile_id: u32 },
}

impl TileResidencyState {
    /// Returns true if the tile has physical backing memory resident in the pool.
    #[inline(always)]
    pub const fn is_resident(&self) -> bool {
        match self {
            Self::Resident { .. } | Self::Aliased { .. } => true,
            Self::Unmapped => false,
        }
    }

    /// Returns the physical tile ID if resident, or `None` if unmapped.
    #[inline(always)]
    pub const fn physical_tile_id(&self) -> Option<u32> {
        match self {
            Self::Resident { physical_tile_id } | Self::Aliased { physical_tile_id } => {
                Some(*physical_tile_id)
            }
            Self::Unmapped => None,
        }
    }
}

/// Eviction priority levels for sparse virtual texture tiles under memory pressure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TileEvictionPriority {
    /// Low priority: fine LOD mips far from active view frustum or unreferenced.
    Low = 0,
    /// Normal priority: mid-range texture mips with moderate sampling probability.
    Normal = 1,
    /// High priority: near-field high-resolution visible texture mips.
    High = 2,
    /// Critical priority: base coarse mip level, mip tail, or actively bound render targets.
    Critical = 3,
}

/// Errors returned by the tile pool and compaction engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TilePoolError {
    /// Physical tile pool has no free 64KB slots remaining.
    OutOfMemory,
    /// The specified physical tile ID exceeds the pool capacity.
    InvalidTileId,
    /// The virtual tile coordinate is out of bounds for the subresource.
    InvalidCoordinate,
    /// Physical tile is currently locked/pinned and cannot be relocated or evicted.
    TilePinned,
    /// Inconsistent residency mapping detected during validation.
    ResidencyMismatch,
}

/// 3D subresource tile coordinate within a sparse virtual texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileCoordinate {
    /// X tile coordinate within the subresource.
    pub x: u32,
    /// Y tile coordinate within the subresource.
    pub y: u32,
    /// Z tile coordinate / array slice.
    pub z: u32,
    /// Subresource / mip-level index.
    pub subresource: u32,
}

impl TileCoordinate {
    /// Creates a new subresource tile coordinate.
    pub const fn new(x: u32, y: u32, z: u32, subresource: u32) -> Self {
        Self { x, y, z, subresource }
    }

    /// Computes the linearized index within a 2D subresource tile grid.
    #[inline(always)]
    pub const fn linear_index_2d(&self, tiles_x: u32) -> usize {
        (self.y as usize) * (tiles_x as usize) + (self.x as usize)
    }
}

/// Metadata descriptor for a single 64KB physical tile slot in the GPU tile pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalTileDescriptor {
    /// Physical tile index within the pool (0..capacity-1).
    pub physical_tile_id: u32,
    /// Whether this physical slot is currently allocated and mapped.
    pub is_allocated: bool,
    /// Reference count of virtual tiles aliased to this physical slot (1 = unique, >1 = aliased).
    pub reference_count: u32,
    /// Identifier of the primary owning sparse virtual texture.
    pub owner_texture_id: u32,
    /// Primary virtual coordinate mapped to this physical tile.
    pub virtual_coord: TileCoordinate,
    /// Monotonic timestamp of the most recent access/touch for LRU eviction.
    pub lru_timestamp: u64,
    /// Eviction priority tier of this tile.
    pub priority: TileEvictionPriority,
    /// Pinned flag: if true, tile cannot be relocated during compaction or evicted.
    pub is_pinned: bool,
    /// Dirty flag: indicates data was updated and requires GPU cache flush.
    pub is_dirty: bool,
}

impl PhysicalTileDescriptor {
    /// Creates an unallocated physical tile descriptor.
    pub const fn empty(physical_tile_id: u32) -> Self {
        Self {
            physical_tile_id,
            is_allocated: false,
            reference_count: 0,
            owner_texture_id: 0,
            virtual_coord: TileCoordinate::new(0, 0, 0, 0),
            lru_timestamp: 0,
            priority: TileEvictionPriority::Normal,
            is_pinned: false,
            is_dirty: false,
        }
    }
}

/// Standard 64KB Physical Tile Pool backing Direct3D 12 Tiled Resources.
#[derive(Debug, Clone)]
pub struct PhysicalTilePool {
    /// Contiguous array of physical 64KB tile metadata descriptors.
    pub tiles: Vec<PhysicalTileDescriptor>,
    /// Total number of 64KB tile slots in this pool.
    pub capacity: usize,
    /// Number of currently allocated 64KB tiles.
    pub allocated_count: usize,
    /// Monotonic hardware access clock used for LRU timestamps.
    pub clock: u64,
}

impl PhysicalTilePool {
    /// Creates a new physical tile pool with the specified 64KB tile capacity.
    pub fn new(capacity: usize) -> Self {
        let mut tiles = Vec::with_capacity(capacity);
        for id in 0..capacity {
            tiles.push(PhysicalTileDescriptor::empty(id as u32));
        }
        Self {
            tiles,
            capacity,
            allocated_count: 0,
            clock: 1,
        }
    }

    /// Allocates the next available 64KB physical tile slot from the pool.
    pub fn allocate_tile(
        &mut self,
        owner_texture_id: u32,
        coord: TileCoordinate,
        priority: TileEvictionPriority,
    ) -> Result<u32, TilePoolError> {
        for tile in self.tiles.iter_mut() {
            if !tile.is_allocated {
                tile.is_allocated = true;
                tile.reference_count = 1;
                tile.owner_texture_id = owner_texture_id;
                tile.virtual_coord = coord;
                tile.priority = priority;
                tile.lru_timestamp = self.clock;
                tile.is_pinned = false;
                tile.is_dirty = false;
                self.clock = self.clock.saturating_add(1);
                self.allocated_count += 1;
                return Ok(tile.physical_tile_id);
            }
        }
        Err(TilePoolError::OutOfMemory)
    }

    /// Increments reference count to alias an existing physical tile with another virtual coordinate.
    pub fn alias_tile(&mut self, physical_tile_id: u32) -> Result<(), TilePoolError> {
        let tile = self
            .tiles
            .get_mut(physical_tile_id as usize)
            .ok_or(TilePoolError::InvalidTileId)?;
        if !tile.is_allocated {
            return Err(TilePoolError::ResidencyMismatch);
        }
        tile.reference_count = tile.reference_count.saturating_add(1);
        tile.lru_timestamp = self.clock;
        self.clock = self.clock.saturating_add(1);
        Ok(())
    }

    /// Releases a reference to a physical tile. If reference count reaches 0, the tile is freed.
    pub fn free_or_unalias(&mut self, physical_tile_id: u32) -> Result<bool, TilePoolError> {
        let tile = self
            .tiles
            .get_mut(physical_tile_id as usize)
            .ok_or(TilePoolError::InvalidTileId)?;
        if !tile.is_allocated {
            return Ok(false);
        }
        if tile.reference_count > 1 {
            tile.reference_count -= 1;
            Ok(false)
        } else {
            *tile = PhysicalTileDescriptor::empty(physical_tile_id);
            self.allocated_count = self.allocated_count.saturating_sub(1);
            Ok(true)
        }
    }

    /// Updates the LRU access timestamp of an allocated physical tile.
    pub fn touch_tile(&mut self, physical_tile_id: u32) {
        if let Some(tile) = self.tiles.get_mut(physical_tile_id as usize) {
            if tile.is_allocated {
                tile.lru_timestamp = self.clock;
                self.clock = self.clock.saturating_add(1);
            }
        }
    }

    /// Pins or unpins a physical tile to protect it from eviction or compaction relocation.
    pub fn set_pinned(&mut self, physical_tile_id: u32, pinned: bool) -> Result<(), TilePoolError> {
        let tile = self
            .tiles
            .get_mut(physical_tile_id as usize)
            .ok_or(TilePoolError::InvalidTileId)?;
        tile.is_pinned = pinned;
        Ok(())
    }

    /// Returns the number of unallocated 64KB physical tiles available in the pool.
    #[inline(always)]
    pub fn free_tile_count(&self) -> usize {
        self.capacity.saturating_sub(self.allocated_count)
    }

    /// Returns the physical pool memory utilization ratio in `[0.0, 1.0]`.
    pub fn utilization_ratio(&self) -> f32 {
        if self.capacity == 0 {
            0.0
        } else {
            (self.allocated_count as f32) / (self.capacity as f32)
        }
    }

    /// Total capacity in bytes of this physical tile pool.
    #[inline(always)]
    pub const fn total_bytes(&self) -> usize {
        self.capacity * D3D12_TILED_RESOURCE_TILE_SIZE_IN_BYTES
    }

    /// Allocated memory in bytes in this physical tile pool.
    #[inline(always)]
    pub const fn allocated_bytes(&self) -> usize {
        self.allocated_count * D3D12_TILED_RESOURCE_TILE_SIZE_IN_BYTES
    }
}

/// Subresource tiling layout descriptor according to D3D12 Tiled Resource rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubresourceTiling {
    /// Mip-level index of this subresource (0 = base level).
    pub mip_level: u32,
    /// Width of the subresource in standard 64KB tiles.
    pub width_in_tiles: u32,
    /// Height of the subresource in standard 64KB tiles.
    pub height_in_tiles: u32,
    /// Depth of the subresource in standard 64KB tiles.
    pub depth_in_tiles: u32,
    /// Starting tile index in the overall virtual resource tile stream.
    pub start_tile_index_in_resource: usize,
    /// Total virtual tiles in this subresource.
    pub total_tiles: usize,
    /// Whether this subresource is packed into the standard D3D12 mip tail.
    pub is_packed_in_mip_tail: bool,
}

/// Configuration descriptor for a Direct3D 12 Sparse Virtual Texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SparseVirtualTextureDescriptor {
    /// Unique resource identifier.
    pub texture_id: u32,
    /// Width in pixels at base mip level 0.
    pub width: u32,
    /// Height in pixels at base mip level 0.
    pub height: u32,
    /// Depth in pixels / array slices.
    pub depth: u32,
    /// Number of mip-map levels in the pyramid.
    pub mip_levels: u32,
    /// Texel size in bytes (e.g., 4 for RGBA8, 16 for RGBA32F/BC7).
    pub bytes_per_pixel: u32,
    /// Hardware support tier.
    pub tier: TiledResourceTier,
    /// Tile width in texels.
    pub tile_width_texels: u32,
    /// Tile height in texels.
    pub tile_height_texels: u32,
}

impl SparseVirtualTextureDescriptor {
    /// Computes dimensions in pixels of a specific mip level.
    pub const fn mip_dimensions(&self, mip: u32) -> (u32, u32) {
        let w = (self.width >> mip).max(1);
        let h = (self.height >> mip).max(1);
        (w, h)
    }

    /// Computes tile grid dimensions `(tiles_x, tiles_y)` for a specific mip level.
    pub const fn mip_tile_dimensions(&self, mip: u32) -> (u32, u32) {
        let (w, h) = self.mip_dimensions(mip);
        let tx = (w + self.tile_width_texels - 1) / self.tile_width_texels;
        let ty = (h + self.tile_height_texels - 1) / self.tile_height_texels;
        (tx.max(1), ty.max(1))
    }
}

/// Sparse Virtual Texture Page-Table Mapping.
///
/// Maps virtual tile indices to their residency state (`Unmapped`, `Resident`, or `Aliased`).
#[derive(Debug, Clone)]
pub struct SparseTileMappingTable {
    /// Linear array of residency states for all virtual tiles in the resource.
    pub mappings: Vec<TileResidencyState>,
    /// Total number of virtual tiles in the address space.
    pub total_virtual_tiles: usize,
}

impl SparseTileMappingTable {
    /// Creates an empty mapping table with all virtual tiles initialized to `Unmapped`.
    pub fn new(total_virtual_tiles: usize) -> Self {
        let mappings = alloc::vec![TileResidencyState::Unmapped; total_virtual_tiles];
        Self {
            mappings,
            total_virtual_tiles,
        }
    }

    /// Queries residency state at a linear virtual tile index.
    #[inline(always)]
    pub fn get(&self, linear_idx: usize) -> TileResidencyState {
        self.mappings
            .get(linear_idx)
            .copied()
            .unwrap_or(TileResidencyState::Unmapped)
    }

    /// Sets residency state at a linear virtual tile index.
    #[inline(always)]
    pub fn set(&mut self, linear_idx: usize, state: TileResidencyState) {
        if let Some(entry) = self.mappings.get_mut(linear_idx) {
            *entry = state;
        }
    }

    /// Unmaps the specified virtual tile index back to NULL residency.
    #[inline(always)]
    pub fn unmap(&mut self, linear_idx: usize) {
        self.set(linear_idx, TileResidencyState::Unmapped);
    }
}

/// Direct3D 12 Sparse Virtual Texture Object.
///
/// Manages virtual tile address space, subresource tiling descriptors,
/// and residency mappings to 64KB physical tile pool allocations.
#[derive(Debug, Clone)]
pub struct SparseVirtualTexture {
    /// Resource configuration descriptor.
    pub desc: SparseVirtualTextureDescriptor,
    /// Subresource tiling descriptors per mip-level.
    pub subresources: Vec<SubresourceTiling>,
    /// Sparse virtual page-table mapping entries.
    pub mapping_table: SparseTileMappingTable,
    /// Minimum resident LOD clamp: shader texture sampling clamps to this mip level or coarser.
    pub min_resident_lod: f32,
}

impl SparseVirtualTexture {
    /// Constructs a new sparse virtual texture and computes subresource tiling layouts.
    pub fn new(desc: SparseVirtualTextureDescriptor) -> Self {
        let mut subresources = Vec::with_capacity(desc.mip_levels as usize);
        let mut current_tile_index = 0;

        for mip in 0..desc.mip_levels {
            let (tx, ty) = desc.mip_tile_dimensions(mip);
            let total = (tx as usize) * (ty as usize);
            // In Tier 2, mip levels requiring <= 64KB can be packed into mip tail
            let (mw, mh) = desc.mip_dimensions(mip);
            let byte_size = (mw as usize) * (mh as usize) * (desc.bytes_per_pixel as usize);
            let is_packed = desc.tier != TiledResourceTier::Tier1
                && mip > 0
                && byte_size <= D3D12_TILED_RESOURCE_TILE_SIZE_IN_BYTES;

            subresources.push(SubresourceTiling {
                mip_level: mip,
                width_in_tiles: tx,
                height_in_tiles: ty,
                depth_in_tiles: 1,
                start_tile_index_in_resource: current_tile_index,
                total_tiles: total,
                is_packed_in_mip_tail: is_packed,
            });

            current_tile_index += total;
        }

        let mapping_table = SparseTileMappingTable::new(current_tile_index);

        Self {
            desc,
            subresources,
            mapping_table,
            min_resident_lod: 0.0,
        }
    }

    /// Resolves a subresource coordinate to its linear index in the virtual address space.
    pub fn coord_to_linear_index(&self, coord: &TileCoordinate) -> Option<usize> {
        let sub = self.subresources.get(coord.subresource as usize)?;
        if coord.x >= sub.width_in_tiles || coord.y >= sub.height_in_tiles {
            return None;
        }
        let offset = (coord.y as usize) * (sub.width_in_tiles as usize) + (coord.x as usize);
        Some(sub.start_tile_index_in_resource + offset)
    }

    /// Resolves a linear index back to a subresource coordinate.
    pub fn linear_index_to_coord(&self, linear_idx: usize) -> Option<TileCoordinate> {
        if linear_idx >= self.mapping_table.total_virtual_tiles {
            return None;
        }
        for sub in self.subresources.iter() {
            let end = sub.start_tile_index_in_resource + sub.total_tiles;
            if linear_idx >= sub.start_tile_index_in_resource && linear_idx < end {
                let offset = (linear_idx - sub.start_tile_index_in_resource) as u32;
                let x = offset % sub.width_in_tiles;
                let y = offset / sub.width_in_tiles;
                return Some(TileCoordinate::new(x, y, 0, sub.mip_level));
            }
        }
        None
    }

    /// Queries the residency state of a virtual tile coordinate.
    pub fn get_residency(&self, coord: &TileCoordinate) -> TileResidencyState {
        match self.coord_to_linear_index(coord) {
            Some(idx) => self.mapping_table.get(idx),
            None => TileResidencyState::Unmapped,
        }
    }

    /// Maps a virtual tile coordinate to a dedicated resident physical tile ID.
    pub fn map_resident_tile(
        &mut self,
        coord: &TileCoordinate,
        physical_tile_id: u32,
    ) -> Result<(), TilePoolError> {
        let idx = self
            .coord_to_linear_index(coord)
            .ok_or(TilePoolError::InvalidCoordinate)?;
        self.mapping_table
            .set(idx, TileResidencyState::Resident { physical_tile_id });
        Ok(())
    }

    /// Maps a virtual tile coordinate to an aliased physical tile ID.
    pub fn map_aliased_tile(
        &mut self,
        coord: &TileCoordinate,
        physical_tile_id: u32,
    ) -> Result<(), TilePoolError> {
        let idx = self
            .coord_to_linear_index(coord)
            .ok_or(TilePoolError::InvalidCoordinate)?;
        self.mapping_table
            .set(idx, TileResidencyState::Aliased { physical_tile_id });
        Ok(())
    }

    /// Unmaps a virtual tile coordinate back to NULL.
    pub fn unmap_tile(&mut self, coord: &TileCoordinate) -> Result<(), TilePoolError> {
        let idx = self
            .coord_to_linear_index(coord)
            .ok_or(TilePoolError::InvalidCoordinate)?;
        self.mapping_table.unmap(idx);
        Ok(())
    }

    /// Counts the total number of currently resident virtual tiles.
    pub fn resident_tile_count(&self) -> usize {
        self.mapping_table
            .mappings
            .iter()
            .filter(|m| m.is_resident())
            .count()
    }

    /// Total number of virtual tiles in this texture's address space.
    #[inline(always)]
    pub const fn total_virtual_tiles(&self) -> usize {
        self.mapping_table.total_virtual_tiles
    }

    /// Transparently remaps any virtual references pointing to `old_physical_id`
    /// to point to `new_physical_id`.
    ///
    /// Returns the number of virtual tile entries updated.
    pub fn remap_physical_tile(&mut self, old_physical_id: u32, new_physical_id: u32) -> usize {
        let mut count = 0;
        for entry in self.mapping_table.mappings.iter_mut() {
            match entry {
                TileResidencyState::Resident { physical_tile_id } if *physical_tile_id == old_physical_id => {
                    *physical_tile_id = new_physical_id;
                    count += 1;
                }
                TileResidencyState::Aliased { physical_tile_id } if *physical_tile_id == old_physical_id => {
                    *physical_tile_id = new_physical_id;
                    count += 1;
                }
                _ => {}
            }
        }
        count
    }
}

// -----------------------------------------------------------------------------
// Tile Pool Dynamic Memory Compactor & Physical Page Relocation
// -----------------------------------------------------------------------------

/// Report on fragmentation and contiguous allocation headroom in the physical tile pool.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TilePoolFragmentationReport {
    /// Total 64KB physical tile capacity.
    pub total_tiles: usize,
    /// Number of allocated physical tiles.
    pub allocated_tiles: usize,
    /// Number of free physical tiles.
    pub free_tiles: usize,
    /// Number of free slots that precede allocated slots (compaction candidates / holes).
    pub hole_count: usize,
    /// Size of the largest contiguous sequence of free 64KB tiles.
    pub max_contiguous_free: usize,
    /// Fragmentation ratio in `[0.0, 1.0]`. 0.0 indicates perfect contiguous allocation.
    pub fragmentation_index: f32,
}

/// Audit log record for a relocated physical 64KB tile during compaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileRelocationRecord {
    /// Original physical tile index before compaction.
    pub src_physical_tile_id: u32,
    /// New compact physical tile index after relocation.
    pub dst_physical_tile_id: u32,
    /// Owner texture ID.
    pub owner_texture_id: u32,
    /// Virtual coordinate in the owning sparse virtual texture.
    pub virtual_coord: TileCoordinate,
    /// Number of virtual references that aliased this physical tile.
    pub reference_count: u32,
}

/// Detailed outcome of a dynamic compaction sweep.
#[derive(Debug, Clone, PartialEq)]
pub struct CompactionResult {
    /// Number of 64KB physical tiles relocated to lower indices.
    pub tiles_relocated: usize,
    /// Total bytes of GPU memory payload relocated.
    pub bytes_relocated: usize,
    /// Number of fragmentation holes before compaction.
    pub initial_holes: usize,
    /// Number of remaining fragmentation holes after compaction (0 unless pinned tiles blocked movement).
    pub remaining_holes: usize,
    /// Largest contiguous run of free physical tiles after compaction.
    pub max_contiguous_free_after: usize,
    /// List of physical tile relocations performed.
    pub relocations: Vec<TileRelocationRecord>,
}

/// Direct3D 12 Tile Pool Dynamic Memory Compactor.
///
/// Implements two-pointer zero-fragmentation compaction:
/// Relocates allocated 64KB physical tiles from high addresses into low unallocated holes,
/// transparently remapping all virtual texture page tables without coordinate disruption.
pub struct TilePoolCompactor;

impl TilePoolCompactor {
    /// Analyzes the physical tile pool to calculate fragmentation and contiguous run statistics.
    pub fn analyze_fragmentation(pool: &PhysicalTilePool) -> TilePoolFragmentationReport {
        let total_tiles = pool.capacity;
        let allocated_tiles = pool.allocated_count;
        let free_tiles = pool.free_tile_count();

        if total_tiles == 0 || allocated_tiles == 0 || free_tiles == 0 {
            return TilePoolFragmentationReport {
                total_tiles,
                allocated_tiles,
                free_tiles,
                hole_count: 0,
                max_contiguous_free: free_tiles,
                fragmentation_index: 0.0,
            };
        }

        // Find the index of the last allocated physical tile
        let mut last_allocated_idx = None;
        for (i, tile) in pool.tiles.iter().enumerate().rev() {
            if tile.is_allocated {
                last_allocated_idx = Some(i);
                break;
            }
        }

        let hole_count = match last_allocated_idx {
            Some(last_idx) => pool.tiles[..last_idx]
                .iter()
                .filter(|t| !t.is_allocated)
                .count(),
            None => 0,
        };

        // Compute maximum contiguous free run
        let mut max_contiguous_free = 0;
        let mut current_free_run = 0;
        for tile in pool.tiles.iter() {
            if !tile.is_allocated {
                current_free_run += 1;
                if current_free_run > max_contiguous_free {
                    max_contiguous_free = current_free_run;
                }
            } else {
                current_free_run = 0;
            }
        }

        let fragmentation_index = if free_tiles > 0 {
            (hole_count as f32) / (free_tiles as f32)
        } else {
            0.0
        };

        TilePoolFragmentationReport {
            total_tiles,
            allocated_tiles,
            free_tiles,
            hole_count,
            max_contiguous_free,
            fragmentation_index,
        }
    }

    /// Performs dynamic memory compaction on the physical tile pool, updating all
    /// sparse virtual textures' residency page tables transparently.
    pub fn compact(
        pool: &mut PhysicalTilePool,
        textures: &mut [SparseVirtualTexture],
    ) -> CompactionResult {
        let initial_report = Self::analyze_fragmentation(pool);
        let mut relocations = Vec::new();

        if initial_report.hole_count == 0 {
            return CompactionResult {
                tiles_relocated: 0,
                bytes_relocated: 0,
                initial_holes: 0,
                remaining_holes: 0,
                max_contiguous_free_after: initial_report.max_contiguous_free,
                relocations,
            };
        }

        let mut free_cursor = 0;
        let mut alloc_cursor = pool.capacity.saturating_sub(1);

        while free_cursor < alloc_cursor {
            // Advance free_cursor to next unallocated slot
            while free_cursor < alloc_cursor && pool.tiles[free_cursor].is_allocated {
                free_cursor += 1;
            }

            // Move alloc_cursor backward to next allocated slot
            while alloc_cursor > free_cursor && !pool.tiles[alloc_cursor].is_allocated {
                alloc_cursor -= 1;
            }

            if free_cursor >= alloc_cursor {
                break;
            }

            // Pinned tiles cannot be relocated; skip and keep searching downward
            if pool.tiles[alloc_cursor].is_pinned {
                alloc_cursor -= 1;
                continue;
            }

            let src_id = alloc_cursor as u32;
            let dst_id = free_cursor as u32;

            // Move physical descriptor
            let mut desc = pool.tiles[alloc_cursor];
            desc.physical_tile_id = dst_id;
            pool.tiles[free_cursor] = desc;
            pool.tiles[alloc_cursor] = PhysicalTileDescriptor::empty(src_id);

            // Transparently update all referencing sparse virtual texture page tables
            let mut total_remapped = 0;
            for tex in textures.iter_mut() {
                total_remapped += tex.remap_physical_tile(src_id, dst_id);
            }

            relocations.push(TileRelocationRecord {
                src_physical_tile_id: src_id,
                dst_physical_tile_id: dst_id,
                owner_texture_id: desc.owner_texture_id,
                virtual_coord: desc.virtual_coord,
                reference_count: total_remapped as u32,
            });

            free_cursor += 1;
            alloc_cursor -= 1;
        }

        let post_report = Self::analyze_fragmentation(pool);

        CompactionResult {
            tiles_relocated: relocations.len(),
            bytes_relocated: relocations.len() * D3D12_TILED_RESOURCE_TILE_SIZE_IN_BYTES,
            initial_holes: initial_report.hole_count,
            remaining_holes: post_report.hole_count,
            max_contiguous_free_after: post_report.max_contiguous_free,
            relocations,
        }
    }
}

