//! Direct3D / Vulkan-Style GPU Command Buffer Recording Infrastructure.
//! Provides high-throughput, low-latency command stream encoding for GPU hardware queues:

extern crate alloc;
use alloc::vec::Vec;
use core::mem::size_of;

// Command Opcodes & 64-bit Header

/// GPU command opcodes mimicking modern Direct3D 12 and Vulkan command streams.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuCommandOpcode {
    Nop = 0,
    DrawIndexed = 1,
    SetPipelineState = 2,
    SetRenderTargets = 3,
    CopyBufferRegion = 4,
    SetViewports = 5,
    SetScissorRects = 6,
    SetIndexBuffer = 7,
    SetVertexBuffer = 8,
    ResourceBarrier = 9,
    BindDescriptorHeaps = 10,
    SetDescriptorTable = 11,
}

impl GpuCommandOpcode {
    pub const fn from_u16(val: u16) -> Option<Self> {
        match val {
            0 => Some(Self::Nop),
            1 => Some(Self::DrawIndexed),
            2 => Some(Self::SetPipelineState),
            3 => Some(Self::SetRenderTargets),
            4 => Some(Self::CopyBufferRegion),
            5 => Some(Self::SetViewports),
            6 => Some(Self::SetScissorRects),
            7 => Some(Self::SetIndexBuffer),
            8 => Some(Self::SetVertexBuffer),
            9 => Some(Self::ResourceBarrier),
            10 => Some(Self::BindDescriptorHeaps),
            11 => Some(Self::SetDescriptorTable),
            _ => None,
        }
    }
}

/// Compact 64-bit command packet header formatted for direct GPU DMA fetch.
/// Bit Layout:
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandPacketHeader(pub u64);

impl CommandPacketHeader {
    pub const fn new(opcode: GpuCommandOpcode, payload_size: u16, flags: u16, stream_id: u16) -> Self {
        let raw = (opcode as u64 & 0xFFFF)
            | ((payload_size as u64 & 0xFFFF) << 16)
            | ((flags as u64 & 0xFFFF) << 32)
            | ((stream_id as u64 & 0xFFFF) << 48);
        Self(raw)
    }

    #[inline(always)]
    pub const fn opcode_raw(&self) -> u16 {
        (self.0 & 0xFFFF) as u16
    }

    #[inline(always)]
    pub fn opcode(&self) -> Option<GpuCommandOpcode> {
        GpuCommandOpcode::from_u16(self.opcode_raw())
    }

    #[inline(always)]
    pub const fn payload_size(&self) -> u16 {
        ((self.0 >> 16) & 0xFFFF) as u16
    }

    #[inline(always)]
    pub const fn flags(&self) -> u16 {
        ((self.0 >> 32) & 0xFFFF) as u16
    }

    #[inline(always)]
    pub const fn stream_id(&self) -> u16 {
        ((self.0 >> 48) & 0xFFFF) as u16
    }

    #[inline(always)]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }
}

// Resource Barriers & Pipeline Stages

/// GPU resource usage states for synchronization barriers (bitflags).
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceState(pub u32);

impl ResourceState {
    pub const COMMON: Self = Self(0);
    pub const VERTEX_AND_CONSTANT_BUFFER: Self = Self(1 << 0);
    pub const INDEX_BUFFER: Self = Self(1 << 1);
    pub const RENDER_TARGET: Self = Self(1 << 2);
    pub const UNORDERED_ACCESS: Self = Self(1 << 3);
    pub const DEPTH_WRITE: Self = Self(1 << 4);
    pub const DEPTH_READ: Self = Self(1 << 5);
    pub const NON_PIXEL_SHADER_RESOURCE: Self = Self(1 << 6);
    pub const PIXEL_SHADER_RESOURCE: Self = Self(1 << 7);
    pub const COPY_DEST: Self = Self(1 << 8);
    pub const COPY_SOURCE: Self = Self(1 << 9);
    pub const PRESENT: Self = Self(1 << 10);

    #[inline(always)]
    pub const fn bits(&self) -> u32 {
        self.0
    }

