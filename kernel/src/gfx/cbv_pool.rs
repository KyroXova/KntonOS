//! Direct3D 12 Dynamic Constant Buffer View (CBV) Descriptor Pool & Binder.
//!
//! Manages dynamic shader constants, GPU virtual addresses (GPU VA), root descriptor slots,
//! and per-draw constant update streams:
//! - Strict D3D12 constant buffer size clamping (up to 64 KiB / 4096 16-byte elements)
//! - Per-draw suballocation and automatic 256-byte alignment
//! - Dirty bitmask tracking for root descriptor bindings
//! - Shadow CPU staging and multi-frame retirement synchronization

use crate::gfx::ring_alloc::{
    ConstantBufferRingAllocator, ConstantBufferSlice, FrameRetirementTracker, RingAllocError,
};

// -----------------------------------------------------------------------------
// Constants and Errors
// -----------------------------------------------------------------------------

/// Maximum allowable constant buffer size in Direct3D 12 (64 KiB).
pub const D3D12_MAX_CONSTANT_BUFFER_SIZE_BYTES: usize = 65536;

/// Maximum number of root signature parameters supported.
pub const D3D12_MAX_ROOT_PARAMETERS: usize = 16;

/// Shader pipeline visibility for a constant buffer view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderVisibility {
    All,
    Vertex,
    Pixel,
    Compute,
}

/// Errors returned by the dynamic CBV pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CbvPoolError {
    InvalidRootParameterIndex { index: u32, max: u32 },
    ConstantBufferSizeExceedsMax { size: usize, max: usize },
    RingAlloc(RingAllocError),
}

impl From<RingAllocError> for CbvPoolError {
    fn from(err: RingAllocError) -> Self {
        CbvPoolError::RingAlloc(err)
    }
}

// -----------------------------------------------------------------------------
// Dynamic CBV Descriptor
// -----------------------------------------------------------------------------

/// Direct3D 12 Constant Buffer View (CBV) descriptor representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DynamicCbvDescriptor {
    /// GPU Virtual Address (GPU VA) aligned to 256 bytes.
    pub buffer_location_gpu_va: u64,
    /// Size of constant buffer in bytes (aligned to 256 bytes).
    pub size_in_bytes: u32,
    /// Root parameter index in the active root signature.
    pub root_parameter_index: u32,
    /// Target shader stage visibility.
    pub visibility: ShaderVisibility,
    /// Ring buffer byte offset.
    pub ring_offset: usize,
}

// -----------------------------------------------------------------------------
// Dynamic CBV Pool
// -----------------------------------------------------------------------------

/// High-performance pool and binder managing per-draw dynamic constant buffers.
pub struct DynamicCbvPool {
    /// Underlying 256-byte aligned ring buffer allocator.
    pub ring: ConstantBufferRingAllocator,
    /// Multi-frame retirement tracker synchronized with GPU timeline fences.
    pub tracker: FrameRetirementTracker,
    /// Currently bound CBV descriptors per root parameter index.
    pub bound_descriptors: [Option<DynamicCbvDescriptor>; D3D12_MAX_ROOT_PARAMETERS],
    /// Bitmask indicating which root parameters have been updated since last draw dispatch.
    pub dirty_mask: u16,
    /// Total number of draw constant updates committed.
    pub total_draw_updates: u64,
}

impl DynamicCbvPool {
    /// Creates a new dynamic CBV pool with specified ring capacity and base GPU VA.
    pub fn new(capacity: usize, base_gpu_va: u64) -> Self {
        Self {
            ring: ConstantBufferRingAllocator::new(capacity, base_gpu_va),
            tracker: FrameRetirementTracker::new(),
            bound_descriptors: [None; D3D12_MAX_ROOT_PARAMETERS],
            dirty_mask: 0,
            total_draw_updates: 0,
        }
    }

    /// Begins recording for a new frame.
    pub fn begin_frame(&mut self, frame_index: u64) {
        self.tracker.begin_frame(frame_index, &self.ring);
        self.dirty_mask = 0;
    }

