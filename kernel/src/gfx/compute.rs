//! Asynchronous Compute Command Stream Encoder & Dispatch Queue Subsystem.
//!
//! Implements modern low-level asynchronous compute command encoding (Direct3D 12 / Vulkan model):
//! - Independent async compute command buffers bypassing graphics pipeline overhead
//! - 3D workgroup dispatch recording (`Dispatch(x, y, z)`)
//! - Compute Pipeline State Objects (PSO) with local threadgroup dimension tracking
//! - Resource descriptor tables (UAVs, SRVs, CBVs) and push constant data streams
//! - Fine-grained memory barrier synchronization (UAV barriers & Transition barriers)
//! - Concurrent workgroup pipeline scheduler running async compute alongside graphics rasterization
//! - High-speed asynchronous compute submission queue

#![allow(dead_code)]

extern crate alloc;
use alloc::vec::Vec;
use crate::gfx::cmdbuf::{PipelineStageFlags, ResourceState};
use crate::gfx::gpu_fence::{GpuTimelineFence, TimelineSyncHub};

// -----------------------------------------------------------------------------
// Workgroup Dimensions & Pipeline State Objects (PSO)
// -----------------------------------------------------------------------------

/// Threadgroup local dimensions for compute kernel execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadgroupDimension {
    pub x: u32,
    pub y: u32,
    pub z: u32,
}

impl ThreadgroupDimension {
    pub const fn new(x: u32, y: u32, z: u32) -> Self {
        Self { x, y, z }
    }

    /// Total number of compute threads within a single threadgroup / workgroup.
    #[inline(always)]
    pub const fn total_threads(&self) -> u32 {
        self.x * self.y * self.z
    }
}

/// Compute Pipeline State Object (PSO) encapsulating compute shader kernel state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComputePipelineState {
    pub pso_id: u32,
    pub kernel_gpu_address: u64,
    pub threadgroup_size: ThreadgroupDimension,
    pub shared_memory_bytes: u32,
    pub root_signature_id: u32,
}

impl ComputePipelineState {
    pub const fn new(
        pso_id: u32,
        kernel_gpu_address: u64,
        threadgroup_size: ThreadgroupDimension,
        shared_memory_bytes: u32,
        root_signature_id: u32,
    ) -> Self {
        Self {
            pso_id,
            kernel_gpu_address,
            threadgroup_size,
            shared_memory_bytes,
            root_signature_id,
        }
    }
}

// -----------------------------------------------------------------------------
// Resource Descriptors & Descriptor Tables
// -----------------------------------------------------------------------------

/// Resource descriptor types for shader binding.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptorType {
    /// Unordered Access View (UAV) - Read/Write buffer or texture.
    Uav = 0,
    /// Shader Resource View (SRV) - Read-only sampled buffer or texture.
    Srv = 1,
    /// Constant Buffer View (CBV) - Shader uniforms and parameters.
    Cbv = 2,
    /// Sampler State - Bilinear, trilinear, anisotropic texture samplers.
    Sampler = 3,
}

/// A single descriptor table entry binding a GPU resource to a shader register slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DescriptorSlot {
    pub slot: u32,
    pub descriptor_type: DescriptorType,
    pub gpu_virtual_address: u64,
    pub size_bytes: u32,
    pub stride_bytes: u32,
}

impl DescriptorSlot {
    pub const fn new(
        slot: u32,
        descriptor_type: DescriptorType,
        gpu_virtual_address: u64,
        size_bytes: u32,
        stride_bytes: u32,
    ) -> Self {
        Self {
            slot,
            descriptor_type,
            gpu_virtual_address,
            size_bytes,
            stride_bytes,
        }
    }
}

/// Descriptor table binding resource descriptors to a shader root parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescriptorTable {
    pub root_index: u32,
    pub slots: Vec<DescriptorSlot>,
}

impl DescriptorTable {
    pub fn new(root_index: u32) -> Self {
        Self {
            root_index,
            slots: Vec::new(),
        }
    }

    pub fn add_slot(&mut self, slot: DescriptorSlot) {
        self.slots.push(slot);
    }

    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }
}

// -----------------------------------------------------------------------------
// Fine-Grained Memory Barrier Synchronization
// -----------------------------------------------------------------------------