    #[inline(always)]
    pub const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

/// Pipeline stage synchronization masks (bitflags).
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineStageFlags(pub u32);

impl PipelineStageFlags {
    pub const TOP_OF_PIPE: Self = Self(1 << 0);
    pub const DRAW_INDIRECT: Self = Self(1 << 1);
    pub const VERTEX_INPUT: Self = Self(1 << 2);
    pub const VERTEX_SHADER: Self = Self(1 << 3);
    pub const PIXEL_SHADER: Self = Self(1 << 4);
    pub const DEPTH_STENCIL: Self = Self(1 << 5);
    pub const RENDER_TARGET: Self = Self(1 << 6);
    pub const COMPUTE_SHADER: Self = Self(1 << 7);
    pub const TRANSFER: Self = Self(1 << 8);
    pub const BOTTOM_OF_PIPE: Self = Self(1 << 9);
    pub const ALL_COMMANDS: Self = Self(0x3FF);

    #[inline(always)]
    pub const fn bits(&self) -> u32 {
        self.0
    }
}

/// Type of resource synchronization barrier.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarrierType {
    /// State transition (e.g. CopyDest -> PixelShaderResource).
    Transition = 0,
    /// Resource aliasing transition within placed resource heaps.
    Aliasing = 1,
    /// UAV memory dependency barrier (flushes compute L1/L2 caches).
    Uav = 2,
}

/// Payload for resource pipeline barriers (`ResourceBarrier`).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceBarrierPayload {
    pub barrier_type: u32,
    pub flags: u32,
    pub src_stages: u32,
    pub dst_stages: u32,
    pub state_before: u32,
    pub state_after: u32,
    pub resource_gpu_address: u64,
    pub subresource: u32,
    pub _reserved: u32,
}

impl ResourceBarrierPayload {
    /// Constructs a state transition barrier between two pipeline states.
    pub const fn transition(
        resource_gpu_address: u64,
        subresource: u32,
        state_before: ResourceState,
        state_after: ResourceState,
        src_stages: PipelineStageFlags,
        dst_stages: PipelineStageFlags,
    ) -> Self {
        Self {
            barrier_type: BarrierType::Transition as u32,
            flags: 0,
            src_stages: src_stages.bits(),
            dst_stages: dst_stages.bits(),
            state_before: state_before.bits(),
            state_after: state_after.bits(),
            resource_gpu_address,
            subresource,
            _reserved: 0,
        }
    }

    /// Constructs an Unordered Access View (UAV) memory barrier to flush caches between compute dispatches.
    pub const fn uav(resource_gpu_address: u64) -> Self {
        Self {
            barrier_type: BarrierType::Uav as u32,
            flags: 0,
            src_stages: PipelineStageFlags::COMPUTE_SHADER.bits(),
            dst_stages: PipelineStageFlags::COMPUTE_SHADER.bits(),
            state_before: ResourceState::UNORDERED_ACCESS.bits(),
            state_after: ResourceState::UNORDERED_ACCESS.bits(),
            resource_gpu_address,
            subresource: 0,
            _reserved: 0,
        }
    }
}

// Descriptor Heaps & Tables

/// Modern GPU descriptor heap types.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptorHeapType {
    /// Shader Resource View (SRV), Constant Buffer View (CBV), Unordered Access View (UAV).
    CbvSrvUav = 0,
    /// Texture samplers (linear, anisotropic, point).
    Sampler = 1,
    /// Render Target Views (RTV).
    Rtv = 2,
    /// Depth Stencil Views (DSV).
    Dsv = 3,
}

/// Payload for binding active descriptor heaps to the GPU shader unit (`BindDescriptorHeaps`).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindDescriptorHeapsPayload {
    pub num_heaps: u32,
    pub _reserved: u32,
    pub heap_gpu_addresses: [u64; 4],
    pub heap_types: [u32; 4],
    pub descriptor_counts: [u32; 4],
}

impl BindDescriptorHeapsPayload {
    pub const fn new() -> Self {
        Self {
            num_heaps: 0,
            _reserved: 0,
            heap_gpu_addresses: [0; 4],
            heap_types: [0; 4],
            descriptor_counts: [0; 4],
        }
    }

    pub fn add_heap(&mut self, heap_type: DescriptorHeapType, gpu_address: u64, count: u32) -> Result<(), &'static str> {
        if self.num_heaps >= 4 {
            return Err("Maximum of 4 descriptor heaps can be bound simultaneously");
        }
        let idx = self.num_heaps as usize;
        self.heap_gpu_addresses[idx] = gpu_address;
        self.heap_types[idx] = heap_type as u32;
        self.descriptor_counts[idx] = count;
        self.num_heaps += 1;
        Ok(())
    }
}

