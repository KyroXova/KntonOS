//! Direct3D 12 Enhanced Barriers & Split-Barrier Synchronized GPU Resource Transition Engine.
//!
//! Replaces legacy monolithic ResourceBarrier state transitions with granular,
//! pipelined Enhanced Barriers:
//! - Granular Synchronization Scopes (`BarrierSync`) and Memory Access flags (`BarrierAccess`)
//! - Explicit Texture Compression & Memory Layout states (`BarrierLayout`)
//! - GlobalBarrier, BufferBarrier, and TextureBarrier with SubresourceRange matching
//! - Split-barrier execution (`BeginBarrier` / `EndBarrier`) maximizing GPU overlap and concurrency

use alloc::vec::Vec;

/// Constant representing all subresources of a texture resource.
pub const ALL_SUBRESOURCES: u32 = 0xFFFF_FFFF;

/// Direct3D 12 Synchronization Scopes (`D3D12_BARRIER_SYNC`).
///
/// Defines which hardware pipeline stages must complete before the barrier (sync_before)
/// and which stages must wait before executing (sync_after).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BarrierSync;

impl BarrierSync {
    pub const NONE: u32 = 0;
    pub const ALL: u32 = 1 << 0;
    pub const DRAW: u32 = 1 << 1;
    pub const INDEX_INPUT: u32 = 1 << 2;
    pub const VERTEX_SHADING: u32 = 1 << 3;
    pub const PIXEL_SHADING: u32 = 1 << 4;
    pub const DEPTH_STENCIL: u32 = 1 << 5;
    pub const RENDER_TARGET: u32 = 1 << 6;
    pub const COMPUTE: u32 = 1 << 7;
    pub const RAYTRACING: u32 = 1 << 8;
    pub const COPY: u32 = 1 << 9;
    pub const RESOLVE: u32 = 1 << 10;
    pub const EXECUTE_INDIRECT: u32 = 1 << 11;
    pub const ALL_SHADING: u32 = 1 << 12;
}

/// Direct3D 12 Memory Access Types (`D3D12_BARRIER_ACCESS`).
///
/// Defines which GPU memory caches must be flushed before the barrier (access_before)
/// and which caches must be invalidated before subsequent access (access_after).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BarrierAccess;

impl BarrierAccess {
    pub const COMMON: u32 = 0;
    pub const VERTEX_BUFFER: u32 = 1 << 0;
    pub const CONSTANT_BUFFER: u32 = 1 << 1;
    pub const INDEX_BUFFER: u32 = 1 << 2;
    pub const RENDER_TARGET: u32 = 1 << 3;
    pub const UNORDERED_ACCESS: u32 = 1 << 4;
    pub const DEPTH_STENCIL_WRITE: u32 = 1 << 5;
    pub const DEPTH_STENCIL_READ: u32 = 1 << 6;
    pub const SHADER_RESOURCE: u32 = 1 << 7;
    pub const COPY_SOURCE: u32 = 1 << 8;
    pub const COPY_DEST: u32 = 1 << 9;
    pub const RESOLVE_SOURCE: u32 = 1 << 10;
    pub const RESOLVE_DEST: u32 = 1 << 11;
    pub const RAYTRACING_ACCELERATION_STRUCTURE_READ: u32 = 1 << 12;
    pub const RAYTRACING_ACCELERATION_STRUCTURE_WRITE: u32 = 1 << 13;
    pub const NO_ACCESS: u32 = 1 << 14;
}

/// Direct3D 12 Texture Layouts (`D3D12_BARRIER_LAYOUT`).
///
/// Governs memory layout, compression metadata (DCC, HTILE), and cache configurations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u32)]
pub enum BarrierLayout {
    #[default]
    Undefined = 0,
    Common = 1,
    Present = 2,
    GenericRead = 3,
    RenderTarget = 4,
    UnorderedAccess = 5,
    DepthStencilWrite = 6,
    DepthStencilRead = 7,
    ShaderResource = 8,
    CopySource = 9,
    CopyDest = 10,
    ResolveSource = 11,
    ResolveDest = 12,
}

/// Flags modifying texture barrier behavior.
pub struct TextureBarrierFlags;

impl TextureBarrierFlags {
    pub const NONE: u32 = 0;
    /// Discards previous texture contents, skipping cache decompression/flushes.
    pub const DISCARD: u32 = 1 << 0;
}