/// Detailed barrier classification for compute pipeline transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarrierKind {
    /// UAV memory barrier: flushes L1/L2 caches ensuring preceding compute writes are visible.
    Uav { resource_gpu_address: u64 },
    /// Resource state transition barrier: e.g. UNORDERED_ACCESS -> PIXEL_SHADER_RESOURCE.
    Transition {
        resource_gpu_address: u64,
        state_before: ResourceState,
        state_after: ResourceState,
        src_stage: PipelineStageFlags,
        dst_stage: PipelineStageFlags,
    },
}

/// Fine-grained memory barrier ensuring cache coherence and stage ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComputeMemoryBarrier {
    pub barrier_id: u32,
    pub kind: BarrierKind,
}

impl ComputeMemoryBarrier {
    pub const fn uav(barrier_id: u32, resource_gpu_address: u64) -> Self {
        Self {
            barrier_id,
            kind: BarrierKind::Uav { resource_gpu_address },
        }
    }

    pub const fn transition(
        barrier_id: u32,
        resource_gpu_address: u64,
        state_before: ResourceState,
        state_after: ResourceState,
        src_stage: PipelineStageFlags,
        dst_stage: PipelineStageFlags,
    ) -> Self {
        Self {
            barrier_id,
            kind: BarrierKind::Transition {
                resource_gpu_address,
                state_before,
                state_after,
                src_stage,
                dst_stage,
            },
        }
    }

    pub fn is_uav(&self) -> bool {
        matches!(self.kind, BarrierKind::Uav { .. })
    }

    /// Verifies that state transitions obey valid hardware rules.
    pub fn is_valid_transition(&self) -> bool {
        match self.kind {
            BarrierKind::Uav { .. } => true,
            BarrierKind::Transition { state_before, state_after, .. } => {
                // Must transition between different states or valid flags
                state_before != state_after
            }
        }
    }
}

/// Simulated GPU Cache Hierarchy State.
#[derive(Debug, Default, Clone)]
pub struct GpuCacheState {
    pub l1_dirty_lines: u32,
    pub l2_dirty_lines: u32,
    pub l1_flushes: u32,
    pub l2_flushes: u32,
    pub read_invalidations: u32,
}

impl GpuCacheState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn flush_uav_caches(&mut self) {
        self.l1_flushes += 1;
        self.l2_flushes += 1;
        self.l1_dirty_lines = 0;
        self.l2_dirty_lines = 0;
    }

    pub fn invalidate_reads(&mut self) {
        self.read_invalidations += 1;
    }
}

// -----------------------------------------------------------------------------
// Asynchronous Compute Command Opcodes & Payload Structures
// -----------------------------------------------------------------------------

/// Command opcodes for async compute streams.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputeOpcode {
    Nop = 0,
    Dispatch = 1,
    SetPipelineState = 2,
    SetDescriptorTable = 3,
    SetPushConstants = 4,
    ResourceBarrier = 5,
    SignalFence = 6,
    WaitFence = 7,
}

/// Payload for a 3D threadgroup compute dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DispatchPayload {
    pub count_x: u32,
    pub count_y: u32,
    pub count_z: u32,
}

impl DispatchPayload {
    pub const fn new(count_x: u32, count_y: u32, count_z: u32) -> Self {
        Self { count_x, count_y, count_z }
    }

    #[inline(always)]
    pub const fn total_threadgroups(&self) -> u64 {
        (self.count_x as u64) * (self.count_y as u64) * (self.count_z as u64)
    }
}

/// High-level compute command variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComputeCommand {
    Dispatch(DispatchPayload),
    SetPipelineState(ComputePipelineState),
    SetDescriptorTable(DescriptorTable),
    SetPushConstants { offset: u32, data: Vec<u8> },
    ResourceBarrier(ComputeMemoryBarrier),
    SignalFence { fence_id: u32, value: u64 },
    WaitFence { fence_id: u32, value: u64 },
}

// -----------------------------------------------------------------------------
// Asynchronous Compute Command Buffer
// -----------------------------------------------------------------------------

/// Asynchronous compute command buffer recording linear compute streams.
#[derive(Debug, Clone)]
pub struct AsyncComputeCommandBuffer {
    pub buffer_id: u32,
    pub commands: Vec<ComputeCommand>,
    pub total_workgroups: u64,
    pub active_pso: Option<ComputePipelineState>,
}

impl AsyncComputeCommandBuffer {
    pub fn new(buffer_id: u32) -> Self {
        Self {
            buffer_id,
            commands: Vec::new(),
            total_workgroups: 0,
            active_pso: None,
        }
    }

    /// Sets the active compute pipeline state object.
    pub fn set_pipeline_state(&mut self, pso: ComputePipelineState) {
        self.active_pso = Some(pso);
        self.commands.push(ComputeCommand::SetPipelineState(pso));
    }