/// Payload for binding a descriptor table into a root signature parameter slot (`SetDescriptorTable`).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetDescriptorTablePayload {
    pub root_parameter_index: u32,
    pub heap_type: u32,
    pub base_descriptor_gpu_address: u64,
    pub descriptor_count: u32,
    pub _reserved: u32,
}

impl SetDescriptorTablePayload {
    pub const fn new(
        root_parameter_index: u32,
        heap_type: DescriptorHeapType,
        base_addr: u64,
        count: u32,
    ) -> Self {
        Self {
            root_parameter_index,
            heap_type: heap_type as u32,
            base_descriptor_gpu_address: base_addr,
            descriptor_count: count,
            _reserved: 0,
        }
    }
}

// Command Payloads

/// Payload for indexed primitive drawing (`DrawIndexed`).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrawIndexedPayload {
    pub index_count: u32,
    pub instance_count: u32,
    pub first_index: u32,
    pub vertex_offset: i32,
    pub first_instance: u32,
    pub _reserved: u32,
}

impl DrawIndexedPayload {
    pub const fn new(
        index_count: u32,
        instance_count: u32,
        first_index: u32,
        vertex_offset: i32,
        first_instance: u32,
    ) -> Self {
        Self {
            index_count,
            instance_count,
            first_index,
            vertex_offset,
            first_instance,
            _reserved: 0,
        }
    }
}

/// Primitive topology types for graphics pipelines.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimitiveTopology {
    PointList = 0,
    LineList = 1,
    LineStrip = 2,
    TriangleList = 3,
    TriangleStrip = 4,
}

/// Payload for setting pipeline state object (`SetPipelineState`).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetPipelineStatePayload {
    pub pipeline_id: u64,
    pub topology: u32,
    pub blend_enable: u8,
    pub depth_test_enable: u8,
    pub depth_write_enable: u8,
    pub cull_mode: u8, // 0 = None, 1 = Front, 2 = Back
}

impl SetPipelineStatePayload {
    pub const fn new(
        pipeline_id: u64,
        topology: PrimitiveTopology,
        blend_enable: bool,
        depth_test: bool,
        depth_write: bool,
        cull_mode: u8,
    ) -> Self {
        Self {
            pipeline_id,
            topology: topology as u32,
            blend_enable: blend_enable as u8,
            depth_test_enable: depth_test as u8,
            depth_write_enable: depth_write as u8,
            cull_mode,
        }
    }
}

/// Payload for binding render targets and depth-stencil buffer (`SetRenderTargets`).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetRenderTargetsPayload {
    pub num_render_targets: u32,
    pub dsv_format: u32,
    pub rtv_formats: [u32; 8],
    pub rtv_gpu_addresses: [u64; 8],
    pub dsv_gpu_address: u64,
}

impl SetRenderTargetsPayload {
    pub const fn single_target(rtv_format: u32, rtv_addr: u64, dsv_format: u32, dsv_addr: u64) -> Self {
        Self {
            num_render_targets: 1,
            dsv_format,
            rtv_formats: [rtv_format, 0, 0, 0, 0, 0, 0, 0],
            rtv_gpu_addresses: [rtv_addr, 0, 0, 0, 0, 0, 0, 0],
            dsv_gpu_address: dsv_addr,
        }
    }
}

/// Payload for direct VRAM/DMA buffer copying (`CopyBufferRegion`).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CopyBufferRegionPayload {
    pub dst_gpu_address: u64,
    pub src_gpu_address: u64,
    pub num_bytes: u64,
    pub dst_offset: u64,
    pub src_offset: u64,
}

impl CopyBufferRegionPayload {
    pub const fn new(dst: u64, src: u64, bytes: u64, dst_off: u64, src_off: u64) -> Self {
        Self {
            dst_gpu_address: dst,
            src_gpu_address: src,
            num_bytes: bytes,
            dst_offset: dst_off,
            src_offset: src_off,
        }
    }
}

/// Viewport definition (coordinates in pixels).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub min_depth: f32,
    pub max_depth: f32,
}

impl Viewport {
    pub const fn new(x: f32, y: f32, width: f32, height: f32, min_depth: f32, max_depth: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
            min_depth,
            max_depth,
        }
    }
}