/// Errors returned by the Enhanced Barrier subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarrierError {
    /// Sync scope bitmask is invalid or contains undefined bits.
    InvalidSyncScope(u32),
    /// Memory access bitmask is invalid.
    InvalidAccessFlags(u32),
    /// Transition between incompatible texture layouts.
    InvalidLayoutTransition { from: BarrierLayout, to: BarrierLayout },
    /// Subresource index or range exceeds texture mip/slice bounds.
    InvalidSubresourceRange,
    /// Buffer virtual address or size is invalid (e.g. zero size).
    InvalidBufferRange,
    /// Split barrier token mismatch or corrupted state.
    SplitBarrierMismatch,
    /// A split barrier with this token is already in flight.
    SplitBarrierAlreadyActive(u64),
    /// No matching active split barrier was found for EndBarrier.
    SplitBarrierNotFound(u64),
}

/// Subresource index range for texture barriers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubresourceRange {
    /// Starting mip level or `ALL_SUBRESOURCES`.
    pub index_or_first_mip: u32,
    /// Number of mip levels in this range.
    pub num_mips: u32,
    /// Starting array slice index.
    pub first_array_slice: u32,
    /// Number of array slices in this range.
    pub num_array_slices: u32,
    /// Starting plane slice index (e.g. stencil plane).
    pub first_plane: u32,
    /// Number of planes in this range.
    pub num_planes: u32,
}

impl Default for SubresourceRange {
    fn default() -> Self {
        Self::all()
    }
}

impl SubresourceRange {
    /// Represents all subresources across all mips, array slices, and planes.
    pub const fn all() -> Self {
        Self {
            index_or_first_mip: ALL_SUBRESOURCES,
            num_mips: 0,
            first_array_slice: 0,
            num_array_slices: 0,
            first_plane: 0,
            num_planes: 0,
        }
    }

    /// Single subresource at a specific mip level.
    pub const fn single_mip(mip: u32) -> Self {
        Self {
            index_or_first_mip: mip,
            num_mips: 1,
            first_array_slice: 0,
            num_array_slices: 1,
            first_plane: 0,
            num_planes: 1,
        }
    }

    /// Mip level slice range for arrayed texture.
    pub const fn mip_range(first_mip: u32, num_mips: u32, first_slice: u32, num_slices: u32) -> Self {
        Self {
            index_or_first_mip: first_mip,
            num_mips,
            first_array_slice: first_slice,
            num_array_slices: num_slices,
            first_plane: 0,
            num_planes: 1,
        }
    }

    /// Whether this range encompasses all subresources.
    #[inline]
    pub const fn is_all(&self) -> bool {
        self.index_or_first_mip == ALL_SUBRESOURCES
    }

    /// Evaluates if this subresource range overlaps with another.
    pub fn overlaps(&self, other: &Self) -> bool {
        if self.is_all() || other.is_all() {
            return true;
        }
        let mip_overlap = self.index_or_first_mip < other.index_or_first_mip + other.num_mips
            && other.index_or_first_mip < self.index_or_first_mip + self.num_mips;

        let slice_overlap = self.first_array_slice < other.first_array_slice + other.num_array_slices
            && other.first_array_slice < self.first_array_slice + self.num_array_slices;

        let plane_overlap = self.first_plane < other.first_plane + other.num_planes
            && other.first_plane < self.first_plane + self.num_planes;

        mip_overlap && slice_overlap && plane_overlap
    }
}

/// Global memory and execution barrier across all GPU resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlobalBarrier {
    /// Pipeline stages that must complete before this barrier.
    pub sync_before: u32,
    /// Pipeline stages that must wait for this barrier.
    pub sync_after: u32,
    /// Memory caches to flush before this barrier.
    pub access_before: u32,
    /// Memory caches to invalidate after this barrier.
    pub access_after: u32,
}

impl GlobalBarrier {
    pub const fn new(sync_before: u32, sync_after: u32, access_before: u32, access_after: u32) -> Self {
        Self {
            sync_before,
            sync_after,
            access_before,
            access_after,
        }
    }

    /// Validates the global barrier fields.
    pub fn validate(&self) -> Result<(), BarrierError> {
        if self.sync_before == 0 || self.sync_after == 0 {
            return Err(BarrierError::InvalidSyncScope(0));
        }
        Ok(())
    }
}