    /// Binds a descriptor table to a root parameter slot.
    pub fn set_descriptor_table(&mut self, table: DescriptorTable) {
        self.commands.push(ComputeCommand::SetDescriptorTable(table));
    }

    /// Records a fine-grained memory barrier.
    pub fn resource_barrier(&mut self, barrier: ComputeMemoryBarrier) {
        self.commands.push(ComputeCommand::ResourceBarrier(barrier));
    }

    /// Records a UAV memory barrier directly.
    pub fn uav_barrier(&mut self, barrier_id: u32, resource_gpu_address: u64) {
        self.resource_barrier(ComputeMemoryBarrier::uav(barrier_id, resource_gpu_address));
    }

    /// Records a transition barrier between resource states.
    pub fn transition_barrier(
        &mut self,
        barrier_id: u32,
        resource_gpu_address: u64,
        before: ResourceState,
        after: ResourceState,
        src_stage: PipelineStageFlags,
        dst_stage: PipelineStageFlags,
    ) {
        self.resource_barrier(ComputeMemoryBarrier::transition(
            barrier_id,
            resource_gpu_address,
            before,
            after,
            src_stage,
            dst_stage,
        ));
    }

    /// Dispatches a 3D grid of threadgroups.
    pub fn dispatch(&mut self, count_x: u32, count_y: u32, count_z: u32) -> Result<(), &'static str> {
        if self.active_pso.is_none() {
            return Err("Compute pipeline state must be bound before dispatch");
        }
        if count_x == 0 || count_y == 0 || count_z == 0 {
            return Err("Dispatch threadgroup dimensions must be strictly positive");
        }
        let payload = DispatchPayload::new(count_x, count_y, count_z);
        self.total_workgroups += payload.total_threadgroups();
        self.commands.push(ComputeCommand::Dispatch(payload));
        Ok(())
    }

    /// Records push constants for fast shader parameter updates.
    pub fn set_push_constants(&mut self, offset: u32, data: &[u8]) {
        self.commands.push(ComputeCommand::SetPushConstants {
            offset,
            data: data.to_vec(),
        });
    }

    /// Records a signal fence operation at this point in the compute stream.
    pub fn signal_fence(&mut self, fence_id: u32, value: u64) {
        self.commands.push(ComputeCommand::SignalFence { fence_id, value });
    }

    /// Records a wait fence operation blocking subsequent compute commands until the fence is satisfied.
    pub fn wait_fence(&mut self, fence_id: u32, value: u64) {
        self.commands.push(ComputeCommand::WaitFence { fence_id, value });
    }

    #[inline(always)]
    pub fn command_count(&self) -> usize {
        self.commands.len()
    }
}

// -----------------------------------------------------------------------------
// Workgroup Workloads & Concurrent Pipeline Scheduler
// -----------------------------------------------------------------------------

/// Classification of asynchronous compute workloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputeWorkloadType {
    PhysicsSimulation,
    GpuSkinning,
    FrustumCulling,
    RaytracingDenoise,
}

/// Complete compute workload specification for scheduling.
#[derive(Debug, Clone)]
pub struct ComputeShaderWorkload {
    pub workload_type: ComputeWorkloadType,
    pub pso: ComputePipelineState,
    pub dispatch: DispatchPayload,
    pub uav_resource_address: u64,
}

/// Graphics rasterization workload to execute concurrently with compute.
#[derive(Debug, Clone)]
pub struct GraphicsRasterWorkload {
    pub name: &'static str,
    pub draw_call_count: u32,
    pub estimated_raster_cycles: u64,
    pub input_resource_address: Option<u64>,
}

/// Performance telemetry from concurrent execution scheduling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConcurrentExecutionMetrics {
    pub total_compute_workgroups: u64,
    pub compute_cycles: u64,
    pub graphics_cycles: u64,
    pub concurrent_overlapped_cycles: u64,
    pub overlap_efficiency_pct: u32,
    pub uav_barriers_flushed: u32,
    pub transitions_executed: u32,
    pub completion_fence_value: u64,
}

/// High-performance concurrent compute pipeline scheduler.
pub struct ConcurrentComputeScheduler {
    pub num_compute_units: u32,
    pub cache_state: GpuCacheState,
}

impl ConcurrentComputeScheduler {
    pub const fn new(num_compute_units: u32) -> Self {
        Self {
            num_compute_units,
            cache_state: GpuCacheState {
                l1_dirty_lines: 0,
                l2_dirty_lines: 0,
                l1_flushes: 0,
                l2_flushes: 0,
                read_invalidations: 0,
            },
        }
    }