/// Scissor rectangle definition (coordinates in pixels).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScissorRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl ScissorRect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }
}

// Linear Command Buffer Recorder & Stream

/// An immutable, finalized command buffer ready for hardware queue submission.
#[derive(Debug, Clone)]
pub struct GpuCommandBuffer {
    buffer: Vec<u8>,
    packet_count: usize,
    stream_id: u16,
}

impl GpuCommandBuffer {
    #[inline(always)]
    pub fn raw_bytes(&self) -> &[u8] {
        &self.buffer
    }

    #[inline(always)]
    pub fn byte_size(&self) -> usize {
        self.buffer.len()
    }

    #[inline(always)]
    pub fn packet_count(&self) -> usize {
        self.packet_count
    }

    #[inline(always)]
    pub fn stream_id(&self) -> u16 {
        self.stream_id
    }
}

/// High-performance linear command buffer recorder.
/// Encodes typed command packets into contiguous aligned memory with minimal overhead.
#[derive(Debug)]
pub struct CommandBufferRecorder {
    buffer: Vec<u8>,
    packet_count: usize,
    stream_id: u16,
    is_closed: bool,
}

impl CommandBufferRecorder {
    /// Creates a new command buffer recorder with pre-allocated memory capacity.
    pub fn new(capacity_bytes: usize, stream_id: u16) -> Self {
        Self {
            buffer: Vec::with_capacity(capacity_bytes),
            packet_count: 0,
            stream_id,
            is_closed: false,
        }
    }