/// Buffer-specific memory and synchronization barrier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferBarrier {
    /// Pipeline stages completing before this barrier.
    pub sync_before: u32,
    /// Pipeline stages waiting for this barrier.
    pub sync_after: u32,
    /// Memory caches to flush.
    pub access_before: u32,
    /// Memory caches to invalidate.
    pub access_after: u32,
    /// GPU virtual address of the buffer.
    pub buffer_gpu_va: u64,
    /// Byte offset within the buffer.
    pub offset: u64,
    /// Size in bytes of the transitioned region.
    pub size: u64,
}

impl BufferBarrier {
    pub const fn new(
        sync_before: u32,
        sync_after: u32,
        access_before: u32,
        access_after: u32,
        buffer_gpu_va: u64,
        offset: u64,
        size: u64,
    ) -> Self {
        Self {
            sync_before,
            sync_after,
            access_before,
            access_after,
            buffer_gpu_va,
            offset,
            size,
        }
    }

    /// Validates the buffer barrier parameters.
    pub fn validate(&self) -> Result<(), BarrierError> {
        if self.sync_before == 0 || self.sync_after == 0 {
            return Err(BarrierError::InvalidSyncScope(0));
        }
        if self.size == 0 {
            return Err(BarrierError::InvalidBufferRange);
        }
        Ok(())
    }
}

/// Texture-specific subresource transition and layout barrier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureBarrier {
    /// Pipeline stages completing before this barrier.
    pub sync_before: u32,
    /// Pipeline stages waiting for this barrier.
    pub sync_after: u32,
    /// Memory caches to flush.
    pub access_before: u32,
    /// Memory caches to invalidate.
    pub access_after: u32,
    /// Layout of the texture before the barrier.
    pub layout_before: BarrierLayout,
    /// Layout of the texture after the barrier.
    pub layout_after: BarrierLayout,
    /// Texture resource ID.
    pub texture_id: u32,
    /// Subresources affected by this barrier.
    pub subresources: SubresourceRange,
    /// Texture barrier flags (e.g. Discard).
    pub flags: u32,
}

impl TextureBarrier {
    pub const fn new(
        sync_before: u32,
        sync_after: u32,
        access_before: u32,
        access_after: u32,
        layout_before: BarrierLayout,
        layout_after: BarrierLayout,
        texture_id: u32,
        subresources: SubresourceRange,
        flags: u32,
    ) -> Self {
        Self {
            sync_before,
            sync_after,
            access_before,
            access_after,
            layout_before,
            layout_after,
            texture_id,
            subresources,
            flags,
        }
    }

    /// Validates the texture barrier parameters.
    pub fn validate(&self) -> Result<(), BarrierError> {
        if self.sync_before == 0 || self.sync_after == 0 {
            return Err(BarrierError::InvalidSyncScope(0));
        }
        // Layout validation: cannot transition to Undefined layout
        if self.layout_after == BarrierLayout::Undefined {
            return Err(BarrierError::InvalidLayoutTransition {
                from: self.layout_before,
                to: self.layout_after,
            });
        }
        Ok(())
    }
}

/// Group of heterogeneous barriers passed to a barrier execution command.
#[derive(Debug, Clone, Default)]
pub struct BarrierGroup {
    pub global_barriers: Vec<GlobalBarrier>,
    pub buffer_barriers: Vec<BufferBarrier>,
    pub texture_barriers: Vec<TextureBarrier>,
}

impl BarrierGroup {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_global(&mut self, barrier: GlobalBarrier) -> Result<(), BarrierError> {
        barrier.validate()?;
        self.global_barriers.push(barrier);
        Ok(())
    }

    pub fn add_buffer(&mut self, barrier: BufferBarrier) -> Result<(), BarrierError> {
        barrier.validate()?;
        self.buffer_barriers.push(barrier);
        Ok(())
    }

    pub fn add_texture(&mut self, barrier: TextureBarrier) -> Result<(), BarrierError> {
        barrier.validate()?;
        self.texture_barriers.push(barrier);
        Ok(())
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.global_barriers.is_empty()
            && self.buffer_barriers.is_empty()
            && self.texture_barriers.is_empty()
    }

    #[inline]
    pub fn total_barriers(&self) -> usize {
        self.global_barriers.len() + self.buffer_barriers.len() + self.texture_barriers.len()
    }
}
