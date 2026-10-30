//! Asynchronous Compute Command Stream Encoder & Dispatch Queue Subsystem.
//!
//! Implements modern low-level asynchronous compute command encoding (Direct3D 12 / Vulkan model):
//! - Independent async compute command buffers bypassing graphics pipeline overhead
//! - 3D workgroup dispatch recording (`Dispatch(x, y, z)`)
//! - Compute Pipeline State Objects (PSO) with local threadgroup dimension tracking
//! - Resource descriptor tables (UAVs, SRVs, CBVs) and push constant data streams
//! - High-speed asynchronous compute submission queue

#![allow(dead_code)]

extern crate alloc;
use alloc::vec::Vec;

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

/// Descriptor table binding up to 16 resource descriptors to a shader root parameter.
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
    ResourceBarrier { barrier_id: u32, is_uav: bool, gpu_address: u64 },
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