    /// Resets the recorder for reuse, clearing existing commands while preserving capacity.
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.packet_count = 0;
        self.is_closed = false;
    }

    /// Appends an 8-byte aligned command packet with a 64-bit header and raw payload slice.
    fn append_packet(&mut self, opcode: GpuCommandOpcode, payload: &[u8], flags: u16) -> Result<(), &'static str> {
        if self.is_closed {
            return Err("Cannot append commands to closed recorder");
        }

        // Ensure 8-byte alignment for the header
        let remainder = self.buffer.len() % 8;
        if remainder != 0 {
            let pad = 8 - remainder;
            self.buffer.resize(self.buffer.len() + pad, 0);
        }

        let payload_len = payload.len();
        if payload_len > 0xFFFF {
            return Err("Payload exceeds 64KB packet limit");
        }

        let header = CommandPacketHeader::new(opcode, payload_len as u16, flags, self.stream_id);
        self.buffer.extend_from_slice(&header.as_u64().to_ne_bytes());
        self.buffer.extend_from_slice(payload);

        // Pad payload to 8-byte boundary
        let pay_rem = payload_len % 8;
        if pay_rem != 0 {
            let pad = 8 - pay_rem;
            self.buffer.resize(self.buffer.len() + pad, 0);
        }

        self.packet_count += 1;
        Ok(())
    }

    /// Encodes a `SetPipelineState` command into the stream.
    pub fn record_set_pipeline_state(&mut self, payload: &SetPipelineStatePayload) -> Result<(), &'static str> {
        let bytes = unsafe {
            core::slice::from_raw_parts(
                payload as *const SetPipelineStatePayload as *const u8,
                size_of::<SetPipelineStatePayload>(),
            )
        };
        self.append_packet(GpuCommandOpcode::SetPipelineState, bytes, 0)
    }

    /// Encodes a `SetRenderTargets` command into the stream.
    pub fn record_set_render_targets(&mut self, payload: &SetRenderTargetsPayload) -> Result<(), &'static str> {
        let bytes = unsafe {
            core::slice::from_raw_parts(
                payload as *const SetRenderTargetsPayload as *const u8,
                size_of::<SetRenderTargetsPayload>(),
            )
        };
        self.append_packet(GpuCommandOpcode::SetRenderTargets, bytes, 0)
    }

    /// Encodes a `DrawIndexed` command into the stream.
    pub fn record_draw_indexed(&mut self, payload: &DrawIndexedPayload) -> Result<(), &'static str> {
        let bytes = unsafe {
            core::slice::from_raw_parts(
                payload as *const DrawIndexedPayload as *const u8,
                size_of::<DrawIndexedPayload>(),
            )
        };
        self.append_packet(GpuCommandOpcode::DrawIndexed, bytes, 0)
    }

    /// Encodes a `CopyBufferRegion` command into the stream.
    pub fn record_copy_buffer_region(&mut self, payload: &CopyBufferRegionPayload) -> Result<(), &'static str> {
        let bytes = unsafe {
            core::slice::from_raw_parts(
                payload as *const CopyBufferRegionPayload as *const u8,
                size_of::<CopyBufferRegionPayload>(),
            )
        };
        self.append_packet(GpuCommandOpcode::CopyBufferRegion, bytes, 0)
    }

    /// Encodes a `SetViewports` command into the stream.
    pub fn record_viewport(&mut self, vp: Viewport) -> Result<(), &'static str> {
        let bytes = unsafe {
            core::slice::from_raw_parts(&vp as *const Viewport as *const u8, size_of::<Viewport>())
        };
        self.append_packet(GpuCommandOpcode::SetViewports, bytes, 0)
    }

    /// Encodes a `SetScissorRects` command into the stream.
    pub fn record_scissor(&mut self, sc: ScissorRect) -> Result<(), &'static str> {
        let bytes = unsafe {
            core::slice::from_raw_parts(&sc as *const ScissorRect as *const u8, size_of::<ScissorRect>())
        };
        self.append_packet(GpuCommandOpcode::SetScissorRects, bytes, 0)
    }

    /// Encodes a generic resource pipeline barrier command.
    pub fn record_resource_barrier(&mut self, barrier: &ResourceBarrierPayload) -> Result<(), &'static str> {
        let bytes = unsafe {
            core::slice::from_raw_parts(
                barrier as *const ResourceBarrierPayload as *const u8,
                size_of::<ResourceBarrierPayload>(),
            )
        };
        self.append_packet(GpuCommandOpcode::ResourceBarrier, bytes, 0)
    }

    /// Encodes a resource state transition barrier (e.g. Common -> RenderTarget, CopyDest -> ShaderResource).
    pub fn record_transition_barrier(
        &mut self,
        resource_gpu_address: u64,
        subresource: u32,
        state_before: ResourceState,
        state_after: ResourceState,
        src_stages: PipelineStageFlags,
        dst_stages: PipelineStageFlags,
    ) -> Result<(), &'static str> {
        let barrier = ResourceBarrierPayload::transition(
            resource_gpu_address,
            subresource,
            state_before,
            state_after,
            src_stages,
            dst_stages,
        );
        self.record_resource_barrier(&barrier)
    }

    /// Encodes an Unordered Access View (UAV) execution barrier between compute dispatches.
    pub fn record_uav_barrier(&mut self, resource_gpu_address: u64) -> Result<(), &'static str> {
        let barrier = ResourceBarrierPayload::uav(resource_gpu_address);
        self.record_resource_barrier(&barrier)
    }

    /// Encodes a `BindDescriptorHeaps` command to activate GPU descriptor pools.
    pub fn record_bind_descriptor_heaps(&mut self, payload: &BindDescriptorHeapsPayload) -> Result<(), &'static str> {
        if payload.num_heaps > 4 {
            return Err("Descriptor heap count cannot exceed 4");
        }
        let bytes = unsafe {
            core::slice::from_raw_parts(
                payload as *const BindDescriptorHeapsPayload as *const u8,
                size_of::<BindDescriptorHeapsPayload>(),
            )
        };
        self.append_packet(GpuCommandOpcode::BindDescriptorHeaps, bytes, 0)
    }

    /// Encodes a `SetDescriptorTable` command binding a table into a root signature slot.
    pub fn record_set_descriptor_table(&mut self, payload: &SetDescriptorTablePayload) -> Result<(), &'static str> {
        let bytes = unsafe {
            core::slice::from_raw_parts(
                payload as *const SetDescriptorTablePayload as *const u8,
                size_of::<SetDescriptorTablePayload>(),
            )
        };
        self.append_packet(GpuCommandOpcode::SetDescriptorTable, bytes, 0)
    }

    /// Finalizes and closes the command buffer for hardware submission.
    pub fn close(&mut self) -> Result<GpuCommandBuffer, &'static str> {
        if self.is_closed {
            return Err("Recorder already closed");
        }
        self.is_closed = true;

        Ok(GpuCommandBuffer {
            buffer: self.buffer.clone(),
            packet_count: self.packet_count,
            stream_id: self.stream_id,
        })
    }
}

// Command Stream Decoder & Packet Iterator