    /// Schedules and executes compute workloads concurrently alongside graphics rasterization.
    pub fn execute_concurrent_pass(
        &mut self,
        compute_jobs: &[ComputeShaderWorkload],
        graphics_job: &GraphicsRasterWorkload,
        sync_hub: &mut TimelineSyncHub,
        fence: &GpuTimelineFence,
        fence_target_val: u64,
    ) -> Result<ConcurrentExecutionMetrics, &'static str> {
        let mut total_workgroups: u64 = 0;
        let mut uav_flushes: u32 = 0;
        let mut transitions: u32 = 0;

        // 1. Process and record all compute workloads across CUs
        for job in compute_jobs {
            let groups = job.dispatch.total_threadgroups();
            total_workgroups += groups;

            // Execute UAV memory barrier between compute dispatches
            self.cache_state.flush_uav_caches();
            uav_flushes += 1;
        }

        // 2. Perform transition barrier from Compute UAV write to Graphics Shader Resource read
        if let Some(res_addr) = graphics_job.input_resource_address {
            let barrier = ComputeMemoryBarrier::transition(
                100,
                res_addr,
                ResourceState::UNORDERED_ACCESS,
                ResourceState::PIXEL_SHADER_RESOURCE,
                PipelineStageFlags::COMPUTE_SHADER,
                PipelineStageFlags::PIXEL_SHADER,
            );
            if !barrier.is_valid_transition() {
                return Err("Invalid resource transition barrier");
            }
            self.cache_state.invalidate_reads();
            transitions += 1;
        }

        // 3. Compute cycles based on compute units:
        // Each workgroup takes ~100 GPU core cycles; distributed across CUs
        let cu_count = self.num_compute_units.max(1) as u64;
        let compute_cycles = (total_workgroups * 100) / cu_count;
        let graphics_cycles = graphics_job.estimated_raster_cycles;

        // Concurrent overlap calculation: both run simultaneously in hardware
        let overlapped_cycles = compute_cycles.min(graphics_cycles);
        let min_cycles = compute_cycles.min(graphics_cycles).max(1);
        let overlap_efficiency = ((overlapped_cycles * 100) / min_cycles) as u32;

        // 4. Signal timeline fence monotonically
        fence.signal(fence_target_val)?;
        fence.complete(fence_target_val);

        // 5. Register and resolve cross-queue sync to present
        sync_hub.synchronize_compute_to_graphics_present(fence.fence_id, fence_target_val)?;
        sync_hub.resolve_dependencies();

        Ok(ConcurrentExecutionMetrics {
            total_compute_workgroups: total_workgroups,
            compute_cycles,
            graphics_cycles,
            concurrent_overlapped_cycles: overlapped_cycles,
            overlap_efficiency_pct: overlap_efficiency,
            uav_barriers_flushed: uav_flushes,
            transitions_executed: transitions,
            completion_fence_value: fence_target_val,
        })
    }
}

// -----------------------------------------------------------------------------
// Asynchronous Compute Queue
// -----------------------------------------------------------------------------

/// Asynchronous hardware compute queue for submitting compute dispatches.
#[derive(Debug)]
pub struct AsyncComputeQueue {
    pub queue_id: u32,
    pub pending_buffers: Vec<AsyncComputeCommandBuffer>,
    pub completed_dispatches: u64,
    pub completed_workgroups: u64,
}

impl AsyncComputeQueue {
    pub fn new(queue_id: u32) -> Self {
        Self {
            queue_id,
            pending_buffers: Vec::new(),
            completed_dispatches: 0,
            completed_workgroups: 0,
        }
    }

    /// Submits a recorded async compute command buffer to the queue.
    pub fn submit(&mut self, cmd_buf: AsyncComputeCommandBuffer) {
        self.pending_buffers.push(cmd_buf);
    }

    /// Processes submitted compute command buffers, advancing workgroup counters.
    pub fn process_pending(&mut self) -> usize {
        let count = self.pending_buffers.len();
        for buf in self.pending_buffers.drain(..) {
            for cmd in &buf.commands {
                if let ComputeCommand::Dispatch(payload) = cmd {
                    self.completed_dispatches += 1;
                    self.completed_workgroups += payload.total_threadgroups();
                }
            }
        }
        count
    }

    #[inline(always)]
    pub fn pending_count(&self) -> usize {
        self.pending_buffers.len()
    }
}