    /// Ends recording for the current frame and registers its GPU fence sequence value.
    pub fn end_frame(&mut self, fence_value: u64) {
        self.tracker.end_frame(&self.ring, fence_value);
    }

    /// Retires completed frames up to `completed_fence_value` to reclaim ring memory.
    pub fn retire_completed_frames(&mut self, completed_fence_value: u64) -> usize {
        self.tracker.retire_completed_frames(&mut self.ring, completed_fence_value)
    }

    /// Stages a slice of raw CPU constant data into the ring buffer and binds it to a root parameter slot.
    ///
    /// - Enforces 256-byte alignment on both GPU VA and size.
    /// - Clamps buffer size to D3D12 maximum (64 KiB).
    /// - Copies CPU bytes to staging memory.
    /// - Marks the root parameter as dirty for the next draw command.
    pub fn stage_constants(
        &mut self,
        root_param_index: u32,
        visibility: ShaderVisibility,
        data: &[u8],
    ) -> Result<DynamicCbvDescriptor, CbvPoolError> {
        let idx = root_param_index as usize;
        if idx >= D3D12_MAX_ROOT_PARAMETERS {
            return Err(CbvPoolError::InvalidRootParameterIndex {
                index: root_param_index,
                max: D3D12_MAX_ROOT_PARAMETERS as u32,
            });
        }

        if data.len() > D3D12_MAX_CONSTANT_BUFFER_SIZE_BYTES {
            return Err(CbvPoolError::ConstantBufferSizeExceedsMax {
                size: data.len(),
                max: D3D12_MAX_CONSTANT_BUFFER_SIZE_BYTES,
            });
        }

        let slice = self.ring.allocate_and_write(data)?;
        self.tracker.current_frame_bytes += slice.aligned_size_bytes;

        let descriptor = DynamicCbvDescriptor {
            buffer_location_gpu_va: slice.gpu_va,
            size_in_bytes: slice.aligned_size_bytes as u32,
            root_parameter_index: root_param_index,
            visibility,
            ring_offset: slice.ring_offset,
        };

        self.bound_descriptors[idx] = Some(descriptor);
        self.dirty_mask |= 1 << idx;
        self.total_draw_updates += 1;

        Ok(descriptor)
    }

    /// Binds an existing pre-allocated constant buffer slice to a root parameter slot.
    pub fn bind_slice(
        &mut self,
        root_param_index: u32,
        visibility: ShaderVisibility,
        slice: &ConstantBufferSlice,
    ) -> Result<DynamicCbvDescriptor, CbvPoolError> {
        let idx = root_param_index as usize;
        if idx >= D3D12_MAX_ROOT_PARAMETERS {
            return Err(CbvPoolError::InvalidRootParameterIndex {
                index: root_param_index,
                max: D3D12_MAX_ROOT_PARAMETERS as u32,
            });
        }

        let descriptor = DynamicCbvDescriptor {
            buffer_location_gpu_va: slice.gpu_va,
            size_in_bytes: slice.aligned_size_bytes as u32,
            root_parameter_index: root_param_index,
            visibility,
            ring_offset: slice.ring_offset,
        };

        self.bound_descriptors[idx] = Some(descriptor);
        self.dirty_mask |= 1 << idx;
        self.total_draw_updates += 1;

        Ok(descriptor)
    }

    /// Commits the current draw bindings, clearing the dirty bitmask and returning the previous dirty mask.
    pub fn commit_draw(&mut self) -> u16 {
        let mask = self.dirty_mask;
        self.dirty_mask = 0;
        mask
    }

    /// Queries the currently bound descriptor for a given root parameter slot.
    pub fn get_binding(&self, root_param_index: u32) -> Option<DynamicCbvDescriptor> {
        let idx = root_param_index as usize;
        if idx < D3D12_MAX_ROOT_PARAMETERS {
            self.bound_descriptors[idx]
        } else {
            None
        }
    }

    /// Verifies if a given root parameter slot is dirty (updated since last draw).
    pub fn is_dirty(&self, root_param_index: u32) -> bool {
        if root_param_index < D3D12_MAX_ROOT_PARAMETERS as u32 {
            (self.dirty_mask & (1 << root_param_index)) != 0
        } else {
            false
        }
    }
}