/// View of an individual decoded command packet.
#[derive(Debug, Clone, Copy)]
pub struct CommandPacketView<'a> {
    pub header: CommandPacketHeader,
    pub payload: &'a [u8],
}

/// Zero-copy command stream iterator over recorded command buffer bytes.
pub struct CommandStreamReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> CommandStreamReader<'a> {
    pub fn new(cmdbuf: &'a GpuCommandBuffer) -> Self {
        Self {
            bytes: cmdbuf.raw_bytes(),
            offset: 0,
        }
    }

    pub fn from_raw_slice(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
}

impl<'a> Iterator for CommandStreamReader<'a> {
    type Item = CommandPacketView<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        // Align to 8 bytes
        let remainder = self.offset % 8;
        if remainder != 0 {
            self.offset += 8 - remainder;
        }

        if self.offset + 8 > self.bytes.len() {
            return None;
        }

        let header_raw = u64::from_ne_bytes([
            self.bytes[self.offset],
            self.bytes[self.offset + 1],
            self.bytes[self.offset + 2],
            self.bytes[self.offset + 3],
            self.bytes[self.offset + 4],
            self.bytes[self.offset + 5],
            self.bytes[self.offset + 6],
            self.bytes[self.offset + 7],
        ]);
        let header = CommandPacketHeader(header_raw);
        self.offset += 8;

        let payload_len = header.payload_size() as usize;
        if self.offset + payload_len > self.bytes.len() {
            return None;
        }

        let payload = &self.bytes[self.offset..self.offset + payload_len];
        self.offset += payload_len;

        // Skip payload alignment padding
        let pay_rem = payload_len % 8;
        if pay_rem != 0 {
            self.offset += 8 - pay_rem;
        }

        Some(CommandPacketView { header, payload })
    }
}

// Unit Tests & Verification

/// Verifies command packet encoding, 64-bit headers, linear recording, and stream decoding.
pub fn verify_command_packet_encoding_and_recorder() -> bool {
    let mut recorder = CommandBufferRecorder::new(2048, 1);

    // 1. Record SetPipelineState
    let pso = SetPipelineStatePayload::new(
        0xDEAD_BEEF_CAFE_BABE,
        PrimitiveTopology::TriangleList,
        true,
        true,
        true,
        2,
    );
    if recorder.record_set_pipeline_state(&pso).is_err() {
        return false;
    }

    // 2. Record SetRenderTargets
    let rts = SetRenderTargetsPayload::single_target(
        0x18, // R8G8B8A8_UNORM
        0xFFFF_8000_1000_0000,
        0x2D, // D32_FLOAT
        0xFFFF_8000_2000_0000,
    );
    if recorder.record_set_render_targets(&rts).is_err() {
        return false;
    }

    // 3. Record Viewport
    let vp = Viewport::new(0.0, 0.0, 1920.0, 1080.0, 0.0, 1.0);
    if recorder.record_viewport(vp).is_err() {
        return false;
    }

    // 4. Record DrawIndexed
    let draw = DrawIndexedPayload::new(36, 1, 0, 0, 0);
    if recorder.record_draw_indexed(&draw).is_err() {
        return false;
    }

    // 5. Record CopyBufferRegion
    let copy = CopyBufferRegionPayload::new(
        0xFFFF_8000_3000_0000,
        0xFFFF_8000_4000_0000,
        4096,
        0,
        0,
    );
    if recorder.record_copy_buffer_region(&copy).is_err() {
        return false;
    }

    // 6. Close command buffer
    let cmdbuf = match recorder.close() {
        Ok(buf) => buf,
        Err(_) => return false,
    };

    if cmdbuf.packet_count() != 5 {
        return false;
    }
    if cmdbuf.stream_id() != 1 {
        return false;
    }

    // 7. Decode command stream via reader
    let reader = CommandStreamReader::new(&cmdbuf);
    let mut decoded_count = 0;

    for packet in reader {
        match packet.header.opcode() {
            Some(GpuCommandOpcode::SetPipelineState) => {
                if packet.payload.len() != size_of::<SetPipelineStatePayload>() {
                    return false;
                }
                let decoded: &SetPipelineStatePayload =
                    unsafe { &*(packet.payload.as_ptr() as *const SetPipelineStatePayload) };
                if decoded.pipeline_id != 0xDEAD_BEEF_CAFE_BABE
                    || decoded.topology != PrimitiveTopology::TriangleList as u32
                    || decoded.cull_mode != 2
                {
                    return false;
                }
            }
            Some(GpuCommandOpcode::SetRenderTargets) => {
                if packet.payload.len() != size_of::<SetRenderTargetsPayload>() {
                    return false;
                }
                let decoded: &SetRenderTargetsPayload =
                    unsafe { &*(packet.payload.as_ptr() as *const SetRenderTargetsPayload) };
                if decoded.num_render_targets != 1
                    || decoded.rtv_formats[0] != 0x18
                    || decoded.rtv_gpu_addresses[0] != 0xFFFF_8000_1000_0000
                {
                    return false;
                }
            }
            Some(GpuCommandOpcode::SetViewports) => {
                if packet.payload.len() != size_of::<Viewport>() {
                    return false;
                }
                let decoded: &Viewport =
                    unsafe { &*(packet.payload.as_ptr() as *const Viewport) };
                if decoded.width != 1920.0 || decoded.height != 1080.0 {
                    return false;
                }
            }
            Some(GpuCommandOpcode::DrawIndexed) => {
                if packet.payload.len() != size_of::<DrawIndexedPayload>() {
                    return false;
                }
                let decoded: &DrawIndexedPayload =
                    unsafe { &*(packet.payload.as_ptr() as *const DrawIndexedPayload) };
                if decoded.index_count != 36 || decoded.instance_count != 1 {
                    return false;
                }
            }
            Some(GpuCommandOpcode::CopyBufferRegion) => {
                if packet.payload.len() != size_of::<CopyBufferRegionPayload>() {
                    return false;
                }
                let decoded: &CopyBufferRegionPayload =
                    unsafe { &*(packet.payload.as_ptr() as *const CopyBufferRegionPayload) };
                if decoded.num_bytes != 4096
                    || decoded.dst_gpu_address != 0xFFFF_8000_3000_0000
                    || decoded.src_gpu_address != 0xFFFF_8000_4000_0000
                {
                    return false;
                }
            }
            _ => return false,
        }
        decoded_count += 1;
    }

    if decoded_count != 5 {
        return false;
    }

    // 8. Test recorder reset
    recorder.reset();
    let draw2 = DrawIndexedPayload::new(6, 1, 0, 0, 0);
    if recorder.record_draw_indexed(&draw2).is_err() {
        return false;
    }
    let cmdbuf2 = match recorder.close() {
        Ok(buf) => buf,
        Err(_) => return false,
    };
    if cmdbuf2.packet_count() != 1 {
        return false;
    }

    true
}