// -----------------------------------------------------------------------------
// Unit Tests & Verification
// -----------------------------------------------------------------------------

/// Unit test verifying dynamic CBV heap descriptor binder and per-draw constant update stream.
pub fn verify_dynamic_cbv_pool_and_binding() -> bool {
    let capacity = 64 * 1024; // 64 KiB
    let base_gpu_va = 0xA000_0000_u64;
    let mut pool = DynamicCbvPool::new(capacity, base_gpu_va);

    // 1. Begin Frame 1
    pool.begin_frame(1);
    if pool.dirty_mask != 0 {
        return false;
    }

    // 2. Stage Per-Draw Constants for Slot 0 (b0 = Model-View-Projection Matrix, 64 bytes)
    #[repr(C, align(16))]
    struct MvpConstants {
        matrix: [f32; 16],
    }
    let mvp = MvpConstants {
        matrix: [
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            0.0, 0.0, 0.0, 1.0,
        ],
    };
    let mvp_bytes = unsafe {
        core::slice::from_raw_parts(
            &mvp as *const MvpConstants as *const u8,
            core::mem::size_of::<MvpConstants>(),
        )
    };

    let desc0 = match pool.stage_constants(0, ShaderVisibility::Vertex, mvp_bytes) {
        Ok(d) => d,
        Err(_) => return false,
    };

    // Verify 256-byte alignment of GPU VA and size
    if desc0.buffer_location_gpu_va != base_gpu_va || (desc0.buffer_location_gpu_va % 256 != 0) {
        return false;
    }
    if desc0.size_in_bytes != 256 || desc0.root_parameter_index != 0 {
        return false;
    }
    if !pool.is_dirty(0) || pool.dirty_mask != (1 << 0) {
        return false;
    }

    // 3. Stage Per-Draw Constants for Slot 1 (b1 = Material / Light Properties, 32 bytes)
    let material_bytes = [0x55_u8; 32];
    let desc1 = match pool.stage_constants(1, ShaderVisibility::Pixel, &material_bytes) {
        Ok(d) => d,
        Err(_) => return false,
    };
    if desc1.buffer_location_gpu_va != base_gpu_va + 256 || (desc1.buffer_location_gpu_va % 256 != 0) {
        return false;
    }
    if desc1.size_in_bytes != 256 || desc1.root_parameter_index != 1 {
        return false;
    }
    if !pool.is_dirty(1) || pool.dirty_mask != ((1 << 0) | (1 << 1)) {
        return false;
    }

    // 4. Commit Draw 1
    let committed_mask = pool.commit_draw();
    if committed_mask != ((1 << 0) | (1 << 1)) {
        return false;
    }
    if pool.dirty_mask != 0 {
        return false; // Dirty mask must be cleared after draw commit
    }

    // 5. Draw 2: Update only Slot 0 (new MVP for next object)
    let desc0_draw2 = match pool.stage_constants(0, ShaderVisibility::Vertex, mvp_bytes) {
        Ok(d) => d,
        Err(_) => return false,
    };
    if desc0_draw2.buffer_location_gpu_va != base_gpu_va + 512 {
        return false;
    }
    if !pool.is_dirty(0) || pool.is_dirty(1) {
        return false; // Only slot 0 is dirty for Draw 2
    }
    pool.commit_draw();

    // 6. End Frame 1 and Retire with Fence = 100
    pool.end_frame(100);
    if pool.tracker.in_flight_frame_count() != 1 {
        return false;
    }

    let reclaimed = pool.retire_completed_frames(100);
    if reclaimed != 768 || pool.tracker.in_flight_frame_count() != 0 {
        return false;
    }

    // 7. Error Handling: Requesting >64 KiB must fail
    let oversize_data = [0u8; 65537];
    if pool.stage_constants(0, ShaderVisibility::All, &oversize_data).is_ok() {
        return false;
    }

    // Requesting invalid slot index >= 16 must fail
    if pool.stage_constants(16, ShaderVisibility::All, &material_bytes).is_ok() {
        return false;
    }

    true
}