/// Verifies pipeline barriers, resource state transitions, UAV barriers, and descriptor heap bindings.
pub fn verify_barriers_and_descriptor_bindings() -> bool {
    let mut recorder = CommandBufferRecorder::new(4096, 2);

    // 1. Transition Texture from Common to CopyDest
    let tex_addr = 0xFFFF_9000_0000_0000u64;
    if recorder
        .record_transition_barrier(
            tex_addr,
            0,
            ResourceState::COMMON,
            ResourceState::COPY_DEST,
            PipelineStageFlags::TOP_OF_PIPE,
            PipelineStageFlags::TRANSFER,
        )
        .is_err()
    {
        return false;
    }

    // 2. CopyBufferRegion upload
    let copy = CopyBufferRegionPayload::new(tex_addr, 0xFFFF_8000_1000_0000, 65536, 0, 0);
    if recorder.record_copy_buffer_region(&copy).is_err() {
        return false;
    }

    // 3. Transition Texture from CopyDest to PixelShaderResource
    if recorder
        .record_transition_barrier(
            tex_addr,
            0,
            ResourceState::COPY_DEST,
            ResourceState::PIXEL_SHADER_RESOURCE,
            PipelineStageFlags::TRANSFER,
            PipelineStageFlags::PIXEL_SHADER,
        )
        .is_err()
    {
        return false;
    }

    // 4. Bind Descriptor Heaps (CBV/SRV/UAV heap + Sampler heap)
    let mut heaps = BindDescriptorHeapsPayload::new();
    if heaps.add_heap(DescriptorHeapType::CbvSrvUav, 0xFFFF_A000_0000_0000, 1024).is_err() {
        return false;
    }
    if heaps.add_heap(DescriptorHeapType::Sampler, 0xFFFF_B000_0000_0000, 64).is_err() {
        return false;
    }
    if recorder.record_bind_descriptor_heaps(&heaps).is_err() {
        return false;
    }

    // 5. Set Descriptor Tables for Root Parameters
    let cbv_table = SetDescriptorTablePayload::new(0, DescriptorHeapType::CbvSrvUav, 0xFFFF_A000_0000_0000, 16);
    if recorder.record_set_descriptor_table(&cbv_table).is_err() {
        return false;
    }
    let srv_table = SetDescriptorTablePayload::new(1, DescriptorHeapType::CbvSrvUav, 0xFFFF_A000_0000_0800, 32);
    if recorder.record_set_descriptor_table(&srv_table).is_err() {
        return false;
    }

    // 6. Transition RenderTarget from Present to RenderTarget
    let rtv_addr = 0xFFFF_C000_0000_0000u64;
    if recorder
        .record_transition_barrier(
            rtv_addr,
            0,
            ResourceState::PRESENT,
            ResourceState::RENDER_TARGET,
            PipelineStageFlags::TOP_OF_PIPE,
            PipelineStageFlags::RENDER_TARGET,
        )
        .is_err()
    {
        return false;
    }

    // 7. UAV execution barrier for compute dispatch
    let compute_buf = 0xFFFF_D000_0000_0000u64;
    if recorder.record_uav_barrier(compute_buf).is_err() {
        return false;
    }

    // 8. Close and decode
    let cmdbuf = match recorder.close() {
        Ok(b) => b,
        Err(_) => return false,
    };

    if cmdbuf.packet_count() != 8 {
        return false;
    }

    let reader = CommandStreamReader::new(&cmdbuf);
    let mut barrier_count = 0;
    let mut heap_count = 0;
    let mut table_count = 0;

    for packet in reader {
        match packet.header.opcode() {
            Some(GpuCommandOpcode::ResourceBarrier) => {
                let b: &ResourceBarrierPayload =
                    unsafe { &*(packet.payload.as_ptr() as *const ResourceBarrierPayload) };
                if b.barrier_type == BarrierType::Transition as u32 {
                    if b.resource_gpu_address == tex_addr && b.state_before == ResourceState::COMMON.bits() {
                        if b.state_after != ResourceState::COPY_DEST.bits() {
                            return false;
                        }
                    } else if b.resource_gpu_address == tex_addr && b.state_before == ResourceState::COPY_DEST.bits() {
                        if b.state_after != ResourceState::PIXEL_SHADER_RESOURCE.bits() {
                            return false;
                        }
                    } else if b.resource_gpu_address == rtv_addr {
                        if b.state_before != ResourceState::PRESENT.bits()
                            || b.state_after != ResourceState::RENDER_TARGET.bits()
                        {
                            return false;
                        }
                    }
                } else if b.barrier_type == BarrierType::Uav as u32 {
                    if b.resource_gpu_address != compute_buf {
                        return false;
                    }
                }
                barrier_count += 1;
            }
            Some(GpuCommandOpcode::BindDescriptorHeaps) => {
                let h: &BindDescriptorHeapsPayload =
                    unsafe { &*(packet.payload.as_ptr() as *const BindDescriptorHeapsPayload) };
                if h.num_heaps != 2
                    || h.heap_types[0] != DescriptorHeapType::CbvSrvUav as u32
                    || h.heap_types[1] != DescriptorHeapType::Sampler as u32
                    || h.descriptor_counts[0] != 1024
                    || h.descriptor_counts[1] != 64
                {
                    return false;
                }
                heap_count += 1;
            }
            Some(GpuCommandOpcode::SetDescriptorTable) => {
                let t: &SetDescriptorTablePayload =
                    unsafe { &*(packet.payload.as_ptr() as *const SetDescriptorTablePayload) };
                if t.root_parameter_index == 0 {
                    if t.descriptor_count != 16 {
                        return false;
                    }
                } else if t.root_parameter_index == 1 {
                    if t.descriptor_count != 32 {
                        return false;
                    }
                }
                table_count += 1;
            }
            Some(GpuCommandOpcode::CopyBufferRegion) => {}
            _ => return false,
        }
    }

    if barrier_count != 4 || heap_count != 1 || table_count != 2 {
        return false;
    }

    true
}
